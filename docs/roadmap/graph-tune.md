# graphmem retrieval — roadmap toward CatRAG

Conceptual order: **Phase 1, HippoRAG 2 (base, implemented) → Phase 2, CatRAG
(dynamic edge weighting, current target) → Phase 3, GraphFlow (learned
retrieval, future ceiling, unplanned)**.

---

## Phase 1 — HippoRAG 2: graph + PPR as the base

**Paper:** *From RAG to Memory: Non-Parametric Continual Learning for Large Language Models*, ICML 2025. ([Paper ICML/PMLR][1]) · [Official code][hippocode]

- [x] Passage/memory nodes inside the graph
- [x] Memory embeddings (`Qwen/Qwen3-Embedding-0.6B` in V1; see Phase 2.5 / PR #2 for the model switch)
- [x] Explicit memory-entity links
- [x] Bidirectional PPR with damping `0.5`
- [x] Scope filter applied before building the returnable memory nodes

A vector search finds semantically similar things but can miss a chain of
relations. HippoRAG 2 puts the documents and their concepts in one graph and
uses Personalized PageRank (PPR) to propagate relevance through multi-hop
relations, avoiding that multi-hop gain destroying recall on simple questions.
([Paper ICML/PMLR][1])

The graph has two kinds of nodes:

```text
Phrase / Entity ──relation──▶ Phrase / Entity
Phrase / Entity ──context───▶ Passage
```

During indexing, an LLM does OpenIE (`"Johanderson develops Minibot in
Python"` → `(Johanderson, develops, Minibot)`, `(Minibot, uses, Python)`); the
terms become phrase nodes and the original passage remains a node. Relation
edges, synonym edges between similar concepts, and context edges between
concepts and passages are added. ([OpenReview][2])

At query time: first the query is embedded against the **full triples**, not
just entity names. Then an LLM does **Recognition Memory**: it filters those
triples and keeps the ones related to the question's intent; the surviving
concepts become strong PPR seeds. ([Memory Papers][3]) There is a second
signal — direct query↔passage similarity — mixed in with a small factor
(`λ≈0.05`) so it does not eclipse the graph. Damping `0.5`. ([Official
code][4])

```text
π(t+1) = (1-d)·seed + d·Tᵀ·π(t)
score(passage) = PPR[passage]
```

It requires training no model — which is why it was the first reasonable base
for graphmem.

---

## Phase 1.1 — scoring fix

- [x] Personalization vector with two channels normalized separately (memory + graph)
- [x] Temperature `0.05` softmax for memories (top-k by cosine) — restores the contrast the embeddings' narrow band was destroying
- [x] Symbolic anchoring: additive anchor `ε` (`entity_anchor_weight`) for entities named literally in the query, via `text_mentions`
- [x] Entity embeddings of their own — a paraphrase can anchor an entity without needing edges
- [x] Memory/graph mixture controlled by `memory_seed_weight`
- [x] Linear PPR: dangling mass accumulates into a scalar instead of being spread node by node
- [x] Retrieval constants moved to `[retrieval]` in `config.toml`
- [x] `src/retrieval_eval.rs` pins the behavior with a deterministic embedder to sweep values

This phase fixed the part that did not behave like the paper (the scoring)
without yet touching the graph shape or PPR transitions. It deliberately left
open the per-query `edge_weights` gap — which is exactly Phase 2.

---

## Phase 2 — CatRAG: dynamic edge weighting (current target)

**Paper:** *Breaking the Static Graph: Context-Aware Traversal for Graph-Based RAG*, Findings ACL 2026. ([Paper ACL][5]) · [CatRAG repository][6]

CatRAG starts from HippoRAG 2 and points out a flaw: the query changes the
seed vector, but the graph's edge weights stay fixed (the **"Static Graph
Fallacy"**). `Johanderson --develops--> Minibot`, `--likes--> Python`,
`--lives--> Madrid` have the same structural weight regardless of whether you
ask about software, cities, or cars.

### Current state vs official CatRAG

(Checked against `src/catrag/CatRAG.py`, published on August 20.)

- [x] Passage/memory nodes inside the graph
- [x] Embeddings for memories, entities, and triples/edges
- [x] PPR with `damping=0.5`
- [x] Seed mixture memory + graph
- [x] Symbolic anchoring (equivalent to CatRAG's, without LLM NER)
- [x] Query→fact/edge similarity (`edge_vector` + `edge_document()` = `"source relation target"`)
- [ ] Coarse edge filtering — 🟡 infrastructure ready (`edge_vector`), still needs activation as a local filter instead of a global one
- [ ] Query-specific edge weights
- [ ] Weighted/directed PPR
- [ ] Key-Fact Passage Enhancement (`memory_edges` provenance + `×2.5` boost)
- [ ] Cross-encoder edge scorer (local reranker, MiniLM) — planned, replaces the paper's `LLM_classify`
- [ ] Recognition Memory (LLM triple filter) — deliberately deferred, out of this roadmap

Estimate: ~65–70% of a reasonable "CatRAG-lite" for graphmem; ~45–50% of a
faithful reproduction of the full paper (which also requires an LLM scorer).

`semantic_results()` already computes almost every signal needed: query
embedding, memory scores, entity scores, and edge scores using the full
triple, turning the best edges into seeds and running PPR. The missing jump is
conceptually small: for the `query↔edge` signal to feed the random walk's
**transitions**, not just the **restart vector**. Today
`personalized_pagerank` receives a flat adjacency (`adjacency: &[Vec<usize>]`)
and distributes `rank[node] * damping / neighbors.len()` equally among
neighbors — that is literally the Static Graph Fallacy.

CatRAG's three mechanisms and how they map to graphmem:

1. **Symbolic Anchoring** — ✅ done (`entity_anchor_weight` + `text_mentions`, no LLM call for NER).
2. **Query-Aware Dynamic Edge Weighting** — cheap cosine filter over the outgoing edges of the top seeds (`Nseed=5`, `Kedge=15` in the official paper), and only then (in the paper) an LLM classifies each edge into `Irrelevant/Weak/High/Direct` → weight multiplier. graphmem replaces that LLM with a **local cross-encoder reranker of the MiniLM type** (e.g. `cross-encoder/ms-marco-MiniLM-L-6-v2`): same role as query↔fact judge, but a small BERT-family model, loadable with `candle_transformers::models::bert` just like DistilBERT in `embedding.rs` today, with no external API. It gives a continuous score instead of 4 labels, mapped to a multiplier with a monotonic function — verified with `retrieval_eval.rs`, not with an external judge.
3. **Key-Fact Passage Enhancement** — if a triple is relevant, the passages containing it get `new_weight(entity, passage) = weight * (1 + β)` with `β=2.5` in the paper. This requires knowing which facts appear in which passage — graphmem does not store that relation today.

```text
                 original                          query-aware
Minibot ─uses──────── Python   w=1        Minibot ─uses──────── Python   w=8
       ─created_by── Johanderson w=1             ─created_by── Johanderson w=.2
       ─hosted_on─── Debian     w=1             ─hosted_on─── Debian     w=.5
```

### Implementation sub-phases (small deliverables, no LLM and no external calls)

Phase 2 is split into four small PRs. `2a`+`2b` are the CatRAG core; `2c` and
`2d` are independent improvements layered on top. Each one leaves retrieval
working on its own.

#### Phase 2a — weight-overlay plumbing (minimal PR)

- [ ] `personalized_pagerank` accepts an optional `edge_weight_overlay: Option<&HashMap<(usize, usize), f64>>` — empty = current behavior (Phase 1/1.1 untouched, additive change).
- [ ] Test in `domain.rs` verifying that an empty overlay reproduces the old signature's ranking exactly.

Value: isolates the `domain.rs` signature change from the new scoring. With no
overlay there is no new behavior, so it does not break Phase 1/1.1.

#### Phase 2b — dynamic edge weights from the seeds (CatRAG core)

- [ ] For the top seed entities, take only their outgoing edges and reuse the `query↔edge_vector` score already computed today (today it is used globally to pick seeds; here it is used locally to weight transitions); map cosine → multiplier with a simple monotonic function.
- [ ] Feed that overlay into the PPR transitions via the `2a` parameter: let the `query↔edge` signal move the random walk's transitions, not just the restart vector.
- [ ] Enable coarse edge filtering as a **local** filter (top seeds × `K_edge`), not a global one.
- [ ] New "hub semantic drift" fixture in `retrieval_eval.rs` (an entity with many edges across different topics) to verify that reweighting reduces spreading toward irrelevant neighbors, not only improves already-easy cases.

Value: this is the conceptual jump — it literally removes the Static Graph
Fallacy. All the signal is already computed; only where it is consumed
changes.

#### Phase 2c — provenance and Key-Fact Passage Enhancement

- [ ] Provenance table:
  ```sql
  CREATE TABLE memory_edges (
      memory_id INTEGER NOT NULL,
      edge_id   INTEGER NOT NULL,
      PRIMARY KEY (memory_id, edge_id)
  );
  ```
  without duplicating text or attributes; `remember_with_graph` populates it when creating a memory's edges.
- [ ] Schema migration for existing stores.
- [ ] Key-Fact Passage Enhancement: when an edge is relevant, `×2.5` boost of the corresponding entity→memory transition via `memory_edges`, renormalizing the total mass.

Value: separable from scoring (it is a migration + provenance). It requires
`2a` to boost transitions, but not `2b`.

#### Phase 2d — cross-encoder edge scorer (optional quality)

- [ ] `EdgeScorer` trait to decouple "how a candidate edge is scored" from PPR, with three tiers behind the same interface:
  ```rust
  trait EdgeScorer {
      fn score(&self, query: &str, candidates: &[EdgeCandidate]) -> Vec<f64>;
  }
  ```
  - `EmbeddingEdgeScorer` (uses the coarse filter's cosine directly + monotonic mapping, already available as a signal today) — default, zero extra cost.
  - `CrossEncoderEdgeScorer` (re-scores the same shortlist with the local MiniLM reranker, see mechanism 2 above) — optional quality improvement via config, still 100% local/no API. Load a MiniLM cross-encoder (e.g. `cross-encoder/ms-marco-MiniLM-L-6-v2`, BERT-family, via `candle_transformers::models::bert`) that scores `(query, edge_document(edge))` as a joint pair instead of comparing independent embeddings — the same "retrieve with bi-encoder, rerank with cross-encoder" pattern already used by search bi-encoder + this reranker.
  - `LlmEdgeScorer` (external LLM like the original paper) — future possibility behind the same interface, **do not implement yet**.
- [ ] Config `[retrieval] edge_scorer = "cosine" | "cross_encoder"` (default `"cosine"`), same pattern as `[embedding] model`/`enabled` (`config.toml` + env var override). It is not a mutually exclusive engine choice: the coarse cosine filter **always** runs first (it selects the `K_edge` candidates reusing already-computed embeddings); the config only decides whether that shortlist is additionally re-scored with the cross-encoder before being turned into a multiplier. `"cosine"` is lighter (~0 cost, already computed); `"cross_encoder"` adds one forward pass per candidate (~`K_edge`≈15 per query) in exchange for higher precision.

Value: the most expensive and the most isolable. The trait waits until the
second real scorer exists; it is not introduced earlier (YAGNI).

The target API is the same one imagined from the start:

```python
graph.pagerank(
    seeds={...},
    edge_weights={...},   # optional overlay, specific to the query
)
```

Empty `edge_weights` → Phase 1 (HippoRAG 2). `edge_weights =
score_edges(query, candidate_edges)` → CatRAG core.

### Phase 2.5 — prerequisite already merged (PR #2)

- [x] Embedding switch to `sentence-transformers/msmarco-distilbert-cos-v5`
- [x] `gmem reembed` as an explicit migration command
- [x] Truncation to the model's `max_position_embeddings` (512 tokens by default)
- [x] `reembed` resilient to per-item failures (does not abort the whole store over one record)

It does not advance the retrieval algorithm, but it leaves
memories/entities/edges with consistent embeddings under a single model,
explicitly re-migratable — safer for experimenting with dynamic edge
weighting on top.

---

## Phase 3 — GraphFlow: learned retrieval (future ceiling, unplanned)

**Paper:** *Can Knowledge-Graph-based Retrieval Augmented Generation Really Retrieve What You Need?*, NeurIPS 2025 Spotlight. ([Paper NeurIPS][7]) · [GraphFlow code][graphflowcode]

- [ ] Trajectory sampler (retrieval as a sequence of decisions, not just scores + PPR)
- [ ] Policy model (shared LLM backbone + LoRA adapters + policy MLP)
- [ ] Flow estimator (`log F(state)`, GFlowNet-style consistency / detailed balance)
- [ ] Self-loop action (`node → itself`) to learn when to stop instead of fixing `max_hops`
- [ ] Local exploration during training (sample alternative neighbors, not only the ground-truth path)
- [ ] Training dataset with labeled trajectories

HippoRAG/CatRAG are "compute scores → run PPR". GraphFlow treats retrieval as
a sequence of decisions (`Johanderson → Minibot → Python → asyncio`) trained
with a GFlowNets idea: it simultaneously learns a policy and a flow function
`F(state)` that lets the final reward propagate back to intermediate decisions
without manually labeling each step. Since the traversal does not backtrack,
the backward policy simplifies to `1`.

At inference several trajectories are sampled in parallel; by assigning
probability proportional to reward (instead of collapsing to the maximum), it
tends to produce several distinct good results — strong gains in recall and
deduplicated recall on STaRK (~+10% average over strong baselines). ([Paper
NeurIPS][7])

For graphmem this would already imply training (policy + flow model +
dataset), not just a scoring function:

```text
GraphMem
  ├── graph store
  ├── vector embeddings
  ├── query states
  ├── trajectory sampler
  ├── policy model
  ├── flow estimator
  └── training dataset
```

It would not be implemented as the project's first algorithm — it is the
roadmap's ceiling, not a dated goal.

---

[1]: https://proceedings.mlr.press/v267/gutierrez25a.html "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models"
[hippocode]: https://github.com/OSU-NLP-Group/HippoRAG "Official HippoRAG code"
[2]: https://openreview.net/pdf?id=LWH8yn4HS2 "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models"
[3]: https://memorypapers.org/papers/hipporag-2-rag-to-memory "From RAG to Memory: Non-Parametric Continual Learning for Large Language Models | Memory Papers"
[4]: https://github.com/lucagattoni/Pinakes/blob/main/docs/graph/hipporag.md "pinakes/docs/graph/hipporag.md at main · lucagattoni/pinakes · GitHub"
[5]: https://aclanthology.org/2026.findings-acl.290/ "Breaking the Static Graph: Context-Aware Traversal for Graph-Based RAG - ACL Anthology"
[6]: https://github.com/kwunhang/CatRAG "GitHub - kwunhang/CatRAG"
[7]: https://proceedings.neurips.cc/paper_files/paper/2025/hash/89d0d5c2f720921df93bbb8fef514571-Abstract-Conference.html "Can Knowledge-Graph-based Retrieval Augmented Generation Really Retrieve What You Need?"
[graphflowcode]: https://github.com/Samyu0304/GraphFlow "GraphFlow code"
