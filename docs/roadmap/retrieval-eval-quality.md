# graphmem retrieval — measure quality, not just behavior

Today there is no way to know whether a retrieval change (tweaking a default
in `[retrieval]`, or later CatRAG vs HippoRAG 2, see `graph-tune.md`) improves
or worsens results overall. There are only point tests.

## What exists today

- `src/retrieval_eval.rs`: a fixture of ~9 memories with a deterministic
  embedder (`BagOfWords`, not the real model) and `assert_eq!`/`assert!` on
  concrete cases (e.g. `ranks_the_memory_that_matches_the_whole_query_first`,
  `the_graph_channel_weight_moves_score_toward_linked_memories`).
- `sweeps_the_memory_channel_weight` (`#[ignore]`): prints a table for manual
  inspection, computes no aggregate metric.

This verifies *point behavior* (not breaking a known case when touching a
knob), not *aggregate quality*.

## What is missing

- [x] A fixed `(query, expected_memory)` set larger than the current ~9 cases
      — covered outside Rust with `scripts/eval_retrieval.py` over HotpotQA,
      2WikiMultihopQA, and MuSiQue (`download_eval_datasets.py`)
- [x] An aggregate metric such as `recall@k` and/or `MRR` over that set
- [x] A graph to evaluate the HippoRAG part without an LLM:
      `scripts/extract_eval_graphs.py` extracts entities and SVO triples with
      spaCy (`en_core_web_sm`) for all paragraphs alike (`--graphs spacy`).
      `oracle` (dataset triples) only serves as a ceiling: it covers only the
      answer path
- [x] Shared corpus (`--corpus shared`, default): all paragraphs in one scope,
      as HippoRAG evaluates; per question recall@10 ≈ 1
- [x] Decide `BagOfWords` vs real embedder: the script uses the real one
      (`msmarco-distilbert-cos-v5`, CPU, batch 1 by default)
- [ ] Replace (or complement) `sweeps_the_memory_channel_weight` with a
      version that prints the aggregate metric per knob value, not just the raw
      ranking — in the script: pass `GRAPHMEM_RETRIEVAL_*` and add a column per
      configuration
- [ ] A mechanism to compare two configurations/algorithms side by side (today
      HippoRAG 2 vs HippoRAG 2 with a different `damping`; later, HippoRAG 2 vs
      CatRAG once Phase 2 exists) over the same set
- [ ] Save results (JSON with commit, model, config, and metrics) to compare
      across commits
- [ ] Reuse memory vectors across graphs: today each combination re-embeds them
      (~0.4 s/paragraph on CPU)
- [ ] spaCy relations are sparse (<1 per paragraph): consider extra patterns
      (appositions, "X is a Y") or a bigger model (`--model`)
