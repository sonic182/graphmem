# Reference algorithm pseudocode

Companion to [`graph-tune.md`](./graph-tune.md). This is pseudocode of
**intent** — what the papers say to do, not how it is (or necessarily will be)
implemented in Rust inside graphmem. The default parameters quoted (`damping`,
`λ`, `ε`, `N_seed`, `K_edge`, `β`) are those reported by the papers/official
repos.

---

## HippoRAG 2 (Phase 1)

### Indexing (offline)

```text
function INDEX(documents):
    graph = new Graph()

    for doc in documents:
        passage_node = graph.add_passage_node(doc)

        triples = OpenIE_LLM(doc)                     # (subject, relation, object)
        for (s, r, o) in triples:
            s_node = graph.get_or_add_phrase_node(s)
            o_node = graph.get_or_add_phrase_node(o)
            graph.add_edge(s_node, o_node, relation=r, type=RELATION)
            graph.add_edge(s_node, passage_node, type=CONTEXT)
            graph.add_edge(o_node, passage_node, type=CONTEXT)

        for (a, b) in near_duplicate_pairs(graph.phrase_nodes, threshold=τ_syn):
            graph.add_edge(a, b, type=SYNONYM)         # concept dedup / paraphrase bridge

    embed_all(graph.passages)      # embedding per passage
    embed_all(graph.triples)       # embedding of the full triple (s, r, o)
    embed_all(graph.phrase_nodes)  # embedding per entity
    return graph
```

### Retrieval (query time)

```text
function RETRIEVE(query, graph, top_k):
    q_vec = embed(query)

    # 1. query ↔ full triple similarity (not just entity name)
    triple_scores = { t: cos(q_vec, embed(t)) for t in graph.triples }
    candidate_triples = top_n(triple_scores, N_triple)

    # 2. Recognition Memory — an LLM filters the truly relevant triples
    relevant_triples = LLM_filter(query, candidate_triples)

    # 3. personalization vector (seeds), two channels
    seeds = {}
    for t in relevant_triples:
        for node in (t.subject, t.object):
            seeds[node] += triple_scores[t]            # "fact/entity" channel
    normalize(seeds)

    passage_scores = { p: cos(q_vec, embed(p)) for p in graph.passages }
    for p in top_n(passage_scores, N_passage):
        seeds[p] += λ * passage_scores[p]               # "direct passage" channel, λ ≈ 0.05
    normalize(seeds)

    # 4. Personalized PageRank over the STATIC graph (same weight every time)
    π = PPR(graph.adjacency, seeds, damping=d)           # d ≈ 0.5

    ranked = sort_desc(graph.passages, key=p -> π[p])
    return ranked[:top_k]


function PPR(adjacency, seed_dist, damping, iterations):
    π = seed_dist.copy()
    for _ in range(iterations):
        next_π = (1 - damping) * seed_dist
        for u in nodes:
            share = π[u] * damping / len(adjacency[u])   # ← UNIFORM split among neighbors
            for v in adjacency[u]:
                next_π[v] += share
        π = normalize(next_π)
    return π
```

The key point (and the one CatRAG attacks): `share` in `PPR` does not depend
on the query — only on how many neighbors `u` has. Two different queries with
the same seeds produce exactly the same propagation.

---

## CatRAG (Phase 2)

Same retrieval skeleton as HippoRAG 2 (steps 1-3 identical), plus three
mechanisms that make the edge weights **query-dependent** before running PPR.

```text
function RETRIEVE_CATRAG(query, graph, top_k):
    q_vec = embed(query)

    # --- steps 1-3: identical to HippoRAG 2 ---
    triple_scores = { t: cos(q_vec, embed(t)) for t in graph.triples }
    candidate_triples = top_n(triple_scores, N_triple)
    relevant_triples = LLM_filter(query, candidate_triples)
    seeds = build_seed_distribution(relevant_triples, triple_scores)   # see HippoRAG 2

    # --- mechanism 1: Symbolic Anchoring ---
    explicit_entities = NER(query)                       # graphmem: literal-mention matching
    for e in explicit_entities:
        if e not in seeds:
            seeds[e] += ε                                 # weak anchor, ε ≈ 0.2
    normalize(seeds)

    top_seed_nodes = top_n(seeds, N_seed)                 # N_seed ≈ 5

    # --- mechanism 2: Query-Aware Dynamic Edge Weighting ---
    edge_weight_overlay = {}
    for u in top_seed_nodes:
        candidates = graph.outgoing_edges(u)

        # cheap filter: coarse embedding filter
        scored = [(e, cos(q_vec, embed(e))) for e in candidates]
        top_candidates = top_n(scored, K_edge)            # K_edge ≈ 15

        # optional second pass depending on config: re-score the shortlist
        for (e, coarse_score) in top_candidates:
            relevance = edge_scorer.score(query, e, coarse_score)  # see EDGE SCORERS below
            multiplier = relevance_to_multiplier(relevance)
            edge_weight_overlay[(u, e.target)] =
                static_weight(u, e.target) * multiplier
            # ← only the expansion direction out of the seed is touched

    # --- mechanism 3: Key-Fact Passage Enhancement ---
    for t in relevant_triples:
        for (entity, passage) in passages_containing(t):   # provenance: fact → passage
            edge_weight_overlay[(entity, passage)] =
                weight(entity, passage) * (1 + β)            # β ≈ 2.5

    renormalize(edge_weight_overlay, graph)                 # outgoing mass must still sum to 1

    # --- weighted, directed PPR over the overlay ---
    π = WEIGHTED_PPR(graph.adjacency, seeds, edge_weight_overlay, damping=d)

    ranked = sort_desc(graph.passages, key=p -> π[p])
    return ranked[:top_k]


function WEIGHTED_PPR(adjacency, seed_dist, weight_overlay, damping, iterations):
    π = seed_dist.copy()
    for _ in range(iterations):
        next_π = (1 - damping) * seed_dist
        for u in nodes:
            out_edges = adjacency[u]
            weights = [ weight_overlay.get((u, v), default_weight(u, v))
                        for v in out_edges ]
            total = sum(weights)
            for v, w in zip(out_edges, weights):
                next_π[v] += π[u] * damping * (w / total)    # ← weighted, not uniform
        π = normalize(next_π)
    return π
```

**Note for graphmem (planned Phase 2):** `NER(query)` already exists as literal
`text_mentions`. `WEIGHTED_PPR` is exactly the
`edge_weight_overlay: Option<&HashMap<(usize, usize), f64>>` described in
`graph-tune.md`; when the overlay is empty, `WEIGHTED_PPR` collapses to Phase
1's uniform `PPR`. `edge_scorer` is the `EdgeScorer` trait from
`graph-tune.md`, with three possible implementations, all behind the same
interface — nothing in `RETRIEVE_CATRAG`/`WEIGHTED_PPR` changes when switching
from one to another:

```text
# the coarse cosine filter (inside RETRIEVE_CATRAG, above) ALWAYS runs first
# and already produces `coarse_score` for each shortlist candidate;
# edge_scorer.score() decides whether that is enough or whether to re-score
# with something more precise — chosen via config, not code:
#
#   [retrieval] edge_scorer = "cosine" | "cross_encoder"   # default "cosine"

function MAKE_EDGE_SCORER(config):
    match config.edge_scorer:
        "cosine":         return EmbeddingEdgeScorer()
        "cross_encoder":  return CrossEncoderEdgeScorer(load_cross_encoder_model())
        # "llm" not exposed yet — see Tier 3 below


# --- Tier 1: EmbeddingEdgeScorer (default, zero extra cost) ---
function EMBEDDING_SCORE(query, edge, coarse_score):
    return coarse_score                        # reuses the coarse filter score, recomputes nothing


# --- Tier 2: CrossEncoderEdgeScorer (optional local reranker, e.g. MiniLM) ---
function CROSS_ENCODER_SCORE(query, edge, coarse_score, cross_encoder_model):
    pair_text = (query, edge_document(edge))   # edge_document(e) = "source relation target"
    return cross_encoder_model.forward(pair_text)
    # BERT-family: [CLS] of the pair (query, edge_document) -> linear head -> 1 score
    # more precise than Tier 1 because query and edge attend to each other
    # (rather than comparing two independently computed embeddings);
    # only runs on the shortlist already filtered by cosine (~K_edge candidates
    # per query), still 100% local — no external LLM call


# --- Tier 3: LlmEdgeScorer (out of this roadmap for now) ---
function LLM_SCORE(query, edge, coarse_score, llm):
    label = llm.classify(query, edge.source, edge.relation, edge.target, context(edge.target))
    return label_to_score(label)               # {Irrelevant, Weak, High, Direct} -> score


function relevance_to_multiplier(relevance):
    return monotonic_map(relevance)             # e.g. clamp/sigmoid instead of bucketing by label
```

Tier 1 is already available today (`edge_vector` + cosine, used globally to
pick seeds; here the same number is just reused, at no extra cost). Tier 2 is
the optional quality improvement — same role as `LLM_classify` in the paper,
but with `cross-encoder/ms-marco-MiniLM-L-6-v2` or similar instead of an LLM,
loadable via `candle_transformers::models::bert` just like DistilBERT in
`embedding.rs`, and one forward pass per candidate (~`K_edge`≈15 per query —
cheap on CPU, but not free, hence the config toggle). Tier 3 is deliberately
kept out of the roadmap.

---

## GraphFlow (Phase 3)

Retrieval as a sequence of decisions trained with a GFlowNets idea (policy +
flow estimator), instead of "compute scores → PPR".

### Inference: sample trajectories

```text
function RETRIEVE_GRAPHFLOW(query, graph, num_trajectories):
    trajectories = []

    for _ in range(num_trajectories):
        state = start_state(query, graph)          # initial seed(s) from the query
        trajectory = [state]

        loop:
            actions = available_actions(state, graph)   # neighbors + special SELF_LOOP action
            probs = softmax(policy_scores(state, actions))   # r_θ(state, action)
            action = sample(actions, probs)

            if action == SELF_LOOP:
                break                                # "this is already a useful retrieval, stop"

            state = transition(state, action)
            trajectory.append(state)

        trajectories.append(trajectory)

    return aggregate_results(trajectories)            # dedupe + rank by frequency/flow
```

### Training: flow consistency (detailed balance)

```text
function TRAIN_STEP(query, ground_truth_trajectory, graph, policy, flow_net):
    loss = 0
    trajectory = ground_truth_trajectory               # e.g. A -> B -> C

    for t in range(len(trajectory) - 1):
        s_t, s_next = trajectory[t], trajectory[t + 1]

        # local exploration: besides C, sample alternative neighbors of B
        alt_actions = available_actions(s_t, graph)
        for s_alt in alt_actions:                       # includes s_next and exploration (D, E, F...)
            log_F_t    = flow_net(s_t)
            log_F_alt  = flow_net(s_alt)
            log_P_fwd  = policy_log_prob(s_t, s_alt)
            log_P_back = 0                               # backward policy fixed to 1 (no backtracking)

            # detailed balance: F(s_t) P(s_t+1|s_t) ≈ F(s_t+1) P_back(s_t|s_t+1)
            loss += (log_F_t + log_P_fwd - log_F_alt - log_P_back) ** 2

    reward = terminal_reward(trajectory, ground_truth_trajectory)  # was the final retrieval good?
    loss += flow_matching_terminal_loss(flow_net, trajectory[-1], reward)

    backprop(loss)     # updates policy MLP + flow MLP (+ LoRA adapters on the LLM backbone)
```

The reward is only observed at the end of the trajectory; `detailed balance`
is what lets it propagate back to intermediate decisions without manually
labeling each step, and sampling proportional to reward (rather than greedy)
is what gives diversity across trajectories at inference.
