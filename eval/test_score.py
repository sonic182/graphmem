"""Regression coverage for semantic import-specifier scoring."""

import json

import pytest

from score import score_task


@pytest.mark.parametrize(
    ("specifiers", "expected_score"),
    [
        (["node:path", "./setup.js"], 1.0),
        (['"node:path"', '"./setup.js"'], 1.0),
        (["`node:path`", "'./setup.js'"], 1.0),
        (['  `"node:path"`  ', "./wrong.js"], 0.5),
    ],
)
def test_import_score_ignores_wrappers_but_not_wrong_specifiers(
    specifiers: list[str], expected_score: float
) -> None:
    """Accept presentation wrappers without forgiving a different import."""
    result = score_task(
        "imports-001",
        json.dumps(specifiers),
        {"specifiers": ["node:path", "./setup.js"]},
    )

    assert result["score"] == expected_score


@pytest.mark.parametrize(
    ("file", "expected_match"),
    [
        (None, False),
        ("", False),
        ("setup.ts", False),
        ("other/setup.ts", False),
        ("/src/setup.ts", False),
        ("src/../src/setup.ts", False),
        ("src/setup.ts", True),
        ("./src/setup.ts", True),
        ("src\\setup.ts", True),
    ],
)
@pytest.mark.parametrize("task", ["symbol-001", "diff-001"])
def test_symbol_scores_require_the_correct_repository_relative_path(
    file: str | None, expected_match: bool, task: str
) -> None:
    if task == "symbol-001":
        answer = {"kind": "function", "start_line": 10, "end_line": 20}
        gold = {"file": "src/setup.ts", "start_line": 10, "end_line": 20}
        if file is not None:
            answer["file"] = file
        result = score_task(task, json.dumps(answer), gold)
        assert result["details"]["file"] is expected_match
    else:
        answer = {"symbol": "setup"}
        gold = {"required": [{"file": "src/setup.ts", "symbol": "setup"}]}
        if file is not None:
            answer["file"] = file
        result = score_task(task, json.dumps([answer]), gold)
        assert result["score"] == float(expected_match)
