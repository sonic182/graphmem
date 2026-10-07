# Code-tool agent benchmark: gmem vs a plain agent

Does a coding agent reach the same answer using less context when the gmem
code tools are available? The harness lives in
[`eval/`](../../eval/README.md). Raw data is in `eval/results/`, and the wide
cross-model table is in `eval/results/comparison.md`.

This report uses the latest full DeepSeek rerun: **75/75 correct**, with a
16,384-token generation cap, at an estimated total cost of **$0.7239**.
Nemotron's stored runs have been rescored with the same corrected scorer,
but have not been rerun with the higher cap.

## Conclusions

1. **Cost and tokens answer different questions.** On DeepSeek, neutral gmem
   is cheaper on four of five tasks, but uses more mean total tokens on
   three. Cached input is much cheaper than fresh input, so report both
   provider token usage and estimated cost, rather than treating them as
   interchangeable.
2. **The tools locate code; the agent still has to understand it.** DeepSeek
   reads fewer shell/file source lines with either gmem variant on every
   task. Total tool output does not always fall: tool definitions, guidance,
   skill loads, and repeated navigation can offset the smaller source reads.
3. **Guidance helps some tasks and adds overhead to others.** Guided DeepSeek
   is 30% cheaper on the workflow task and 11% cheaper on the diff task, but
   36% more expensive on imports and 4% more expensive on a single-symbol
   lookup. The shipped guidance is not a universal savings switch.
4. **Value is model-dependent.** Both gmem variants rescue Nemotron's symbol
   lookup accuracy from 2/5 to 5/5, while neutral gmem reduces its accuracy
   on other tasks. Lower cost is not a win if the answer is wrong.
5. **There is no stable, universal savings percentage.** Repeated DeepSeek
   matrices produced materially different workflow and diff costs. Five
   samples per cell, changing cache state, and different generation caps do
   not support a claim that gmem reliably reduces variance or wins every
   structural task.
6. **Method matters.** Use deterministic scoring, show accuracy beside cost,
   distinguish means from medians, and bound runaway agent loops. Normalize
   harmless import quoting instead of counting it as a semantic error.

## Setup

- **Corpus:** `openclaw/openclaw`, pinned to `8f5c33c3` (the llama.cpp
  host-compatibility fix; parent `4de57f22` is the diff base). 45,457 indexed
  source files.
- **Agents:** three variants, fresh `Agent` per run, same model, temperature
  (0.0), checkout and limits within each model's matrix:
  - `control`: `shell`, `read_file`.
  - `gmem`: the two shared tools plus the gmem MCP tools `find_symbol`,
    `code_outline`, `code_imports`, `code_diff` (memory tools filtered out),
    with a **neutral** system prompt that never mentions gmem.
  - `gmem-guided`: as `gmem`, plus the shipped Graphmem plugin guidance and
    a `skill` tool that loads `plugin/skills/*` on demand.
- **Models:** Fireworks `deepseek-v4p1-flash` and
  `nemotron-lightning-3p5-30b-a3b`.
- **Runs:** 5 tasks × 3 variants × 5 runs = 75 runs per model.
- **Generation cap:** latest DeepSeek matrix uses `--max-tokens 16384`;
  stored Nemotron matrix used 8192. The harness default remains 8192.
- **Scoring:** deterministic against `eval/gold.json`; no LLM judge.
  Import strings have matching surrounding quotes/backticks removed before
  computing F1. Both model files were rescored with this correction.
- **Cost:** derived per run from `eval/pricing.json` (rates from models.dev),
  not actual invoice data.

## DeepSeek V4.1 Flash

Mean per run. All 15 cells answered 5/5 correctly. `tool out` is characters;
`src lines` counts only shell and file reads, not MCP source text.

| task | variant | tokens | cost usd | tool out | src lines | cycles |
|---|---|---:|---:|---:|---:|---:|
| `symbol-001` | control | 9,149 | $0.0010 | 9,103 | 166 | 4.2 |
| `symbol-001` | gmem | 11,011 | $0.0007 | 3,163 | 64 | 3.6 |
| `symbol-001` | gmem-guided | 14,703 | $0.0011 | 8,467 | 20 | 3.0 |
| `outline-001` | control | 12,071 | $0.0018 | 7,056 | 212 | 5.4 |
| `outline-001` | gmem | 10,848 | $0.0009 | 4,167 | 28 | 3.0 |
| `outline-001` | gmem-guided | 21,913 | $0.0012 | 11,810 | 35 | 3.8 |
| `imports-001` | control | 11,524 | $0.0010 | 22,570 | 501 | 2.6 |
| `imports-001` | gmem | 7,955 | $0.0007 | 2,653 | 55 | 2.4 |
| `imports-001` | gmem-guided | 16,797 | $0.0013 | 10,138 | 59 | 3.2 |
| `diff-001` | control | 35,082 | $0.0059 | 17,308 | 418 | 8.0 |
| `diff-001` | gmem | 50,209 | $0.0067 | 18,880 | 271 | 8.0 |
| `diff-001` | gmem-guided | 42,811 | $0.0052 | 21,960 | 173 | 5.6 |
| `workflow-001` | control | 780,805 | $0.0438 | 79,535 | 2,758 | 27.6 |
| `workflow-001` | gmem | 788,386 | $0.0427 | 75,444 | 2,378 | 24.0 |
| `workflow-001` | gmem-guided | 695,266 | $0.0308 | 87,853 | 2,286 | 24.4 |

Change vs control, based on unrounded means. Negative values mean fewer
tokens or lower estimated cost.

| task | gmem tokens | gmem cost | guided tokens | guided cost |
|---|---:|---:|---:|---:|
| `diff-001` | +43% | +13% | +22% | −11% |
| `imports-001` | −31% | −30% | +46% | +36% |
| `outline-001` | −10% | −52% | +82% | −32% |
| `symbol-001` | +20% | −35% | +61% | +4% |
| `workflow-001` | +1% | −3% | −11% | −30% |

### What it says

- **Neutral gmem is cheaper on four of five tasks**, but not on the diff
  task. Guided gmem is cheaper on three of five. Claims that `code_diff`
  always wins on both cost and tokens are not supported by this rerun.
- **Guidance helps the workflow task:** mean cost falls 30% and mean total
  tokens fall 11%. Its median tokens fall 31%; the mean and median are
  different statistics, not contradictory results.
- **Source reads shrink more consistently than total tool output.** Neutral
  gmem cuts shell/file lines by about 14–89%; guided cuts them by 17–88%.
  On the diff task, however, both gmem variants ingest more total tool-output
  characters than control. Guided outline and workflow output also increases.
- **Small lookups pay a fixed prompt overhead.** The added tool definitions
  and guidance can outweigh navigation savings. On `symbol-001`, neutral
  gmem uses 20% more mean tokens but costs 35% less; guided costs 4% more.

### Why the matrix was rerun

The preceding 8192-token matrix scored 73/75. One guided diff run stopped
mid-tool-call with `MaxTokensReachedException`. A guided import run returned
all correct specifiers, but with literal quotes inside the JSON strings;
this scored zero before quote normalization. Rescoring fixed the latter,
leaving 74/75. Rather than selectively replacing the failed sample, the
entire DeepSeek matrix was rerun with a 16,384-token cap. The current 75/75
result contains only that full rerun.

## Cross-model: Nemotron Lightning 3.5 30B A3B

Median tokens / mean cost per run, with accuracy. These are the stored
8192-token runs, rescored without new API calls: **53/75 correct**.

| task | control | gmem | gmem-guided |
|---|---:|---:|---:|
| `diff-001` | 386,450 / $0.0102 (1/5) | 210,226 / $0.0062 (1/5) | 203,658 / $0.0043 (2/5) |
| `imports-001` | 27,487 / $0.0012 (4/5) | 56,834 / $0.0016 (3/5) | 8,326 / $0.0003 (5/5) |
| `outline-001` | 84,876 / $0.0023 (5/5) | 101,900 / $0.0026 (3/5) | 121,319 / $0.0089 (4/5) |
| `symbol-001` | 14,389 / $0.0003 (2/5) | 6,202 / $0.0001 (5/5) | 7,572 / $0.0002 (5/5) |
| `workflow-001` | 366,803 / $0.0082 (5/5) | 606,263 / $0.0268 (3/5) | 505,470 / $0.0081 (5/5) |

### What it says

- **The tools can rescue a weak model.** On `symbol-001`, Nemotron's control
  scores 2/5 on 14k median tokens; both gmem variants score 5/5 on roughly
  6–8k. Guided imports reaches 5/5 at $0.0003 against 4/5 at $0.0012.
- **The tools can also hurt it.** Neutral gmem drops accuracy on outline,
  workflow, and imports; guided outline is 4/5 at nearly four times the cost.
- **Diff cost savings are not reliable answers.** Guided diff is 58% cheaper
  than control, but only 2/5 answers are fully correct.
- **Runaway loops distort means.** One guided outline run used 107 cycles,
  102 shell calls, and 2.7M tokens. The cross-model token table uses medians;
  costs remain means because the outlier still incurs spend.

## Limitations

- Five runs per cell and one corpus. Differences are directional, not precise.
  Repeated matrices changed the apparent workflow and diff winners; this is
  not enough evidence to claim a general reduction in run-to-run variance.
- DeepSeek and Nemotron currently have different generation caps. Compare
  variants within a model; cross-model differences are not controlled for
  that setting. Rerun Nemotron with the same cap before making stronger claims.
- `--max-tokens` limits one generation, not total cycles, elapsed time, or
  total run tokens. There is still no explicit agent-loop cap.
- Cache state and concurrent request ordering are not controlled. Cost uses
  published list prices and the models.dev cached-input rate.
- `source_lines_read` excludes MCP output. Use `tool_output_chars` alongside
  it; fewer shell/file lines alone does not prove less total context.
- Provider-reported token counts include reasoning tokens when reported.
  The runs logged LiteLLM warnings that `reasoningContent` is unsupported in
  multi-turn Chat Completions; its effect on agent behavior was not isolated.
- Run records do not currently capture the generation cap or a complete
  reproducibility manifest; the explicit commands and settings here matter.

## Next steps

1. Add GLM 5.3 Flash and MiniMax M3 to the cross-model table.
2. Record the full run configuration and bound cycles or wall time; rerun
   Nemotron at the same generation cap.
3. Add larger structural navigation tasks and more corpora; repeat matrices
   before claiming stable savings.
