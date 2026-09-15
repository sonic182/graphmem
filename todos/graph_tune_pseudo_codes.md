# Pseudocódigo de los algoritmos de referencia

Companion de [`graph_tune.md`](./graph_tune.md). Esto es pseudocódigo de
**intención** — qué dicen los papers que hay que hacer, no cómo está (ni
estará necesariamente) implementado en Rust dentro de graphmem. Los
parámetros por defecto citados (`damping`, `λ`, `ε`, `N_seed`, `K_edge`,
`β`) son los que reportan los papers/repos oficiales.

---

## HippoRAG 2 (Fase 1)

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

    embed_all(graph.passages)      # embedding por passage
    embed_all(graph.triples)       # embedding del triple completo (s, r, o)
    embed_all(graph.phrase_nodes)  # embedding por entidad
    return graph
```

### Retrieval (query time)

```text
function RETRIEVE(query, graph, top_k):
    q_vec = embed(query)

    # 1. similitud query ↔ triple completo (no solo nombre de entidad)
    triple_scores = { t: cos(q_vec, embed(t)) for t in graph.triples }
    candidate_triples = top_n(triple_scores, N_triple)

    # 2. Recognition Memory — un LLM filtra los triples realmente relevantes
    relevant_triples = LLM_filter(query, candidate_triples)

    # 3. vector de personalización (seeds), dos canales
    seeds = {}
    for t in relevant_triples:
        for node in (t.subject, t.object):
            seeds[node] += triple_scores[t]            # canal "fact/entity"
    normalize(seeds)

    passage_scores = { p: cos(q_vec, embed(p)) for p in graph.passages }
    for p in top_n(passage_scores, N_passage):
        seeds[p] += λ * passage_scores[p]               # canal "passage directo", λ ≈ 0.05
    normalize(seeds)

    # 4. Personalized PageRank sobre el grafo ESTÁTICO (mismo peso siempre)
    π = PPR(graph.adjacency, seeds, damping=d)           # d ≈ 0.5

    ranked = sort_desc(graph.passages, key=p -> π[p])
    return ranked[:top_k]


function PPR(adjacency, seed_dist, damping, iterations):
    π = seed_dist.copy()
    for _ in range(iterations):
        next_π = (1 - damping) * seed_dist
        for u in nodes:
            share = π[u] * damping / len(adjacency[u])   # ← reparto UNIFORME entre vecinos
            for v in adjacency[u]:
                next_π[v] += share
        π = normalize(next_π)
    return π
```

El punto clave (y el que CatRAG ataca): `share` en `PPR` no depende de la
query — solo de cuántos vecinos tiene `u`. Dos queries distintas con los
mismos seeds producen exactamente la misma propagación.

---

## CatRAG (Fase 2)

Mismo esqueleto de retrieval que HippoRAG 2 (pasos 1-3 idénticos), más tres
mecanismos que hacen los pesos de las edges **dependientes de la query**
antes de correr PPR.

```text
function RETRIEVE_CATRAG(query, graph, top_k):
    q_vec = embed(query)

    # --- pasos 1-3: idénticos a HippoRAG 2 ---
    triple_scores = { t: cos(q_vec, embed(t)) for t in graph.triples }
    candidate_triples = top_n(triple_scores, N_triple)
    relevant_triples = LLM_filter(query, candidate_triples)
    seeds = build_seed_distribution(relevant_triples, triple_scores)   # ver HippoRAG 2

    # --- mecanismo 1: Symbolic Anchoring ---
    explicit_entities = NER(query)                       # graphmem: literal-mention matching
    for e in explicit_entities:
        if e not in seeds:
            seeds[e] += ε                                 # ancla débil, ε ≈ 0.2
    normalize(seeds)

    top_seed_nodes = top_n(seeds, N_seed)                 # N_seed ≈ 5

    # --- mecanismo 2: Query-Aware Dynamic Edge Weighting ---
    edge_weight_overlay = {}
    for u in top_seed_nodes:
        candidates = graph.outgoing_edges(u)

        # filtro barato: coarse embedding filter
        scored = [(e, cos(q_vec, embed(e))) for e in candidates]
        top_candidates = top_n(scored, K_edge)            # K_edge ≈ 15

        # segunda pasada, opcional según config: re-puntuar el shortlist
        for (e, coarse_score) in top_candidates:
            relevance = edge_scorer.score(query, e, coarse_score)  # ver EDGE SCORERS más abajo
            multiplier = relevance_to_multiplier(relevance)
            edge_weight_overlay[(u, e.target)] =
                static_weight(u, e.target) * multiplier
            # ← solo se toca la dirección de expansión desde el seed

    # --- mecanismo 3: Key-Fact Passage Enhancement ---
    for t in relevant_triples:
        for (entity, passage) in passages_containing(t):   # provenance: fact → passage
            edge_weight_overlay[(entity, passage)] =
                weight(entity, passage) * (1 + β)            # β ≈ 2.5

    renormalize(edge_weight_overlay, graph)                 # la masa saliente debe seguir sumando 1

    # --- PPR ponderado y dirigido sobre el overlay ---
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
                next_π[v] += π[u] * damping * (w / total)    # ← ponderado, no uniforme
        π = normalize(next_π)
    return π
```

**Nota para graphmem (Fase 2 planeada):** `NER(query)` ya existe como
`text_mentions` literal. `WEIGHTED_PPR` es exactamente el
`edge_weight_overlay: Option<&HashMap<(usize, usize), f64>>` descrito en
`graph_tune.md`; cuando el overlay está vacío, `WEIGHTED_PPR` colapsa a la
`PPR` uniforme de Fase 1. `edge_scorer` es el trait `EdgeScorer` de
`graph_tune.md`, con tres implementaciones posibles, todas detrás de la
misma interfaz — nada en `RETRIEVE_CATRAG`/`WEIGHTED_PPR` cambia al pasar de
una a otra:

```text
# el filtro coarse por coseno (dentro de RETRIEVE_CATRAG, arriba) SIEMPRE
# corre primero y ya produce `coarse_score` para cada candidato del
# shortlist; edge_scorer.score() decide si eso basta o si hay que
# re-puntuar con algo más preciso — se elige vía config, no por código:
#
#   [retrieval] edge_scorer = "cosine" | "cross_encoder"   # default "cosine"

function MAKE_EDGE_SCORER(config):
    match config.edge_scorer:
        "cosine":         return EmbeddingEdgeScorer()
        "cross_encoder":  return CrossEncoderEdgeScorer(load_cross_encoder_model())
        # "llm" no expuesto todavía — ver Tier 3 más abajo


# --- Tier 1: EmbeddingEdgeScorer (default, coste cero adicional) ---
function EMBEDDING_SCORE(query, edge, coarse_score):
    return coarse_score                        # reutiliza el score del filtro coarse, no recalcula nada


# --- Tier 2: CrossEncoderEdgeScorer (reranker local opcional, p.ej. MiniLM) ---
function CROSS_ENCODER_SCORE(query, edge, coarse_score, cross_encoder_model):
    pair_text = (query, edge_document(edge))   # edge_document(e) = "source relation target"
    return cross_encoder_model.forward(pair_text)
    # BERT-family: [CLS] del par (query, edge_document) -> cabeza lineal -> 1 score
    # más preciso que Tier 1 porque query y edge se atienden mutuamente
    # (no se comparan dos embeddings calculados de forma independiente);
    # solo corre sobre el shortlist ya filtrado por coseno (~K_edge candidatos
    # por query), sigue siendo 100% local — sin llamada a un LLM externo


# --- Tier 3: LlmEdgeScorer (fuera de este roadmap por ahora) ---
function LLM_SCORE(query, edge, coarse_score, llm):
    label = llm.classify(query, edge.source, edge.relation, edge.target, context(edge.target))
    return label_to_score(label)               # {Irrelevant, Weak, High, Direct} -> score


function relevance_to_multiplier(relevance):
    return monotonic_map(relevance)             # p.ej. clamp/sigmoid en vez de bucketing por etiqueta
```

Tier 1 ya está disponible hoy (`edge_vector` + coseno, usado globalmente
para elegir seeds; aquí solo se reutiliza el mismo número, sin coste
extra). Tier 2 es la mejora de calidad opcional — mismo rol que
`LLM_classify` en el paper, pero con `cross-encoder/ms-marco-MiniLM-L-6-v2`
o similar en vez de un LLM, cargable vía `candle_transformers::models::bert`
igual que DistilBERT en `embedding.rs`, y una forward pass por candidato
(~`K_edge`≈15 por query — barato en CPU, pero no gratis, de ahí el toggle
por config). Tier 3 queda deliberadamente fuera del roadmap.

---

## GraphFlow (Fase 3)

Retrieval como una secuencia de decisiones entrenada con una idea de
GFlowNets (policy + flow estimator), en vez de "calcula scores → PPR".

### Inferencia: muestrear trayectorias

```text
function RETRIEVE_GRAPHFLOW(query, graph, num_trajectories):
    trajectories = []

    for _ in range(num_trajectories):
        state = start_state(query, graph)          # seed(s) inicial desde la query
        trajectory = [state]

        loop:
            actions = available_actions(state, graph)   # vecinos + acción especial SELF_LOOP
            probs = softmax(policy_scores(state, actions))   # r_θ(state, action)
            action = sample(actions, probs)

            if action == SELF_LOOP:
                break                                # "esto ya es un retrieval útil, parar"

            state = transition(state, action)
            trajectory.append(state)

        trajectories.append(trajectory)

    return aggregate_results(trajectories)            # dedupe + rank por frecuencia/flow
```

### Entrenamiento: consistencia de flujo (detailed balance)

```text
function TRAIN_STEP(query, ground_truth_trajectory, graph, policy, flow_net):
    loss = 0
    trajectory = ground_truth_trajectory               # p.ej. A -> B -> C

    for t in range(len(trajectory) - 1):
        s_t, s_next = trajectory[t], trajectory[t + 1]

        # local exploration: además de C, muestrear vecinos alternativos de B
        alt_actions = available_actions(s_t, graph)
        for s_alt in alt_actions:                       # incluye s_next y exploración (D, E, F...)
            log_F_t    = flow_net(s_t)
            log_F_alt  = flow_net(s_alt)
            log_P_fwd  = policy_log_prob(s_t, s_alt)
            log_P_back = 0                               # backward policy fija a 1 (no hay backtrack)

            # detailed balance: F(s_t) P(s_t+1|s_t) ≈ F(s_t+1) P_back(s_t|s_t+1)
            loss += (log_F_t + log_P_fwd - log_F_alt - log_P_back) ** 2

    reward = terminal_reward(trajectory, ground_truth_trajectory)  # ¿el retrieval final fue bueno?
    loss += flow_matching_terminal_loss(flow_net, trajectory[-1], reward)

    backprop(loss)     # actualiza policy MLP + flow MLP (+ adapters LoRA sobre el LLM backbone)
```

La recompensa solo se observa al final de la trayectoria; `detailed
balance` es lo que permite propagarla hacia decisiones intermedias sin
etiquetar manualmente cada paso, y el muestreo proporcional a la
recompensa (en vez de greedy) es lo que da diversidad entre trayectorias en
inferencia.
