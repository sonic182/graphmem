# gmem code-tool agent benchmark

A minimal, reproducible benchmark that asks one question:

> When a coding agent has the gmem code tools (`find_symbol`, `code_outline`,
> `code_imports`, `code_diff`) available **in addition to** `rg`, `sed`, `git`
> and the rest of its shell workflow, does it reach the same answer using less
> context, fewer tool calls and fewer source lines read?

The control agent is not handicapped: both variants share the same shell and
file tools. gmem is purely additive, so the comparison measures the real value
of the tools, not the effect of taking tools away.

## Variants

| variant | tools | prompt |
|---|---|---|
| `control` | `shell`, `read_file` | shared system prompt |
| `gmem` | `shell`, `read_file`, `find_symbol`, `code_outline`, `code_imports`, `code_diff` (gmem MCP, memory tools filtered out) | shared system prompt |
| `gmem-guided` | `gmem` tools plus a `skill` loader | shared system prompt **plus** the shipped plugin guidance, and the skills available on demand |

`gmem-guided` mirrors what the Graphmem plugin actually does at session
start: it injects the guidance text (`prompts/guidance.txt`, copied verbatim
from the Pi extension) and exposes `plugin/skills/*` through a `skill` tool so
the agent can pull `graphmem-code-analysis` or `graphmem-mcp-for-dev` when it
wants them. `gmem` with no guidance answers a different question: does the tool
pay for itself when the agent is never told about it?

All variants use the same model, temperature, checkout and limits, and each
run starts from a fresh `Agent` so no context carries over.

## Layout

```
eval/
├── harness.py          # entrypoint: runs the matrix, writes runs.jsonl + summary.md
├── tools.py            # shell + read_file with per-run metrics and output truncation
├── budget.py           # shared pre-call cost reservations and per-run token guards
├── limits.json         # baseline +30% default resource ceilings
├── retry.py            # bounded LiteLLM/native throttling backoff
├── score.py            # deterministic scorer (no LLM judge)
├── pricing.py          # per-model token pricing and cost
├── pricing.json        # rates (USD per 1M tokens) from models.dev
├── compare.py          # merge per-model runs.jsonl files into one wide table
├── tasks.json          # task prompts
├── gold.json           # gold answers and the workflow rubric
├── prompts/system.txt  # system prompt, identical for both variants
├── setup_corpus.sh     # fetch the pinned OpenClaw checkout and build the gmem index
├── pyproject.toml      # runtime/dev dependencies, pytest and Ruff config
├── uv.lock             # pinned runtime and local-test dependencies
└── results/
    ├── runs.jsonl      # one JSON object per run
    └── summary.md      # generated tables
```

## Setup

Requires [uv](https://docs.astral.sh/uv/) and a Fireworks API key in
`FIREWORKS_API_KEY`.

```bash
# pinned OpenClaw checkout + gmem index (~750 MB, ~40 s to index)
eval/setup_corpus.sh
```

The corpus is pinned to `openclaw/openclaw@8f5c33c3` (parent `4de57f22`), the
llama.cpp host-compatibility fix, so the benchmark can be repeated exactly.

## Run

```bash
cd eval

# smoke test: one task, all three variants, one run
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --task-filter symbol-001 --runs 1

# latest DeepSeek matrix: 5 tasks x 3 variants x 5 runs = 75 runs
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --runs 5 --jobs 8 --max-tokens 16384 --temperature 0
```

Every finished run is appended to the output file immediately, so a crash or a
kill never loses completed work; `rescore.py` regenerates the summary from the
partial file.

Runs execute concurrently (`--jobs`, default 4). Use `--jobs 1` to keep the
strict interleaved order, or a lower value for models whose contexts grow
large enough to pressure memory. Other flags: `--dry-run`, `--variants control,gmem`,
`--task-filter a,b`, `--model`, `--temperature`, `--max-tokens`,
`--skills-dir`, `--guidance`.

The harness defaults to an 8192-token generation cap. All three stored matrices
use version 2, `--max-tokens 16384 --temperature 0 --jobs 8`.
A generation cap alone does not bound total run tokens or agent cycles;
GLM and new invocations additionally enforce the resource budgets below;
DeepSeek and Nemotron are the uncapped calibration baselines. The output
file is replaced at startup; do not selectively replace failed samples.

Each agent uses bounded exponential backoff for LiteLLM `RateLimitError` and
native Strands throttling: at most six attempts per model call, with waits of
4, 8, 16, 32, and 64 seconds (maximum configured delay: 240 seconds). A
successful call resets the backoff. Authentication/configuration errors and
token-limit stops are not retried. Retries stay within the same agent run,
so earlier paid cycles and tool results are retained; wall time includes waits.

## Resource budgets

Every new invocation uses `limits.json` by default:

- **$1.15 estimated cost for the entire matrix**, not per run or per worker.
  This is the largest valid baseline matrix cost ($0.8823) plus 30%, rounded
  up to cents. The budget resets for each invocation, including reruns.
- **Cumulative input + output tokens per run**, selected by task:

  | task | token ceiling |
  |---|---:|
  | `symbol-001` | 15,327 |
  | `outline-001` | 92,869 |
  | `imports-001` | 17,099 |
  | `diff-001` | 223,845 |
  | `workflow-001` | 1,339,194 |

  Each ceiling is the larger of the two baseline models' per-task medians
  across all variants, plus 30%, rounded up. Medians avoid letting runaway
  outliers define the allowance.

Before **every model attempt**, including retries, a shared, thread-safe
ledger reserves conservative fresh-input cost plus maximum output cost.
Input cost uses a serialized-byte bound with a 4,096-token framing allowance,
not the SDK's less conservative tokenizer estimate; cache hits are never
assumed. Eight jobs share the same ledger, including in-flight reservations.
Calls wait for pending reservations to settle, or stop if unaffordable.
Delivered usage releases unused funds. Known throttling/authentication/bad
request rejections release their reservation; other failures without usage
charge the full reservation to the ledger, not to the invoice estimate.

The output generation cap is also lowered to fit remaining per-run tokens
minus the SDK's projected input. Input estimation can be inaccurate: the
actual token count can overshoot on the last call, which is retained and
marked as a limit error. A missing estimate or unknown model price prevents
calls. If provider usage exceeds its conservative cost reservation, further
calls are blocked across the matrix. These are **estimated-cost controls**,
not a guarantee about the provider's invoice or account balance.

Use `--max-cost-usd`, `--max-run-tokens` (one ceiling for every selected task),
or `--limits PATH` for explicit overrides. Limits must be positive; there
is no default unbounded mode. `--dry-run` prints the selected limits without
making provider calls. Limit stops keep paid usage, are errors, and cannot
count correct. New records include `max_matrix_cost_usd`, `max_run_tokens`,
`budget_cost_usd` (ledger debit, potentially conservative), and
`budget_stop_reason`. Missing or unusable provider usage cannot enable free
loops: the run stops, complete token totals/cost become `n/a`, and
`known_usage` preserves any measured lower bounds from earlier paid calls.
Earlier stored matrices predate these resource caps;
do not describe them as bounded runs or compare future capped outcomes as
though the limits were identical.

## Rescore and compare

To re-score stored answers after changing `gold.json` or `score.py` without
re-running the agents: `uv run rescore.py`.

To run another model, write to its own directory and then build the wide
table:

```bash
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --runs 5 --jobs 8 --max-tokens 16384 --temperature 0 \
    --model fireworks_ai/accounts/fireworks/models/nemotron-lightning-3p5-30b-a3b \
    --out results/nemotron-lightning-3p5-30b-a3b/runs.jsonl \
    --summary results/nemotron-lightning-3p5-30b-a3b/summary.md
uv run compare.py          # reads every results/**/runs.jsonl
```

## Local tests

Tests run locally only; there is no eval GitHub Actions job. They replay
provider stream events through the real Strands agent loop and stub only the
external model/MCP transports. No API key, gmem binary, or corpus download
is needed. The `uv.lock` file pins runtime and test dependencies.

```bash
cd eval
uv sync --locked --group dev
uv run --locked --group dev pytest -q
uv run --locked --group dev ruff check .
uv run --locked --group dev ruff format --check .
```

Coverage includes scoring/path regressions, terminal-answer selection,
usage after token-limit failures, MCP output caps, delivered source-line
counts, pricing with unknown usage, parallel harness output, bounded backoff
(recovery, exhaustion, reset, and non-retryable errors), and the
`rescore.py`/`compare.py` CLIs.

## Tasks

One task per code tool plus one end-to-end navigation task:

| id | type | what it asks |
|---|---|---|
| `symbol-001` | `find_symbol` | locate `runLlamaCppSetup` (file, kind, line range) |
| `outline-001` | `code_outline` | exported top-level functions/classes of `llama-server-install.ts` |
| `imports-001` | `code_imports` | module specifiers imported by `setup.ts` |
| `diff-001` | `code_diff` | source symbols added/modified between the two pinned revisions |
| `workflow-001` | end-to-end | where llama-server host compatibility is decided, the restrictions, the user-facing error, and the tests |

Scoring uses only the terminal assistant response. Intermediate answers and
unfinished tool-use turns cannot earn credit; errored invocations are not
counted as correct. Repository-relative file paths must match after separator
and `./` normalization; empty paths, basenames, other directories, absolute
paths, and parent traversals are rejected.

Scoring is deterministic against `gold.json`: exact match for `symbol-001`,
recall over required symbols for `outline-001` and `diff-001`, F1 for
`imports-001`, and a citation/fact rubric for `workflow-001`. Import scoring
normalizes matching surrounding single/double quotes and backticks inside
JSON strings; wrong or missing specifiers still reduce F1.

## Metrics

Recorded per run in `results/runs.jsonl`:

- `input_tokens`, `output_tokens`, `total_tokens`, `cache_read_tokens` — from
  the model provider, accumulated over every agent cycle. `cache_read_tokens`
  is a subset of `input_tokens`, so `total_tokens = input + output`.
- `cost_usd` — derived per run from `pricing.json`, because total tokens is a
  poor cost proxy: cached input is far cheaper than fresh input (for example
  deepseek-v4p1-flash is $0.006 vs $0.30 per 1M). Rates come from the
  models.dev catalog (`https://models.dev/api.json`, provider `fireworks-ai`).
- `cycles`, `wall_ms` — agent-loop cycles and wall time.
- `tool_calls`, `tool_calls_by_name` — every tool call, including the four
  gmem tools.
- `tool_output_chars` — characters delivered to the agent by shell,
  `read_file`, MCP, and skill tools, measured before its next model turn.
- `source_lines_read` — lines returned by `shell` and `read_file`.
- `score`, `correct`, `details`, `answer`, `error` — `answer` is the terminal
  response, while `assistant_messages` preserves assistant text/tool-use turns
  for diagnostics and rescoring.
- `benchmark_version`, `max_tokens`, `temperature`, `jobs` — records capture
  the corrected harness version and shared generation/concurrency settings.

Usage is taken from the agent's accumulated metrics even when generation
raises. If no usage is available (for example, model construction fails),
token fields are `null` and cost/token cells are `n/a`, not zero. A group with
unknown usage cannot claim a complete mean cost. Tool-output characters are
recorded when results are delivered, rather than reconstructed from possibly
trimmed final history. File line counts include only retained numbered lines;
shell output counts exclude exit/truncation metadata.

All delivered tool text, including MCP results, is capped at 50 000 characters
by the same post-tool hook before the model's next turn. Head/tail truncation
includes its marker within that budget; multiple MCP text blocks share one
budget. Non-text content and success/error status are preserved.

## Notes and limitations

- Three fresh version-2 matrices are stored: DeepSeek is 75/75, Nemotron
  53/75, and GLM 70/75. The first two have no invocation errors. GLM has four
  token-budget stops and one incorrect symbol answer, with $0.5124 matrix
  cost ($0.51336 including its separate provider check). The baselines lack
  GLM's additional resource caps, so cross-model quality comparisons are not
  controlled for that policy. `rescore.py` requires preserved assistant turns
  and rejects inputs without them rather than guessing.

- `control` and neutral `gmem` share the same system prompt, which does not
  mention gmem. `gmem-guided` additionally receives the shipped plugin guidance
  and the skill loader.
- Token counts are provider-reported and include reasoning tokens when the
  model reports them.
- The corpus is a single repository; treat the numbers as a signal, not a
  universal constant.
