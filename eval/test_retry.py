"""Exercise bounded backoff through real Strands model invocations."""

import asyncio
from argparse import Namespace
from unittest.mock import AsyncMock

import pytest
from litellm import AuthenticationError, RateLimitError
from strands.types.exceptions import ModelThrottledException

import harness
from budget import MatrixBudget
from conftest import ScriptedModel, model_turn
from pricing import load_pricing


def rate_limit() -> RateLimitError:
    """Construct the same exception type emitted by the Fireworks adapter."""
    return RateLimitError(message="rate limit exceeded", llm_provider="fireworks_ai", model="test")


@pytest.fixture
def backoff_clock(monkeypatch: pytest.MonkeyPatch) -> AsyncMock:
    """Observe backoff delays without waiting or making provider requests."""
    clock = AsyncMock(spec=asyncio.sleep)
    monkeypatch.setattr(asyncio, "sleep", clock)
    return clock


@pytest.mark.parametrize("native", [False, True])
def test_rate_limit_backoff_recovers_with_only_successful_usage(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, backoff_clock: AsyncMock, native: bool
) -> None:
    eval_args.matrix_budget = MatrixBudget(0.015, load_pricing()[eval_args.model])
    error = ModelThrottledException("throttled") if native else rate_limit()
    model = ScriptedModel([error, error, model_turn('["real"]')])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "control",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert record.correct and record.error is None
    assert len(model.requests) == 3
    assert [call.args[0] for call in backoff_clock.await_args_list] == [4, 8]
    assert record.input_tokens == 100
    assert record.output_tokens == 20
    assert record.total_tokens == 120
    assert record.budget_cost_usd == pytest.approx(0.00004224)
    assert eval_args.matrix_budget.reserved == 0


def test_rate_limit_exhaustion_is_bounded_and_preserves_previous_paid_cycles(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, backoff_clock: AsyncMock
) -> None:
    (eval_args.repo / "source.txt").write_text("source\n")
    model = ScriptedModel(
        [
            model_turn(
                "Searching.",
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            ),
            *(rate_limit() for _ in range(6)),
        ]
    )
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "control",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert "RateLimitError" in record.error
    assert not record.correct
    assert len(model.requests) == 7  # One paid turn, then six attempts of the next call.
    assert [call.args[0] for call in backoff_clock.await_args_list] == [4, 8, 16, 32, 64]
    assert record.input_tokens == 100
    assert record.output_tokens == 20
    assert record.total_tokens == 120
    assert record.tool_calls == 1
    assert record.source_lines_read == 1


def test_success_resets_backoff_for_the_next_model_call(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, backoff_clock: AsyncMock
) -> None:
    (eval_args.repo / "source.txt").write_text("source\n")
    model = ScriptedModel(
        [
            rate_limit(),
            model_turn(
                "Searching.",
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            ),
            rate_limit(),
            model_turn('["real"]'),
        ]
    )
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "control",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert record.correct and record.error is None
    assert [call.args[0] for call in backoff_clock.await_args_list] == [4, 4]
    assert record.total_tokens == 240
    assert record.tool_calls == 1


@pytest.mark.parametrize(
    "error",
    [
        RuntimeError("invalid configuration"),
        AuthenticationError(message="invalid key", llm_provider="fireworks_ai", model="test"),
    ],
)
def test_non_retryable_errors_fail_without_backoff(
    monkeypatch: pytest.MonkeyPatch,
    eval_args: Namespace,
    backoff_clock: AsyncMock,
    error: Exception,
) -> None:
    model = ScriptedModel([error])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "control",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert record.error is not None and not record.correct
    assert len(model.requests) == 1
    assert backoff_clock.await_count == 0
