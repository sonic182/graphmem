"""Exercise scoring and accounting through the real offline agent loop."""

import json
import sys
from argparse import Namespace
from pathlib import Path

import pytest

import harness
from conftest import ScriptedModel, model_turn
from pricing import cost_usd


@pytest.mark.parametrize(
    ("preliminary", "final", "expected_score"),
    [(' ["wrong"] ', '["real"]', 1.0), ('["real"]', '["wrong"]', 0.0)],
)
def test_run_once_scores_only_terminal_response(
    monkeypatch: pytest.MonkeyPatch,
    eval_args: Namespace,
    preliminary: str,
    final: str,
    expected_score: float,
) -> None:
    Path(eval_args.repo / "source.txt").write_text("source\n")
    model = ScriptedModel(
        [
            model_turn(
                preliminary,
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            ),
            model_turn(final),
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

    assert record.error is None
    assert record.answer == final
    assert record.score == expected_score
    assert len(record.assistant_messages) == 2
    assert record.input_tokens == 200
    assert record.tool_calls == 1
    assert record.benchmark_version == 2
    assert record.max_tokens == 8192


def test_run_once_preserves_paid_usage_when_generation_hits_limit(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    Path(eval_args.repo / "source.txt").write_text("source\n")
    model = ScriptedModel(
        [
            model_turn(
                "Searching.",
                stop_reason="tool_use",
                tool_name="read_file",
                tool_input={"path": "source.txt"},
            ),
            model_turn('["real"]', stop_reason="max_tokens"),
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

    assert record.error.startswith("MaxTokensReachedException:")
    assert record.input_tokens == 200
    assert record.output_tokens == 40
    assert record.total_tokens == 240
    assert record.cache_read_tokens == 80
    assert record.cycles == 2
    assert record.tool_calls == 1
    assert not record.correct
    assert record.tool_output_chars > 0
    rates = {eval_args.model: {"input": 1.0, "output": 2.0, "cache_read": 0.25}}
    assert (
        cost_usd(
            rates, record.model, record.input_tokens, record.output_tokens, record.cache_read_tokens
        )
        == 0.00022
    )


def test_setup_failure_is_unknown_cost_not_free(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    def fail_model(**kwargs: object) -> None:
        raise RuntimeError("provider unavailable")

    monkeypatch.setattr(harness, "LiteLLMModel", fail_model)
    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "control",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert record.error == "RuntimeError: provider unavailable"
    assert record.total_tokens is None
    assert record.input_tokens is None
    assert record.output_tokens is None
    assert record.cache_read_tokens is None
    assert "n/a" in harness.summarize([record])


@pytest.mark.parametrize(
    "messages",
    [
        [],
        [
            {"role": "assistant", "content": [{"text": "preliminary"}]},
            {"role": "user", "content": []},
        ],
        [
            {
                "role": "assistant",
                "content": [{"text": "preliminary"}, {"toolUse": {"name": "read_file"}}],
            }
        ],
    ],
)
def test_extract_answer_requires_a_completed_assistant_turn(messages: list[dict]) -> None:
    assert harness.extract_answer(messages) == ""


def test_harness_cli_writes_complete_parallel_runs_without_an_api_key(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    tasks = tmp_path / "tasks.json"
    gold = tmp_path / "gold.json"
    output = tmp_path / "runs.jsonl"
    summary = tmp_path / "summary.md"
    tasks.write_text(json.dumps({"tasks": [{"id": "imports-001", "prompt": "Find imports."}]}))
    gold.write_text(json.dumps({"imports-001": {"specifiers": ["real"]}}))
    monkeypatch.delenv("FIREWORKS_API_KEY", raising=False)
    monkeypatch.setattr(
        harness, "load_pricing", lambda: {"offline/test": {"input": 1.0, "output": 2.0}}
    )
    monkeypatch.setattr(
        harness, "LiteLLMModel", lambda **kwargs: ScriptedModel([model_turn('["real"]')])
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "harness.py",
            "--repo",
            str(tmp_path),
            "--model",
            "offline/test",
            "--tasks",
            str(tasks),
            "--gold",
            str(gold),
            "--out",
            str(output),
            "--summary",
            str(summary),
            "--variants",
            "control",
            "--runs",
            "2",
            "--jobs",
            "2",
        ],
    )

    assert harness.main() == 0
    records = [json.loads(line) for line in output.read_text().splitlines()]
    assert {record["run"] for record in records} == {1, 2}
    assert len(records) == 2
    assert all(record["correct"] and record["error"] is None for record in records)
    assert all(record["total_tokens"] == 120 for record in records)
    assert all(record["jobs"] == 2 for record in records)
    assert "2/2" in summary.read_text()
