# Conclusions

Summary of the retrieval-quality work. Numbers are in
[results.md](results.md).

## What works

- **Embeddings + graph is the best configuration** on all three benchmarks at
  the top ranks (recall@2/5, MRR). The lift comes from the graph: adding one
  beats every no-graph variant on every dataset.
- **Without a graph, embeddings barely beat BM25.** `msmarco-distilbert` alone
  is roughly level with FTS5 (sometimes behind, e.g. 2Wiki recall@10), which
  is why semantic-only recall is not enough for multi-hop questions.
- **With a graph, embeddings clearly beat BM25**, because PageRank reaches the
  second supporting paragraph through shared entities that lexical matching
  misses.
- **Graph quality matters more than graph size.** spaCy with no filtering
  *hurt* HotpotQA recall@5. Filtering the corpus graph — canonical titles,
  drop entities outside a document-frequency band, drop date/nationality
  labels — improved all three datasets.
- **The ceiling is still higher.** Feeding the datasets' gold triples
  (`oracle`, which leaks the answer) reaches much higher recall, so the
  remaining gap is relation-extraction quality, not the scoring pipeline.
  spaCy yields under 1 relation per paragraph; paragraphs mostly connect via
  shared entities.

## Which graph

`mentions` (title-mention edges) and `spacy` (SVO triples) trade blows by
dataset: `mentions` wins HotpotQA and 2Wiki at 100 questions, `spacy` wins
MuSiQue. A combined graph is the natural next step.

## Other findings

- **Lexical recall** now ORs a plain query's terms and ranks by BM25
  (previously an implicit AND made full questions match nothing). Strict FTS5
  syntax still triggers on uppercase `AND` / `OR` / `NOT` / `NEAR`.
- **Backends** produce identical recall. WGPU is ~3–4× faster than CPU on an
  iGPU; CUDA is comparable, and the cost scales with graph size. CPU defaults
  to batch 1 because padding costs more than batching saves there.
- **Evaluation must use the shared corpus.** With per-question paragraphs,
  recall@10 is ~1 and cannot separate approaches.

## Pending

- More questions / full validation split to tighten confidence.
- Reuse memory vectors across graph runs (today each graph re-embeds).
- Tune `[retrieval]` parameters side by side.
- Better relation extraction (LLM OpenIE, richer spaCy patterns) or a
  combined `mentions` + `spacy` graph.
- Measure `RUSTFLAGS="-C target-cpu=native"` for CPU builds.
