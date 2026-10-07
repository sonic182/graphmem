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
