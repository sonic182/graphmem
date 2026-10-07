# Code-tool agent benchmark: gmem vs a plain agent

Does a coding agent reach the same answer using less context when the gmem
code tools are available? This is the first run of the harness in
[`eval/`](../../eval/README.md). Raw data is in `eval/results/`, and the wide
cross-model table in `eval/results/comparison.md`.

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
- **Models:** Fireworks `deepseek-v4p1-flash` and
  `nemotron-lightning-3p5-30b-a3b`.
- **Runs:** 5 tasks × 3 variants × 5 runs = 75 runs per model.
- **Scoring:** deterministic against `eval/gold.json`; no LLM judge.
- **Cost:** derived per run from `eval/pricing.json` (rates from models.dev).

## DeepSeek V4.1 Flash

Mean per run. All 15 cells answered 5/5 correctly.

| task | variant | tokens | cost usd | tool out | src lines | cycles |
|---|---|---:|---:|---:|---:|---:|
| `symbol-001` | control | 7,794 | $0.0009 | 8,263 | 160 | 4.0 |
| `symbol-001` | gmem | 11,796 | **$0.0005** | **1,175** | 20 | 4.0 |
| `symbol-001` | gmem-guided | 14,675 | $0.0009 | 8,467 | 20 | 3.0 |
| `outline-001` | control | 13,273 | $0.0023 | 9,177 | 219 | 5.0 |
| `outline-001` | gmem | 11,063 | **$0.0008** | **4,993** | 52 | 3.0 |
| `outline-001` | gmem-guided | 22,038 | $0.0018 | 11,749 | 26 | 3.8 |
| `imports-001` | control | 8,417 | $0.0008 | 18,328 | 409 | 2.4 |
| `imports-001` | gmem | 10,064 | **$0.0006** | **2,714** | 55 | 3.0 |
| `imports-001` | gmem-guided | 13,279 | $0.0018 | 8,402 | 149 | 2.8 |
| `diff-001` | control | 54,425 | $0.0079 | 23,353 | 562 | 9.4 |
| `diff-001` | gmem | **46,541** | $0.0064 | 19,889 | 305 | 7.4 |
| `diff-001` | gmem-guided | 54,357 | **$0.0057** | 22,829 | 219 | 6.8 |
| `workflow-001` | control | 609,445 | $0.0411 | 86,074 | 2,368 | 24.8 |
| `workflow-001` | gmem | 1,020,737 | $0.0555 | 65,372 | 3,026 | 28.0 |
| `workflow-001` | gmem-guided | **596,509** | **$0.0274** | 82,942 | 1,991 | 24.2 |

Change vs control:

| task | gmem tokens | gmem cost | guided tokens | guided cost |
|---|---:|---:|---:|---:|
| `diff-001` | **−14%** | **−19%** | 0% | **−29%** |
| `imports-001` | +20% | **−33%** | +58% | +113% |
| `outline-001` | **−17%** | **−63%** | +66% | **−20%** |
| `symbol-001` | +51% | **−39%** | +88% | +1% |
| `workflow-001` | +67% | +35% | **−2%** | **−33%** |

### What it says

- **Cost and tokens disagree, and cost is the number that matters.** Cached
  input is ~90% of the input count and priced at 2–8% of fresh input
  (deepseek: $0.006 vs $0.30 per 1M). gmem returns short symbol lists instead
  of file bodies, so it trades expensive output and fresh input for cheap
  cached input. `imports-001` is the clearest case: gmem uses **20% more
  tokens but costs 33% less**.
- **By cost, gmem wins on four of five tasks**, and guided wins on three
  (diff, outline, workflow). The token metric alone understates it.
- **Guidance is decisive on the hard task.** Neutral `gmem` ignored the tools
  on `workflow-001` (0.8 gmem calls/run), explored more than control and cost
  **+35%**. Guided used them (3.0 calls + a skill load), read the fewest source
  lines of any variant and cost **−33%**.
- **`code_diff` is the clearest tool-shaped win**: −14% tokens, −19% cost,
  −46% source lines, fewer cycles, tool used every run.
- **Small lookups still lose on tokens** (the four tool definitions cost
  ~2.5k tokens per model call) **but win on cost** because the model emits
  fewer output tokens and the rest is cached.

## Cross-model: Nemotron Lightning 3.5 30B A3B

Median tokens / mean cost per run, with accuracy. Nemotron is a much weaker
tool selector and a much weaker agent.

| task | control | gmem | gmem-guided |
|---|---:|---:|---:|
| `diff-001` | 386,450 / $0.0102 (1/5) | 210,226 / $0.0062 (1/5) | 203,658 / **$0.0043** (2/5) |
| `imports-001` | 27,487 / $0.0012 (4/5) | 56,834 / $0.0016 (3/5) | 8,326 / **$0.0003** (5/5) |
| `outline-001` | 84,876 / $0.0023 (5/5) | 101,900 / $0.0026 (3/5) | 121,319 / $0.0089 (4/5) |
| `symbol-001` | 14,389 / $0.0003 (2/5) | 6,202 / **$0.0001** (5/5) | 7,572 / $0.0002 (5/5) |
| `workflow-001` | 366,803 / $0.0082 (5/5) | 606,263 / $0.0268 (3/5) | 505,470 / $0.0081 (5/5) |

### What it says

- **Model quality dominates everything.** Nemotron's *control* is far weaker
  and far more expensive than deepseek's: on `diff-001` it needs a median
  386k tokens and still only gets 1/5 right, against deepseek's 53k and 5/5.
- **The tools can rescue a weak model.** On `symbol-001` nemotron's control
  scores 2/5 and burns 14k tokens; both gmem variants score 5/5 on ~7k tokens.
  Guided `imports-001` is 5/5 at $0.0003 against control's 4/5 at $0.0012.
- **The tools can also hurt a weak model.** Neutral `gmem` drops accuracy on
  `outline-001` (3/5), `workflow-001` (3/5) and `imports-001` (3/5), and guided
  `outline-001` is 4/5 at 4× the cost.
- **Weak models burn money with more tools available.** One nemotron guided
  `outline-001` run looped for 107 cycles and 102 shell calls, spending 2.7M
  tokens — enough to distort a mean, which is why the table above reports
  medians.

## Limitations

- Five runs per cell. Runaway loops and high variance mean single cells are
  directional, not precise; the cross-model table uses medians for that reason.
- Two models, both "flash"-class. A stronger model may exploit the tools
  differently.
- `source_lines_read` counts lines from `shell` and `read_file` only, so gmem
  tool output is captured by `tool out` rather than by that column.
- `cost_usd` uses published list prices and assumes cached input is billed at
  the models.dev `cache_read` rate.
- Token counts are provider-reported and include reasoning tokens when the
  model reports them.

## Next steps

1. Add GLM 5.3 Flash and MiniMax M3 to the cross-model table.
2. Add a per-run step or wall-clock cap so a runaway loop cannot dominate a
   cell, and re-run the weak model.
3. Add tasks where gmem should dominate (rename a symbol across a package,
   list everything that changed in a subsystem) to test the upper bound.
