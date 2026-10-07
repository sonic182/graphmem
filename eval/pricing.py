"""Per-model token pricing in USD per 1M tokens.

Rates come from the models.dev catalog (``https://models.dev/api.json``,
provider ``fireworks-ai``) and are list prices as published there. Cached
input is far cheaper than fresh input (for example deepseek-v4p1-flash is
$0.006 vs $0.30), which is why total tokens is a poor cost proxy.

The provider reports ``cacheReadInputTokens`` as a subset of ``inputTokens``
(``totalTokens = inputTokens + outputTokens``), so cached tokens are billed
instead of, not on top of, the fresh input count.
"""

from __future__ import annotations

import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PER_MILLION = 1_000_000


def load_pricing(path: Path | None = None) -> dict[str, dict[str, float]]:
    return json.loads((path or HERE / "pricing.json").read_text())


def cost_usd(
    pricing: dict[str, dict[str, float]],
    model: str,
    input_tokens: int | None,
    output_tokens: int | None,
    cache_read_tokens: int | None,
) -> float | None:
    """Cost of one run, or ``None`` when the model has no pricing entry."""
    rates = pricing.get(model)
    if rates is None or input_tokens is None or output_tokens is None or cache_read_tokens is None:
        return None
    fresh_input = max(0, input_tokens - cache_read_tokens)
    cache_rate = rates.get("cache_read", rates["input"])
    return (
        fresh_input * rates["input"]
        + cache_read_tokens * cache_rate
        + output_tokens * rates["output"]
    ) / PER_MILLION


def mean_cost(
    pricing: dict[str, dict[str, float]],
    model: str,
    records: list,
) -> float | None:
    """Return the group mean, or ``None`` if any run's cost is unknown."""
    costs = [
        cost_usd(
            pricing, model, record.input_tokens, record.output_tokens, record.cache_read_tokens
        )
        for record in records
    ]
    if not costs or any(cost is None for cost in costs):
        return None
    return sum(costs) / len(costs)
