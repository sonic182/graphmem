"""Exercise budget enforcement at the real agent and CLI boundaries."""

import asyncio
import sys
import threading
from argparse import Namespace
from collections.abc import AsyncIterator
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

import pytest

import harness
from budget import MatrixBudget
from conftest import ScriptedModel, model_turn
from pricing import load_pricing

TASK = {"id": "imports-001", "prompt": "Find imports."}
GOLD = {"imports-001": {"specifiers": ["real"]}}


def test_token_limit_stops_a_loop_before_another_paid_request(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    (eval_args.repo / "source.txt").write_text("source\n")
    eval_args.run_token_limits = {"imports-001": 360}
    model = ScriptedModel(
        [
            model_turn(
                "Searching.",
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            )
            for _ in range(10)
        ]
    )
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert not record.correct
    assert "run token budget" in record.budget_stop_reason
    assert len(model.requests) == 2
    assert record.total_tokens == 240
    assert record.input_tokens == 200
    assert record.tool_calls == 2
    assert record.max_run_tokens == 360
    assert record.budget_cost_usd == pytest.approx(0.00008448)


def test_unaffordable_first_request_never_reaches_the_provider(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    eval_args.matrix_budget = MatrixBudget(0.000001, load_pricing()[eval_args.model])
    model = ScriptedModel([model_turn('["real"]')])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert model.requests == []
    assert "matrix cost budget" in record.budget_stop_reason
    assert not record.correct
    assert record.total_tokens == 0
    assert record.budget_cost_usd == 0


def test_eight_jobs_share_one_budget_including_in_flight_reservations(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    barrier = threading.Barrier(8)
    models: list[ScriptedModel] = []
    eval_args.jobs = 8
    eval_args.max_tokens = 20
    eval_args.run_token_limits = {"imports-001": 100_000}
    eval_args.matrix_budget = MatrixBudget(0.016, load_pricing()[eval_args.model])

    class LargePromptModel(ScriptedModel):
        async def count_tokens(self, *args: Any, **kwargs: Any) -> int:
            barrier.wait(timeout=10)
            return 50_000

    def provider(**kwargs: Any) -> ScriptedModel:
        turn = model_turn('["real"]')
        turn[-1]["metadata"]["usage"] = {
            "inputTokens": 50_000,
            "outputTokens": 20,
            "totalTokens": 50_020,
            "cacheReadInputTokens": 0,
        }
        model = LargePromptModel([turn])
        models.append(model)
        return model

    monkeypatch.setattr(harness, "LiteLLMModel", provider)
    with ThreadPoolExecutor(max_workers=8) as pool:
        records = list(
            pool.map(lambda run: harness.run_once(eval_args, TASK, "control", run, GOLD), range(8))
        )

    assert sum(len(model.requests) for model in models) == 1
    assert sum(record.correct for record in records) == 1
    assert sum(record.budget_stop_reason is not None for record in records) == 7
    assert sum(record.input_tokens for record in records) == 50_000
    assert float(eval_args.matrix_budget.spent) == pytest.approx(0.015024)
    assert eval_args.matrix_budget.spent <= eval_args.matrix_budget.limit
    assert eval_args.matrix_budget.reserved == 0


@pytest.mark.parametrize("paid_prefix", [False, True])
def test_unknown_failed_usage_does_not_release_its_reserved_cash(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, paid_prefix: bool
) -> None:
    (eval_args.repo / "source.txt").write_text("source\n")
    turns = []
    if paid_prefix:
        turns.append(
            model_turn(
                "Searching.",
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            )
        )
    model = ScriptedModel([*turns, RuntimeError("stream interrupted before usage arrived")])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert record.error is not None
    assert record.budget_cost_usd > (0.00004224 if paid_prefix else 0)
    assert record.total_tokens is None
    assert record.known_usage["totalTokens"] == (120 if paid_prefix else 0)
    assert "n/a" in harness.summarize([record])
    assert float(eval_args.matrix_budget.spent) == record.budget_cost_usd
    assert eval_args.matrix_budget.reserved == 0


@pytest.mark.parametrize(
    ("option", "value", "message"),
    [
        ("--max-cost-usd", "0", "finite and positive"),
        ("--max-cost-usd", "nan", "finite and positive"),
        ("--max-run-tokens", "0", "positive token limit"),
        ("--model", "unknown/provider", "model has no price"),
    ],
)
def test_invalid_budget_fails_before_overwriting_results(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    option: str,
    value: str,
    message: str,
) -> None:
    output = tmp_path / "runs.jsonl"
    output.write_text("original results\n")
    monkeypatch.setattr(
        sys, "argv", ["harness.py", "--repo", str(tmp_path), "--out", str(output), option, value]
    )

    with pytest.raises(SystemExit) as error:
        harness.main()

    assert error.value.code == 2
    assert message in capsys.readouterr().err
    assert output.read_text() == "original results\n"


@pytest.mark.parametrize("inputs", [500, 100_000])
def test_terminal_overrun_keeps_paid_usage_and_is_not_a_success(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, inputs: int
) -> None:
    eval_args.run_token_limits = {"imports-001": 360}
    turn = model_turn('["real"]')
    turn[-1]["metadata"]["usage"] = {
        "inputTokens": inputs,
        "outputTokens": 20,
        "totalTokens": inputs + 20,
        "cacheReadInputTokens": 0,
    }
    model = ScriptedModel([turn])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert record.budget_stop_reason is not None
    assert not record.correct
    assert record.input_tokens == inputs
    assert record.total_tokens == inputs + 20
    assert record.answer == '["real"]'
    assert record.budget_cost_usd == pytest.approx(inputs * 0.0000003 + 0.000024)
    if inputs == 100_000:
        # Even though the matrix has money left, its input-cost bound was
        # violated. A subsequent run must not trust that estimate again.
        next_model = ScriptedModel([model_turn('["real"]')])
        monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: next_model)
        next_record = harness.run_once(eval_args, TASK, "control", 2, GOLD)
        assert next_model.requests == []
        assert "cost reservation" in next_record.budget_stop_reason


def test_unavailable_input_estimate_fails_closed_without_a_provider_call(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    class UncountableModel(ScriptedModel):
        async def count_tokens(self, *args: Any, **kwargs: Any) -> int:
            raise RuntimeError("token estimator unavailable")

    model = UncountableModel([model_turn('["real"]')])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert model.requests == []
    assert "estimate unavailable" in record.budget_stop_reason
    assert record.budget_cost_usd == 0


def test_cancelled_stream_settles_its_unknown_reservation(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    class CancelledModel(ScriptedModel):
        async def stream(self, *args: Any, **kwargs: Any) -> AsyncIterator[dict]:
            yield {"messageStart": {"role": "assistant"}}
            raise asyncio.CancelledError("interrupted without usage")

    model = CancelledModel([])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    with pytest.raises(asyncio.CancelledError, match="interrupted without usage"):
        harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert eval_args.matrix_budget.reserved == 0
    assert eval_args.matrix_budget.spent > 0


def test_zero_reported_usage_cannot_enable_a_free_agent_loop(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    turn = model_turn(
        "Searching.",
        stop_reason="tool_use",
        tool_name="read_file",
        tool_input={"path": "source.txt"},
    )
    turn[-1]["metadata"]["usage"] = {"inputTokens": 0, "outputTokens": 0, "totalTokens": 0}
    (eval_args.repo / "source.txt").write_text("source\n")
    model = ScriptedModel([turn, model_turn('["real"]')])
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(eval_args, TASK, "control", 1, GOLD)

    assert len(model.requests) == 1
    assert record.total_tokens is None
    assert record.budget_cost_usd > 0
    assert "usage unavailable" in record.budget_stop_reason
    assert not record.correct
