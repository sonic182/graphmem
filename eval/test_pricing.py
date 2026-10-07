"""Protect cached-input arithmetic and unknown-usage reporting."""

from collections.abc import Callable

import pytest

from compare import group_cost
from harness import DEFAULT_MODEL, Record
from pricing import cost_usd, mean_cost


@pytest.mark.parametrize(
    ("input_tokens", "output_tokens", "cache_tokens", "expected"),
    [
        (100, 20, 40, 0.00011),
        (0, 0, 0, 0.0),
        (None, 20, 40, None),
        (100, None, 40, None),
        (100, 20, None, None),
    ],
)
def test_cost_bills_cache_instead_of_fresh_input(
    input_tokens: int | None,
    output_tokens: int | None,
    cache_tokens: int | None,
    expected: float | None,
) -> None:
    rates = {"model": {"input": 1.0, "output": 2.0, "cache_read": 0.25}}
    assert cost_usd(rates, "model", input_tokens, output_tokens, cache_tokens) == expected


def test_unknown_run_does_not_make_group_cost_look_cheaper(
    record_factory: Callable[..., Record],
) -> None:
    records = [record_factory(), record_factory(input_tokens=None, total_tokens=None)]
    rates = {DEFAULT_MODEL: {"input": 1.0, "output": 2.0, "cache_read": 0.25}}
    assert mean_cost(rates, DEFAULT_MODEL, records) is None
    assert group_cost(rates, records) is None


def test_model_without_cache_rate_uses_input_rate() -> None:
    assert cost_usd({"model": {"input": 1.0, "output": 2.0}}, "model", 100, 20, 40) == 0.00014
    assert cost_usd({}, "model", 100, 20, 40) is None
