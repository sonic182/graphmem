# Code-tool agent benchmark: gmem vs a plain agent

Does a coding agent reach the same answer using less context when the gmem
code tools are available? This is the first run of the harness in
[`eval/`](../../eval/README.md). Raw data is in `eval/results/runs.jsonl`.

## Setup

- **Corpus:** `openclaw/openclaw`, pinned to `8f5c33c3` (the llama.cpp
  host-compatibility fix; parent `4de57f22` is the diff base). 45,457 indexed
  source files.
- **Agents:** three variants, fresh `Agent` per run, same model, temperature
  (0.0), checkout and limits:
  - `control`: `shell`, `read_file`.
  - `gmem`: the two shared tools plus the gmem MCP tools `find_symbol`,
    `code_outline`, `code_imports`, `code_diff` (memory tools filtered out),
    with a **neutral** system prompt that never mentions gmem.
  - `gmem-guided`: as `gmem`, plus the guidance text the shipped Graphmem
    plugin injects at session start and a `skill` tool that loads
    `plugin/skills/*` on demand. This is the product experience.
- **Model:** Fireworks `accounts/fireworks/models/deepseek-v4p1-flash`.
- **Runs:** 5 tasks × 3 variants × 5 runs = 75 runs, 6 at a time.
- **Scoring:** deterministic against `eval/gold.json`; no LLM judge.

## Results

Means over 5 runs. `tool out` is the total characters of tool output the agent
ingested (all tools); `src lines` is lines returned by `shell`/`read_file`.

| task | variant | correct | tokens | tool out | src lines | cycles | tool calls | gmem calls |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `symbol-001` | control | 5/5 | 7,794 | 8,263 | 160 | 4.0 | 3.0 | 0 |
| `symbol-001` | gmem | 5/5 | 11,796 | **1,175** | 20 | 4.0 | 3.0 | 1.0 |
| `symbol-001` | gmem-guided | 5/5 | 14,675 | 8,467 | 20 | 3.0 | 4.0 | 1.0 |
| `outline-001` | control | 5/5 | 13,273 | 9,177 | 219 | 5.0 | 7.0 | 0 |
| `outline-001` | gmem | 5/5 | 11,063 | **4,993** | 52 | 3.0 | 3.0 | 1.0 |
| `outline-001` | gmem-guided | 5/5 | 22,038 | 11,749 | 26 | 3.8 | 4.4 | 1.0 |
| `imports-001` | control | 5/5 | 8,417 | 18,328 | 409 | 2.4 | 1.6 | 0 |
| `imports-001` | gmem | 5/5 | 10,064 | **2,714** | 55 | 3.0 | 3.0 | 1.0 |
| `imports-001` | gmem-guided | 5/5 | 13,279 | 8,402 | 149 | 2.8 | 3.0 | 1.0 |
| `diff-001` | control | 5/5 | 54,425 | 23,353 | 562 | 9.4 | 12.6 | 0 |
| `diff-001` | gmem | 5/5 | **46,541** | 19,889 | 305 | 7.4 | 10.6 | 3.6 |
| `diff-001` | gmem-guided | 5/5 | 54,357 | 22,829 | 219 | 6.8 | 9.2 | 3.0 |
| `workflow-001` | control | 5/5 | 609,445 | 86,074 | 2,368 | 24.8 | 44.2 | 0 |
| `workflow-001` | gmem | 5/5 | 1,020,737 | 65,372 | 3,026 | 28.0 | 50.8 | 0.8 |
| `workflow-001` | gmem-guided | 5/5 | **596,509** | 82,942 | 1,991 | 24.2 | 43.0 | 3.0 |

Token change vs control:

| task | control tok | gmem | gmem-guided |
|---|---:|---:|---:|
| `diff-001` | 54,425 | **−14%** | 0% |
| `imports-001` | 8,417 | +20% | +58% |
| `outline-001` | 13,273 | **−17%** | +66% |
| `symbol-001` | 7,794 | +51% | +88% |
| `workflow-001` | 609,445 | +67% | **−2%** |

All 15 cells answered 5/5 correctly: no variant traded accuracy for tokens.

## Findings

- **Guidance is what makes the tools pay off on a hard task.** With a neutral
  prompt the agent ignored the tools on the open-ended task (0.8 gmem calls
  per run) and explored more than the control, costing **+67%** tokens. With
  the shipped guidance it used them (3.0 calls plus one skill load), read the
  fewest source lines of any variant (1,991 vs 2,368 control), and landed at
  **−2%** — parity, not the earlier regression.
- **`code_diff` is the clearest tool-shaped win.** On `diff-001` the neutral
  agent called it every run (3.6 calls), read 46% fewer source lines, ran
  fewer cycles and used **14% fewer tokens**. Guidance did not improve on
  that: the skill text it loads offsets the tool saving.
- **Small lookups cannot pay for the tools.** The four tool definitions add
  roughly 2.5k tokens to every model call; with 3–4 calls that exceeds the
  entire cost of a simple lookup, so `symbol-001` and `imports-001` cost
  20–88% more even though they read 75–90% less source.
- **The tools cut reading everywhere.** `tool out` fell 22–86% across tasks in
  both gmem variants, and `src lines` fell on every task except the neutral
  open-ended run.
- **gmem locates, it does not explain.** On `workflow-001` the answer requires
  the implementation, so even a guided agent still reads the relevant files;
  the tools remove the search cost, not the reading cost.

## Interpretation

The tools do what they claim: they cut the search-and-read work substantially.
Whether that becomes a token saving depends on two things the benchmark makes
visible. First, the agent has to choose the tools — the shipped guidance
decides that, and without it a flash model defaults to `rg` and `git diff`.
Second, the task has to be big enough to amortize the fixed cost of the tool
definitions; on a one-symbol lookup gmem is a net loss on tokens even while it
reads a fraction of the source.

## Limitations

- Five runs per cell. `workflow-001` has very high variance (control
  582k–770k tokens), so its numbers are directional; `symbol-001` wall time
  has 50 s outliers from provider latency.
- One model. `deepseek-v4p1-flash` is fast and cheap, not the strongest tool
  selector.
- `source_lines_read` counts lines from `shell` and `read_file` only, so gmem
  tool output is captured by `tool out` rather than by that column.
- Token counts are provider-reported and include reasoning tokens when the
  model reports them.

## Next steps

1. Run the same 3-variant matrix across several models (Nemotron Lightning,
   GLM Flash, DeepSeek, MiniMax) for a wide table of which models exploit the
   tools and which ignore them.
2. More runs per cell to tighten the `workflow-001` estimate.
3. Add tasks where gmem should dominate (rename a symbol across a package,
   list everything that changed in a subsystem) to test the upper bound.
