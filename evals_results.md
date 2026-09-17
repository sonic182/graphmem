# Retrieval evaluation results

Concrete numbers for `gmem` recall on three multi-hop QA benchmarks. For the
interpretation, see [conclusions.md](conclusions.md).

## Setup

- **Scripts:** `scripts/eval_retrieval.py` drives the `gmem mcp` server over
  stdio; graphs come from `scripts/extract_eval_graphs.py` (spaCy
  `en_core_web_sm`, no LLM). Datasets come from
  `scripts/download_eval_datasets.py`.
- **Datasets:** HotpotQA (distractor), 2WikiMultihopQA and MuSiQue, validation
  split, first 100 questions each.
- **Corpus:** `shared` (default) — every paragraph of the selected questions,
  deduplicated, in one scope; each question ranks the whole corpus, as
  HippoRAG evaluates. (`per-question` keeps each question's own paragraphs,
  where recall@10 is ~1 and nothing is discriminated.)
- **Baseline model:** `sentence-transformers/msmarco-distilbert-cos-v5`.
- **Binary:** release `gmem` built with `--features cuda`,
  `GRAPHMEM_EMBEDDING_BACKEND=auto` → CUDA (RTX 4060), batch 16. The
  50-question run reproduces the earlier CPU batch-1 numbers exactly, so the
  backend changes speed, not quality.
- **Metrics:** mean over questions. `recall@k` = fraction of supporting
  paragraphs in the top k; `MRR` = 1 / rank of the first supporting paragraph.

Graphs:

- `none`: no entities.
- `mentions`: one entity per paragraph title, plus an edge when a paragraph's
  text mentions another title from the same question.
- `spacy`: spaCy entities and subject-verb-object triples, corpus-filtered
  (`--spacy-max-df 0.02 --spacy-min-df 2`).
- `oracle`: the dataset's own gold triples. These cover only the answer path,
  so this **leaks the answer** and is only a ceiling.

Modes:

- `embeddings`: `recall` with `use_embeddings=true` (semantic seeds +
  Personalized PageRank).
- `fts-raw`: `use_embeddings=false` with the raw question.
- `fts-or`: `use_embeddings=false` with the question's words OR-joined.

## MS MARCO DistilBERT: 100 questions, shared corpus

| dataset | graph | mode | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|---|
| HotpotQA | none | embeddings | 0.580 | 0.725 | 0.830 | 0.889 |
| HotpotQA | none | fts-raw | 0.540 | 0.740 | 0.895 | 0.872 |
| HotpotQA | none | fts-or | 0.550 | 0.745 | 0.895 | 0.885 |
| HotpotQA | mentions | embeddings | **0.645** | **0.825** | **0.940** | **0.896** |
| HotpotQA | spacy | embeddings | 0.620 | 0.800 | 0.920 | 0.887 |
| 2Wiki | none | embeddings | 0.615 | 0.698 | 0.728 | 0.953 |
| 2Wiki | none | fts-raw | 0.605 | 0.690 | 0.755 | 0.950 |
| 2Wiki | none | fts-or | 0.605 | 0.690 | 0.755 | 0.950 |
| 2Wiki | mentions | embeddings | **0.723** | **0.915** | **0.968** | 0.962 |
| 2Wiki | spacy | embeddings | 0.705 | 0.833 | 0.907 | **0.980** |
| 2Wiki | oracle | embeddings | 0.828 | 0.963 | 0.965 | 0.985 |
| MuSiQue | none | embeddings | 0.490 | 0.580 | 0.655 | 0.858 |
| MuSiQue | none | fts-raw | 0.465 | 0.520 | 0.565 | 0.812 |
| MuSiQue | none | fts-or | 0.465 | 0.520 | 0.565 | 0.812 |
| MuSiQue | mentions | embeddings | 0.560 | 0.685 | 0.740 | 0.888 |
| MuSiQue | spacy | embeddings | **0.605** | **0.735** | **0.855** | **0.891** |
| MuSiQue | oracle | embeddings | 0.765 | 0.855 | 0.885 | 0.930 |

## all-MiniLM-L6-v2: 100 questions, shared corpus

Run at commit `0db8dcf` with
`GRAPHMEM_EMBEDDING_MODEL=sentence-transformers/all-MiniLM-L6-v2`, the same
CUDA release configuration, RTX 4060, batch 16, corpus, graph extraction, and first
100 validation questions as the DistilBERT baseline above. The complete sweep
(11 reembeds) took **104 s**.

| dataset | graph | mode | recall@2 | recall@5 | recall@10 | MRR |
|---|---|---|---|---|---|---|
| HotpotQA | none | embeddings | 0.155 | 0.215 | 0.250 | 0.300 |
| HotpotQA | none | fts-raw | 0.540 | 0.740 | 0.895 | 0.872 |
| HotpotQA | none | fts-or | 0.550 | 0.745 | 0.895 | 0.885 |
| HotpotQA | mentions | embeddings | 0.345 | 0.485 | 0.620 | 0.571 |
| HotpotQA | spacy | embeddings | **0.380** | **0.525** | **0.695** | **0.639** |
| 2Wiki | none | embeddings | 0.033 | 0.048 | 0.048 | 0.065 |
| 2Wiki | none | fts-raw | 0.605 | 0.690 | 0.755 | 0.950 |
| 2Wiki | none | fts-or | 0.605 | 0.690 | 0.755 | 0.950 |
| 2Wiki | mentions | embeddings | 0.453 | 0.580 | 0.660 | 0.806 |
| 2Wiki | spacy | embeddings | **0.557** | **0.657** | **0.688** | **0.924** |
| 2Wiki | oracle | embeddings | 0.682 | 0.863 | 0.885 | 0.882 |
| MuSiQue | none | embeddings | 0.065 | 0.090 | 0.100 | 0.129 |
| MuSiQue | none | fts-raw | 0.465 | 0.520 | 0.565 | 0.812 |
| MuSiQue | none | fts-or | 0.465 | 0.520 | 0.565 | 0.812 |
| MuSiQue | mentions | embeddings | 0.295 | 0.355 | 0.395 | 0.510 |
| MuSiQue | spacy | embeddings | **0.305** | **0.380** | **0.425** | **0.558** |
| MuSiQue | oracle | embeddings | 0.460 | 0.560 | 0.705 | 0.733 |

MiniLM is faster for this sweep but its embedding recall is lower than the
MS MARCO DistilBERT baseline in every non-oracle setting. The lexical rows are
unchanged because they do not use embeddings. As with the baseline, graph
context substantially improves MiniLM over its `none` configuration.

## Best non-oracle configuration (MS MARCO DistilBERT)

| dataset | graph | recall@2 | recall@5 | recall@10 |
|---|---|---|---|---|
| HotpotQA | mentions | 0.645 | 0.825 | 0.940 |
| 2Wiki | mentions | 0.723 | 0.915 | 0.968 |
| MuSiQue | spacy | 0.605 | 0.735 | 0.855 |

Adding a graph improves every dataset at the top ranks; without one,
embeddings are roughly level with BM25 (HotpotQA and 2Wiki FTS even lead at
recall@10).

## 50 vs 100 questions

The ordering above already held at 50 questions; absolute numbers were higher
because the harder tail is absent. The 50-question run is also the CPU-parity
check (identical recall/MRR to the earlier CPU batch-1 run).

| dataset | graph | 50q recall@2 / @5 / @10 | 100q recall@2 / @5 / @10 |
|---|---|---|---|
| HotpotQA | mentions | 0.650 / 0.840 / 0.930 | 0.645 / 0.825 / 0.940 |
| HotpotQA | spacy | 0.670 / 0.840 / 0.930 | 0.620 / 0.800 / 0.920 |
| 2Wiki | mentions | 0.700 / 0.910 / 0.965 | 0.723 / 0.915 / 0.968 |
| 2Wiki | spacy | 0.660 / 0.825 / 0.935 | 0.705 / 0.833 / 0.907 |
| MuSiQue | mentions | 0.620 / 0.690 / 0.730 | 0.560 / 0.685 / 0.740 |
| MuSiQue | spacy | 0.700 / 0.820 / 0.910 | 0.605 / 0.735 / 0.855 |

`mentions` and `spacy` trade blows: at 100, `spacy` wins MuSiQue, `mentions`
wins HotpotQA and 2Wiki; the two are within noise of each other.

## Lexical recall fix

Before, FTS5 required every whitespace term (implicit AND), so a full natural
-language question matched nothing. Now a plain query ORs its terms and BM25
ranks; quoted phrases and `prefix*` are preserved, and only an uppercase
`AND` / `OR` / `NOT` / `NEAR` switches to strict FTS5 syntax.

| dataset | `fts-raw` before | `fts-raw` now | `fts-or` |
|---|---|---|---|
| HotpotQA | 0 / 0 / 0 / 0 | 0.540 / 0.740 / 0.895 / 0.872 | 0.550 / 0.745 / 0.895 / 0.885 |
| 2Wiki | 0 / 0 / 0 / 0 | 0.605 / 0.690 / 0.755 / 0.950 | 0.605 / 0.690 / 0.755 / 0.950 |
| MuSiQue | 0 / 0 / 0 / 0 | 0.465 / 0.520 / 0.565 / 0.812 | 0.465 / 0.520 / 0.565 / 0.812 |

Because the OR-join now applies to plain queries, `fts-raw` is effectively a
second OR baseline rather than an all-terms baseline.

## Performance

`gmem reembed` over 100 HotpotQA paragraphs:

| backend | batch | time |
|---|---|---|
| CPU | 1 | ~40 s |
| CPU | 8 | 50.4 s |
| CPU | 32 | 78.8 s |
| WGPU (Vega iGPU) | 1 | 11.2 s (3.7×) |
| WGPU | 4 / 8 / 16 | ~11.4–11.7 s |

- CPU defaults to batch 1: padding to the longest text costs more than
  batching saves there. CUDA defaults to batch 16.
- The full 100-question sweep (3 datasets, all graphs/modes, 11 reembeds) took
  **175 s** on CUDA vs 72 s for the 50-question sweep.
- The same 100-question CUDA sweep took **104 s** with all-MiniLM-L6-v2.
- All backends produce identical recall/MRR; the batching changes speed only.

## Notes

- n = 100 questions per dataset; one recall point ≈ one question, so ±0.02–0.03
  is noise.
- `spacy` extracts fewer than 1 relation per paragraph; paragraphs are linked
  mostly through shared entities.
- `oracle` confirms the ceiling: with gold triples, 2Wiki reaches
  recall@10 ≥ 0.965 and MuSiQue 0.885, well above the no-LLM graphs.

## Pending

- More questions (or the full validation split) to tighten confidence.
- Reuse memory vectors across graph runs instead of re-embedding per graph.
- Tune `[retrieval]` (`damping`, `memory_seed_weight`, …) side by side.
- Improve relation extraction (broader spaCy patterns or LLM OpenIE), or a
  combined `mentions` + `spacy` graph.
- Measure `RUSTFLAGS="-C target-cpu=native"` for CPU builds.
