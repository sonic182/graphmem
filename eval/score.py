"""Deterministic scoring for the benchmark answers.

No LLM judge: every task is scored against ``gold.json`` with exact or
line-range matching, so a score is reproducible from the raw answer.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

INT = re.compile(r"\d+")


def extract_json(text: str) -> Any | None:
    """Return the first JSON object or array in ``text``, or ``None``."""
    stripped = text.strip()
    for fence in ("```json", "```"):
        if fence in stripped:
            start = stripped.find(fence) + len(fence)
            end = stripped.find("```", start)
            stripped = stripped[start : end if end != -1 else len(stripped)].strip()
            break
    decoder = json.JSONDecoder()
    for index, character in enumerate(stripped):
        if character in "[{":
            try:
                value, _ = decoder.raw_decode(stripped[index:])
                return value
            except json.JSONDecodeError:
                continue
    return None


def _basename(path: str) -> str:
    return path.replace("\\", "/").rsplit("/", 1)[-1]


def _file_matches(candidate: str, expected: str) -> bool:
    candidate = candidate.replace("\\", "/")
    return (
        candidate.endswith(expected)
        or expected.endswith(candidate)
        or _basename(candidate) == _basename(expected)
    )


def _window_has_range(answer: str, file: str, start: int, end: int, tolerance: int) -> bool:
    lines = answer.splitlines()
    needle = _basename(file)
    for index, line in enumerate(lines):
        if needle not in line and file not in line:
            continue
        window = " ".join(lines[max(0, index - 10) : index + 11])
        for number in (int(value) for value in INT.findall(window)):
            if start - tolerance <= number <= end + tolerance:
                return True
    return False


def score_task(task_id: str, answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    scorer = SCORERS.get(task_id)
    if scorer is None:
        raise KeyError(f"no scorer for task {task_id}")
    return scorer(answer, gold)


def score_symbol(answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    parsed = extract_json(answer)
    checks = {
        "file": False,
        "kind": False,
        "start_line": False,
        "end_line": False,
    }
    if isinstance(parsed, dict):
        checks["file"] = isinstance(parsed.get("file"), str) and _file_matches(
            parsed["file"], gold["file"]
        )
        checks["kind"] = "function" in str(parsed.get("kind", "")).lower()
        checks["start_line"] = parsed.get("start_line") == gold["start_line"]
        checks["end_line"] = parsed.get("end_line") == gold["end_line"]
    passed = sum(checks.values())
    return {"score": passed / len(checks), "details": checks}


def score_outline(answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    parsed = extract_json(answer)
    items = parsed if isinstance(parsed, list) else []
    matched = []
    for required in gold["required"]:
        for item in items:
            if not isinstance(item, dict):
                continue
            name = str(item.get("name", "")).strip()
            if name != required["name"]:
                continue
            start = item.get("start_line")
            if isinstance(start, int) and abs(start - required["start_line"]) <= 2:
                matched.append(required["name"])
                break
    return {
        "score": len(matched) / len(gold["required"]),
        "details": {
            "matched": matched,
            "missing": [r["name"] for r in gold["required"] if r["name"] not in matched],
        },
    }


def _normalize_import_specifier(item: Any) -> str:
    value = str(item).strip()
    while len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"', "`"}:
        value = value[1:-1].strip()
    return value


def score_imports(answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    parsed = extract_json(answer)
    found = (
        {_normalize_import_specifier(item) for item in parsed}
        if isinstance(parsed, list)
        else set()
    )
    expected = set(gold["specifiers"])
    hit = found & expected
    precision = len(hit) / len(found) if found else 0.0
    recall = len(hit) / len(expected) if expected else 0.0
    f1 = 2 * precision * recall / (precision + recall) if precision + recall else 0.0
    return {
        "score": f1,
        "details": {
            "missing": sorted(expected - found),
            "extra": sorted(found - expected),
            "exact": found == expected,
        },
    }


def score_diff(answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    parsed = extract_json(answer)
    items = parsed if isinstance(parsed, list) else []
    matched = []
    for required in gold["required"]:
        for item in items:
            if not isinstance(item, dict):
                continue
            symbol = str(item.get("symbol", "")).strip()
            file = str(item.get("file", ""))
            if symbol == required["symbol"] and _file_matches(file, required["file"]):
                matched.append(required["symbol"])
                break
    return {
        "score": len(matched) / len(gold["required"]),
        "details": {
            "matched": matched,
            "missing": [r["symbol"] for r in gold["required"] if r["symbol"] not in matched],
        },
    }


def score_workflow(answer: str, gold: dict[str, Any]) -> dict[str, Any]:
    lower = answer.lower()
    details: dict[str, bool] = {}
    for item in gold["rubric"]:
        if item["type"] == "citation":
            if "file_any" in item:
                passed = any(_basename(path).lower() in lower for path in item["file_any"])
            else:
                passed = _window_has_range(
                    answer,
                    item["file"],
                    item["start_line"],
                    item["end_line"],
                    item.get("tolerance", 5),
                )
        else:
            passed = any(option.lower() in lower for option in item["any_of"])
        details[item["id"]] = passed
    return {"score": sum(details.values()) / len(details), "details": details}


SCORERS = {
    "symbol-001": score_symbol,
    "outline-001": score_outline,
    "imports-001": score_imports,
    "diff-001": score_diff,
    "workflow-001": score_workflow,
}


def load_gold(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())
