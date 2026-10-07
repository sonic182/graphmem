#!/usr/bin/env python3
"""Re-score an existing ``runs.jsonl`` against the current ``gold.json``.

Scoring changes (a corrected gold answer, a wider citation window) should not
require re-running the agents. This rewrites the score, correct and details
fields in place and regenerates the summary.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict
from pathlib import Path

from harness import Record, extract_answer, summarize
from score import load_gold, score_task

HERE = Path(__file__).resolve().parent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=HERE / "results" / "runs.jsonl")
    parser.add_argument("--gold", type=Path, default=HERE / "gold.json")
    parser.add_argument("--summary", type=Path, default=HERE / "results" / "summary.md")
    args = parser.parse_args()

    gold = load_gold(args.gold)
    records = []
    for line in args.runs.read_text().splitlines():
        if not line.strip():
            continue
        data = json.loads(line)
        if "assistant_messages" not in data:
            parser.error("run has no assistant_messages; regenerate it with the current harness")
        record = Record(**data)
        record.answer = extract_answer(record.assistant_messages)
        scored = score_task(record.task, record.answer, gold[record.task])
        record.score = scored["score"]
        record.correct = record.error is None and scored["score"] >= 1.0
        record.details = scored["details"]
        records.append(record)

    with args.runs.open("w") as handle:
        for record in records:
            handle.write(json.dumps(asdict(record)) + "\n")
    table = summarize(records)
    args.summary.write_text(table + "\n")
    print(table)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
