# Improvement ideas

Ideas to raise the numbers in [results.md](results.md) without touching the
base approach (entity graph + embeddings + PageRank), which is already
validated on all three datasets.

## Embedding model

- **Try a multi-hop-retrieval-oriented or larger model.**
  `msmarco-distilbert-cos-v5` is generic and, without a graph, is weak or mixed
  against BM25 (see the no-graph modes in [results.md](results.md)). The Qwen3
  that `embedding.rs` already supports is a candidate, or a model from the
  `bge`/`gte` family trained for multi-hop QA.
- **A model-specific `embed_query` instruction.** A prefix `Instruct:` is
  already used for Qwen3; trying variants of that instruction (or adding one
  for DistilBERT, if the checkpoint supports it) may move recall without a
  graph.

## `[retrieval]` parameters

- **Sweep `damping` and `memory_seed_weight`** for Personalized PageRank across
  the three datasets. They are untouched today; they were never compared side
  by side.
- **IDF-weighted entity seeds**, inside gmem instead of the evaluation script's
  frequency cap (`--spacy-max-df`/`--spacy-min-df`). Today a frequent entity is
  dropped entirely; weighting it by IDF could rescue signal without the
  noisy-PageRank overhead that caused the frequency-cap fallback (see
  [conclusions.md](conclusions.md) and [results.md](results.md)).

## Relation extraction (no LLM)

- **spaCy extracts very few triples** (less than 1 per paragraph). Paragraph
  connection comes mostly from shared entities, not relations. Try richer
  extraction rules (coordination, apposition, pronouns resolved with
  coreference) or a larger spaCy model (`en_core_web_trf`) to see whether the
  number and quality of triples improve.
- **The gap to `oracle` measures the real headroom:** on 2Wiki, recall@10 goes
  from 0.935 (filtered spacy) to 0.985 (oracle); on MuSiQue, from 0.910 to
  1.000. Closing that gap is the biggest improvement opportunity identified so
  far.

## Evaluation

- **More questions per dataset** (100 or more) so each recall point does not
  depend on 1-2 questions and ±0.03 differences are reliable.
- **Measure CPU time on MuSiQue** with and without a graph — today there are
  only WGPU numbers, so the real speedup there cannot be computed as it is for
  HotpotQA/2Wiki.
- **`oracle` on HotpotQA:** the dataset ships no triples of its own, so there
  is no ceiling there; a comparable ceiling would have to be derived from
  another source (e.g. its `supporting_facts`).

## Performance

- **Compile with `RUSTFLAGS="-C target-cpu=native"` (or `x86-64-v3`).** Today
  `gmem` is compiled for generic x86-64 (SSE2 only); candle's own SIMD kernels
  are gated on `target_feature = "avx2"` at compile time, so on CPU they are
  not used even when the CPU supports them. Matrix multiplication (`gemm`)
  already detects AVX2/FMA at runtime, but the rest of the element-wise
  operations do not.
- **Reuse vectors across graphs.** Today each combination (dataset, graph)
  re-embeds from scratch (~0.36 s per memory); if the text does not change
  between graphs, caching the memory vector and re-embedding only new
  entities/edges would save most of each evaluation's time.
- **Measure WGPU's default batch with a larger corpus.** With 100 paragraphs,
  the batch did not change total time (11.2–11.7 s from batch 1 to 16): model
  loading dominates. With more text per measurement, the batch effect should
  show and a default could be set from real data instead of the provisional
  value (8).
- **Propose `enqueue_64_big`** (already used in `binary.rs` in the candle fork)
  for the operations that still use `enqueue_64` (softmax, mask, matmul), in
  candle's PR #3379. It would remove the need for token-budget batching in gmem
  for anyone using the fork directly.
