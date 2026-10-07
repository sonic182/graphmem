"""Exercise reporting CLIs against temporary run files, without API calls."""

import json
import subprocess
import sys
from collections.abc import Callable
from dataclasses import asdict
from pathlib import Path

import pytest

from harness import Record

HERE = Path(__file__).resolve().parent


def write_runs(path: Path, records: list[Record]) -> None:
    """Write independent run inputs for CLI tests."""
    path.write_text("".join(json.dumps(asdict(record)) + "\n" for record in records))


def run_script(name: str, *args: str) -> subprocess.CompletedProcess[str]:
    """Invoke a reporting command with the test environment's Python."""
    return subprocess.run(
        [sys.executable, str(HERE / name), *args],
        capture_output=True,
        text=True,
        timeout=60,
        check=True,
    )


@pytest.mark.parametrize("error", [None, "MaxTokensReachedException: capped"])
def test_rescore_uses_final_turn_and_preserves_usage_and_failure(
    tmp_path: Path, record_factory: Callable[..., Record], error: str | None
) -> None:
    runs = tmp_path / "runs.jsonl"
    summary = tmp_path / "summary.md"
    gold = tmp_path / "gold.json"
    record = record_factory(
        answer='["wrong"]',
        error=error,
        benchmark_version=2,
        assistant_messages=[
            {"role": "assistant", "content": [{"text": '["wrong"]'}]},
            {"role": "assistant", "content": [{"text": '["real"]'}]},
        ],
    )
    write_runs(runs, [record])
    gold.write_text(json.dumps({"imports-001": {"specifiers": ["real"]}}))

    run_script("rescore.py", "--runs", str(runs), "--summary", str(summary), "--gold", str(gold))
    rescored = json.loads(runs.read_text())

    assert rescored["answer"] == '["real"]'
    assert rescored["score"] == 1.0
    assert rescored["correct"] is (error is None)
    assert rescored["error"] == error
    assert rescored["input_tokens"] == 100
    assert rescored["total_tokens"] == 120
    assert summary.exists()


def test_rescore_rejects_missing_turns_without_overwriting_input(
    tmp_path: Path, record_factory: Callable[..., Record]
) -> None:
    runs = tmp_path / "runs.jsonl"
    gold = tmp_path / "gold.json"
    summary = tmp_path / "summary.md"
    data = asdict(record_factory(answer='["real"]'))
    del data["assistant_messages"]
    original = json.dumps(data) + "\n"
    runs.write_text(original)
    gold.write_text(json.dumps({"imports-001": {"specifiers": ["real"]}}))

    with pytest.raises(subprocess.CalledProcessError) as caught:
        run_script(
            "rescore.py", "--runs", str(runs), "--summary", str(summary), "--gold", str(gold)
        )
    assert "no assistant_messages" in caught.value.stderr
    assert runs.read_text() == original
    assert not summary.exists()


def test_compare_uses_median_tokens_and_keeps_accuracy(
    tmp_path: Path, record_factory: Callable[..., Record]
) -> None:
    runs = tmp_path / "runs.jsonl"
    output = tmp_path / "comparison.md"
    write_runs(
        runs,
        [
            record_factory(run=1, input_tokens=80, total_tokens=100),
            record_factory(run=2, input_tokens=80, total_tokens=100),
            record_factory(
                run=3,
                input_tokens=9980,
                total_tokens=10000,
                correct=False,
                error="EvaluationLimitError: run token budget",
                budget_stop_reason="run token budget",
            ),
        ],
    )

    run_script("compare.py", "--runs", str(runs), "--out", str(output))
    report = output.read_text()
    assert "| `imports-001` | 100<br>2/3 |" in report
    assert "Mean cost USD and accuracy" in report
    assert (
        "| `deepseek-v4p1-flash` | 3 | not recorded | not recorded | not recorded | not recorded | not recorded | 1 | 0 |"
        in report
    )
    assert "Resource\nlimits may differ across models" in report


def test_compare_handles_unknown_usage_without_reporting_zero_cost(
    tmp_path: Path, record_factory: Callable[..., Record]
) -> None:
    runs = tmp_path / "runs.jsonl"
    output = tmp_path / "comparison.md"
    write_runs(
        runs,
        [
            record_factory(
                input_tokens=None,
                output_tokens=None,
                total_tokens=None,
                cache_read_tokens=None,
                correct=False,
                error="provider unavailable",
            )
        ],
    )

    run_script("compare.py", "--runs", str(runs), "--out", str(output))
    report = output.read_text()
    assert "n/a<br>0/1" in report
    assert "$0.0000" not in report
