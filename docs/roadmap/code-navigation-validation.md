# Real-repository code navigation validation

## Snapshot

Checked on **2026-10-02**, using `./target/release/gmem` (`gmem 0.8.0`)
from the Graphmem checkout at `24f3c41`. The binary used index version 7.
This is a baseline for [the code navigation roadmap](todo-code-outline-and-indexing.md),
not a general performance guarantee.

The private Phoenix app's name, paths, commit ID, and query names are anonymized;
measurements and counts are unchanged.

| Repository | Working-tree location | Base commit | Source status |
| --- | --- | --- | --- |
| Phoenix app (private) | Not published | Not published | 23 status entries; results include local changes |
| Minibot (Python) | `.references/minibot/` | `15cee7b` | Clean |

Used a fresh scratch `GRAPHMEM_HOME` with embeddings disabled and
`GRAPHMEM_CODE=on`. Neither source repository nor the normal memory store was
modified. Comparators: ast-grep 0.45.3, ripgrep 15.2.0, and Python 3.14.7's AST.

## Indexing

Cold and warm indexing were each measured once with shell wall-clock timing.
Warm refresh immediately followed cold indexing. Source discovery used gmem's
normal Git-aware filtering and default file limit; neither index was truncated.

| Repository | Indexed files | Cold index | Warm refresh |
| --- | ---: | ---: | ---: |
| Phoenix app | 2,013 | 754 ms | 69 ms |
| Minibot | 341 | 161 ms | 62 ms |

The Phoenix app included 1,546 Elixir files and 166 HEEx files. Minibot included
324 Python files. These are useful real-project baselines, not a stress test
near the default 20,000-file limit.

## Definition accuracy

### Python

Parsed all 324 indexed Python files independently with `ast.parse`, collecting
classes, synchronous functions, and asynchronous functions, including nested
definitions. Compared `(path, name, end_line)` against indexed `class`,
`function`, and `method` symbols.

- **3,686 unique definitions matched.**
- **0 missing definitions, 0 extra definitions, 0 Python parse failures.**
- This check did not establish method-versus-function classification accuracy,
  parent accuracy, start-line accuracy, or import completeness.

### Elixir

Scanned the Phoenix app's `lib/` with ast-grep for calls whose target was one of
`def`, `defp`, `defdelegate`, `defmacro`, `defmacrop`, `defguard`, `defguardp`,
`defmodule`, `defprotocol`, or `defimpl`.

- **15,626 standard definition clauses matched indexed symbols.**
- **0 missing clauses within that supported-form inventory.**
- Function/macro/guard matching checked the name, kind, and containing range;
  module matching required exact ranges. Multiple clauses can map to one merged
  symbol, so this is a clause count, not a unique-symbol count.
- This was not a complete false-positive audit of Elixir symbols and did not
  validate arity, imports, or arbitrary macro-generated definitions.

### HEEx spot check

Compared one HEEx template in the Phoenix app with its outline: all 13
component/slot opening locations matched, with form/field/slot nesting
represented correctly. This is one template, not a whole-repository HEEx audit.

### Coverage reporting

The Phoenix app reported partial syntax coverage for 10 Elixir files, 8 HEEx files,
and 104 SCSS files; its single EEx file reported directive-only coverage.
Minibot reported partial syntax coverage for 2 HTML files. These warnings were
not treated as missing definitions or proof that the source itself is invalid.
All indexed Python files reported complete parser coverage.

## Common navigation queries

Each warm CLI lookup below was run five times; timings are medians including
process startup. Compared `gmem code find QUERY` with `rg -n -w -- QUERY .`.
The queries were a hand-picked navigation sample, not a random workload.

| Repository | Query | gmem median | rg median | gmem lines | rg lines |
| --- | --- | ---: | ---: | ---: | ---: |
| Phoenix app | `Context` | 76.9 ms | 17.5 ms | 14 | 232 |
| Phoenix app | `check_service` | 81.4 ms | 19.3 ms | 1 | 11 |
| Phoenix app | `auth_error` | 71.0 ms | 11.9 ms | 1 | 17 |
| Phoenix app | `StorageClient` | 92.6 ms | 19.3 ms | 1 | 35 |
| Phoenix app | `cached_status` | 106.1 ms | 16.9 ms | **0 (miss)** | 2 |
| Minibot | `AppContainer` | 61.4 ms | 10.7 ms | 1 | 128 |
| Minibot | `Settings` | 70.2 ms | 10.4 ms | 3 | 174 |
| Minibot | `EventBus` | 58.9 ms | 11.5 ms | 1 | 153 |
| Minibot | `from_dict` | 56.8 ms | 7.5 ms | 1 | 32 |
| Minibot | `ProviderHTTPError` | 75.8 ms | 10.8 ms | 1 | 15 |

For the nine successful queries, total output was **24 lines / 3,505 bytes**
from gmem versus **797 lines / 78,749 bytes** from rg. gmem included definition
ranges and parent names where applicable.

This is not an equivalent-output speed comparison: rg includes usages and prose,
whereas gmem returns definitions with exact/last-segment/prefix matching. A
syntax-filtered ast-grep query can also remove usage noise. Independent ast-grep
checks confirmed the exact ranges of `EventBus` (60–121) and the Phoenix app's
error helper (anonymized as `auth_error`, 483–496).

**Conclusion:** the demonstrated benefit is smaller, structured navigation
output, not faster raw search. Actual agent tool-call savings and MCP latency
were not measured.

## Actionable gap: Memoize definitions

The ast-grep scan found **eight `defmemo`/`defmemop` definitions** outside the
standard definition inventory. gmem does not recognize these forms.

A confirmed navigation miss is a private cached helper (anonymized as
`cached_status/1`), declared with `defmemop` at line 147. Finding it by name
returned nothing, although the file reported complete parser coverage.

Complete parser coverage does not imply support for every framework's
function-defining macros. Keep that distinction explicit when interpreting
navigation results.

## Follow-ups

1. Add `defmemo`/`defmemop` extraction with a CLI regression fixture before
   expanding into unrelated languages.
2. Repeat representative agent navigation tasks to measure actual tool calls;
   compare against syntax-aware ast-grep, not only broad rg output.
3. Investigate partial-coverage files when a real navigation task misses a symbol;
   do not count every parser warning as an extraction failure.

## Repeating the baseline

Use a new scratch home so cold indexing does not reuse an existing cache:

```bash
BIN="$PWD/target/release/gmem"
HOME_DIR=$(mktemp -d)
printf '[embedding]\nenabled = false\n' > "$HOME_DIR/config.toml"
export GRAPHMEM_HOME="$HOME_DIR" GRAPHMEM_CODE=on

PHOENIX_REPO="/path/to/your/phoenix-checkout"
PHOENIX_QUERY="your_symbol"
TIMEFORMAT='wall=%3R seconds'
for repo in "$PHOENIX_REPO" "$PWD/.references/minibot"; do
  time "$BIN" code index "$repo"
  time "$BIN" code index "$repo"
done

(cd "$PHOENIX_REPO" && "$BIN" code find "$PHOENIX_QUERY")
(cd .references/minibot && "$BIN" code find EventBus)
```

This repeats indexing and representative lookups, not the full AST accuracy
comparison. Results depend on the working trees, hardware, filesystem cache,
and built binary; remeasure rather than treating these timings as thresholds.
