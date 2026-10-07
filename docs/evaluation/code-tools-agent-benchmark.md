# Code-tool agent benchmark: gmem vs a plain agent

Does a coding agent reach the same answer using less context when gmem's
code tools are available? The harness is in [`eval/`](../../eval/README.md).
Raw attempts and generated summaries are in `eval/results/`; the generated
[cross-model comparison](../../eval/results/comparison.md) reports median
tokens, mean estimated cost, accuracy, and resource settings.

## Current results

All three matrices contain 75 fresh attempts: five tasks × three variants ×
five runs. They use the corrected version-2 harness, temperature 0, eight
jobs, and a 16,384-token generation **ceiling**.

| model | correct | estimated matrix cost | additional resource caps |
|---|---:|---:|---|
| DeepSeek V4.1 Flash | 75/75 | $0.8823 | none; baseline predates caps |
| Nemotron Lightning 3.5 30B A3B | 53/75 | $0.4462 | none; baseline predates caps |
| GLM 5.3 Flash | 70/75 | $0.5124 | shared cost budget and per-task token caps |

DeepSeek and Nemotron have no invocation errors. GLM has four deliberate
per-run token-budget stops and one incorrect symbol answer; no other
invocation errors. All retained attempts have known provider usage. Failed
and stopped attempts remain in accuracy, token, and cost aggregates.

GLM's preliminary provider check cost $0.00099149 and is not included in its
75-attempt matrix. It was deducted from the agreed $1.15 allowance, leaving
$1.14900851 for the matrix. Combined estimated GLM spend was **$0.51336**;
the cost ceiling did not bind.

**Do not treat these as controlled model-quality rankings:** GLM ran with
additional token budgets while the two baselines did not. Compare variants
within each matrix, and retain the budget-stop counts alongside accuracy.
No failed samples were selectively replaced.

## Conclusions

1. **Savings are task- and model-dependent.** Neutral GLM reduces diff mean
   cost and tokens by about 55%, with 5/5 correct in both cells. Neutral
   DeepSeek instead uses 5% more mean diff tokens and costs 1% more.
2. **Cost is not total tokens.** Neutral DeepSeek costs 48% less on symbol
   lookup and 20% less on outline, despite using 30% and 46% more mean tokens.
   Cache hits and model-specific cached-input rates matter.
3. **Less shell/file reading is not always less context.** DeepSeek's gmem
   variants read fewer shell/file lines on four of five tasks, but both read
   more on workflow. MCP text, tool definitions, and guidance add context.
4. **Guidance is not a universal savings switch.** Guided GLM imports finishes
   only 2/5: three runs hit the token allowance. Guided Nemotron imports
   reaches 5/5 at 79% lower mean cost than its control.
5. **Cheaper incorrect answers are not a win.** Nemotron's diff task has only
   one fully correct answer across all 15 attempts, despite lower gmem costs.
6. **Protect the budget before requests, not after a whole agent finishes.**
   Shared reservations include all eight workers and retries. Historical
   medians, rather than runaway maxima, define new per-task allowances.

## Method

- **Corpus:** `openclaw/openclaw@8f5c33c3`; diff base `4de57f22`.
- **Control:** `shell` and `read_file`, retaining rg/sed/git. It is not
  handicapped; gmem tools are additive.
- **Neutral gmem:** control tools plus `find_symbol`, `code_outline`,
  `code_imports`, and `code_diff`; memory tools are filtered out. Same shared
  system prompt as control.
- **Guided gmem:** neutral gmem plus shipped plugin guidance and an on-demand
  skill loader. Each attempt starts a fresh agent.
- **Scoring:** deterministic against `eval/gold.json`, not an LLM judge.
  Only the terminal assistant response is scored. Unfinished tool turns and
  errored invocations cannot count correct. Repository-relative paths must
  match fully; basename-only, unrelated, absolute, and traversal paths fail.
  Import F1 normalizes matching surrounding quotes/backticks.
- **Usage:** accumulated provider input/output/cache counts are retained on
  failure. Unknown complete usage is `null`/`n/a`; `known_usage` retains any
  measured lower bounds. Cost uses `eval/pricing.json` list rates, not invoices.
- **Tool delivery:** all text, including MCP and skills, shares a 50,000-character
  per-result cap including the truncation marker. Character metrics are
  collected at delivery. Shell/file line counts exclude metadata and discarded
  lines, and do not include MCP source text.
- **Retries:** explicit LiteLLM/native throttling backoff, six attempts with
  waits of 4/8/16/32/64 seconds. Retrying a model call retains the existing
  agent and paid history; authentication/configuration errors are not retried.

## DeepSeek V4.1 Flash

Means per attempt; every cell is 5/5 correct. `tool chars` measures delivered
text; `src lines` includes only shell/file output, not MCP.

| task | variant | tokens | cost usd | tool chars | src lines | cycles |
|---|---|---:|---:|---:|---:|---:|
| `diff-001` | control | 36,719 | $0.0060 | 19,757 | 450 | 7.6 |
| `diff-001` | gmem | 38,680 | $0.0061 | 17,327 | 231 | 6.4 |
| `diff-001` | gmem-guided | 51,989 | $0.0059 | 23,315 | 205 | 6.6 |
| `imports-001` | control | 12,919 | $0.0007 | 22,508 | 500 | 2.8 |
| `imports-001` | gmem | 11,790 | $0.0008 | 2,749 | 54 | 3.4 |
| `imports-001` | gmem-guided | 14,863 | $0.0018 | 9,819 | 154 | 3.0 |
| `outline-001` | control | 9,159 | $0.0020 | 7,527 | 205 | 4.4 |
| `outline-001` | gmem | 13,377 | $0.0016 | 6,050 | 70 | 3.4 |
| `outline-001` | gmem-guided | 26,246 | $0.0016 | 12,332 | 45 | 4.4 |
| `symbol-001` | control | 9,083 | $0.0018 | 8,462 | 159 | 4.2 |
| `symbol-001` | gmem | 11,802 | $0.0009 | 1,175 | 20 | 4.0 |
| `symbol-001` | gmem-guided | 15,098 | $0.0012 | 10,455 | 64 | 3.0 |
| `workflow-001` | control | 967,219 | $0.0477 | 130,718 | 2,388 | 30.6 |
| `workflow-001` | gmem | 1,030,933 | $0.0481 | 162,708 | 3,227 | 28.4 |
| `workflow-001` | gmem-guided | 1,003,091 | $0.0503 | 156,989 | 2,576 | 27.0 |

Neutral gmem increases mean total tokens on four tasks and guided gmem on
all five. Neutral is cheaper on two tasks; guided on three. Workflow costs
rise about 1% and 6%, respectively. This matrix does not establish a general
workflow or structural-navigation win.

## Nemotron Lightning 3.5 30B A3B

Median tokens / mean USD / correct attempts. Do not confuse these token
medians with the DeepSeek means above.

| task | control | gmem | gmem-guided |
|---|---:|---:|---:|
| `diff-001` | 219,897 / $0.0193 / 0/5 | 130,536 / $0.0050 / 1/5 | 172,188 / $0.0043 / 0/5 |
| `imports-001` | 47,149 / $0.0020 / 5/5 | 25,402 / $0.0030 / 4/5 | 9,142 / $0.0004 / 5/5 |
| `outline-001` | 71,437 / $0.0019 / 4/5 | 99,003 / $0.0025 / 4/5 | 67,050 / $0.0029 / 5/5 |
| `symbol-001` | 49,643 / $0.0011 / 3/5 | 6,202 / $0.0001 / 5/5 | 7,493 / $0.0004 / 5/5 |
| `workflow-001` | 272,629 / $0.0058 / 5/5 | 797,354 / $0.0327 / 4/5 | 453,126 / $0.0077 / 3/5 |

Both gmem variants improve symbol lookup to 5/5 and reduce its cost. Guided
imports is also substantially cheaper with unchanged accuracy. Neither
variant improves workflow accuracy or cost. Long loops reached 132 cycles;
median tokens limit their influence on the displayed central tendency, but
mean costs retain their spend.

## GLM 5.3 Flash

Median tokens / mean USD / correct attempts. Every variant shares the same
per-task token allowance. Stopped attempts remain included.

| task | control | gmem | gmem-guided |
|---|---:|---:|---:|
| `diff-001` | 28,768 / $0.0046 / 5/5 | 17,296 / $0.0020 / 5/5 | 34,884 / $0.0036 / 5/5 |
| `imports-001` | 7,536 / $0.0007 / 5/5 | 8,469 / $0.0008 / 5/5 | 14,094 / $0.0015 / 2/5 |
| `outline-001` | 8,696 / $0.0010 / 5/5 | 13,321 / $0.0014 / 5/5 | 21,441 / $0.0021 / 5/5 |
| `symbol-001` | 7,842 / $0.0009 / 4/5 | 9,245 / $0.0006 / 5/5 | 13,462 / $0.0011 / 5/5 |
| `workflow-001` | 215,887 / $0.0236 / 5/5 | 427,251 / $0.0345 / 4/5 | 348,693 / $0.0240 / 5/5 |

Neutral diff is 55% cheaper by mean cost, with 40% fewer median tokens;
mean tokens fall 55%. Guided diff is 22% cheaper but has 21% more median
tokens. Neutral workflow has one budget stop and costs 46% more on average.
Three guided import attempts stop before another call would exhaust the
17,099-token allowance. The remaining non-correct attempt is a control symbol
answer scoring 0.75. Budget stops do not prove those answers would remain
incorrect with an unlimited allowance.

## Resource controls and limitations

[`eval/limits.json`](../../eval/limits.json) fixes new defaults at $1.15 per
invocation and the larger baseline model/task median +30% for run tokens.
Before each request, a shared ledger reserves conservative uncached-input
and maximum-output cost, including in-flight calls and retries. Unknown
usage consumes its reservation rather than becoming free. Missing prices or
input estimates block calls. Paid usage survives stops.

The per-run token guard uses projected input and clamps output to remaining
allowance; inaccurate estimation can overshoot tokens on the last call.
Unexpected cost-reservation overruns block further matrix calls. Controls
use list rates and conservative local bounds, not an exact provider invoice
or account-balance guarantee. Explicit overrides are documented in
[`eval/README.md`](../../eval/README.md#resource-budgets).

Other limitations:

- Five samples per cell and a single corpus; no evidence of universal savings
  or reduced variance. Means and medians can suggest different relative changes.
- GLM's resource policy differs from the uncapped baselines. A future controlled
  cross-model study would need fresh, consistently bounded matrices.
- Cache state and concurrent request order are uncontrolled.
- Fewer shell/file lines do not imply less total delivered text or billed context.
- LiteLLM warns that multi-turn `reasoningContent` is unsupported; its behavioral
  effect was not isolated.
- Records capture model, generation/concurrency/resource settings and assistant
  turns, but not a full binary/index/prompt-hash manifest.

Future work: larger structural tasks, more corpora, repeated bounded matrices,
and a complete reproducibility manifest. Local regression tests require no
provider credentials, and no eval GitHub Actions job is added.
