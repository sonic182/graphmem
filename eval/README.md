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
├── score.py            # deterministic scorer (no LLM judge)
├── pricing.py          # per-model token pricing and cost
├── pricing.json        # rates (USD per 1M tokens) from models.dev
├── compare.py          # merge per-model runs.jsonl files into one wide table
├── tasks.json          # task prompts
├── gold.json           # gold answers and the workflow rubric
├── prompts/system.txt  # system prompt, identical for both variants
├── setup_corpus.sh     # fetch the pinned OpenClaw checkout and build the gmem index
├── pyproject.toml      # ruff (PEP 8) config
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

# smoke test: one task, both variants, one run
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --task-filter symbol-001 --runs 1

# full matrix: 5 tasks x 2 variants x 5 runs = 50 runs
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --runs 5
```

Every finished run is appended to the output file immediately, so a crash or a
kill never loses completed work; `rescore.py` regenerates the summary from the
partial file.

Runs execute concurrently (`--jobs`, default 4). Use `--jobs 1` to keep the
strict interleaved order, or a lower value for models whose contexts grow
large enough to pressure memory. Other flags: `--dry-run`, `--variants control,gmem`,
`--task-filter a,b`, `--model`, `--temperature`, `--max-tokens`,
`--skills-dir`, `--guidance`.

To re-score stored answers after changing `gold.json` or `score.py` without
re-running the agents: `uv run rescore.py`.

To run another model, write to its own directory and then build the wide
table:

```bash
uv run harness.py --repo ~/.cache/gmem-eval/openclaw-bench \
    --gmem ../target/release/gmem --runs 5 --jobs 6 \
    --model fireworks_ai/accounts/fireworks/models/nemotron-lightning-3p5-30b-a3b \
    --out results/nemotron/runs.jsonl --summary results/nemotron/summary.md
uv run compare.py          # reads every results/**/runs.jsonl
```

## Tasks

One task per code tool plus one end-to-end navigation task:

| id | type | what it asks |
|---|---|---|
| `symbol-001` | `find_symbol` | locate `runLlamaCppSetup` (file, kind, line range) |
| `outline-001` | `code_outline` | exported top-level functions/classes of `llama-server-install.ts` |
| `imports-001` | `code_imports` | module specifiers imported by `setup.ts` |
| `diff-001` | `code_diff` | source symbols added/modified between the two pinned revisions |
| `workflow-001` | end-to-end | where llama-server host compatibility is decided, the restrictions, the user-facing error, and the tests |

Scoring is deterministic against `gold.json`: exact match for `symbol-001`,
recall over required symbols for `outline-001` and `diff-001`, F1 for
`imports-001`, and a citation/fact rubric for `workflow-001`.

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
- `tool_output_chars` — characters of tool output the agent ingested, summed
  from the message history so it covers shell, `read_file` and gmem alike.
- `source_lines_read` — lines returned by `shell` and `read_file`.
- `score`, `correct`, `details`, `answer`, `error`.

Every tool result is truncated (head + tail, 50 000 characters) identically for
both variants, the way a real coding harness truncates tool output.

## Notes and limitations

- The system prompt is identical for both variants and does not mention gmem.
  The gmem variant discovers the tools from their MCP descriptions, which is
  the conservative choice: the shipped Graphmem plugin also injects guidance.
- Token counts are provider-reported and include reasoning tokens when the
  model reports them.
- The corpus is a single repository; treat the numbers as a signal, not a
  universal constant.
