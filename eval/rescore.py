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

from harness import DEFAULT_MODEL, Record, summarize
from score import load_gold, score_task

HERE = Path(__file__).resolve().parent


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs", type=Path, default=HERE / "results" / "runs.jsonl")
    parser.add_argument("--gold", type=Path, default=HERE / "gold.json")
    parser.add_argument("--summary", type=Path, default=HERE / "results" / "summary.md")
    parser.add_argument("--model", default=DEFAULT_MODEL, help="Fill in a missing model field")
    args = parser.parse_args()

    gold = load_gold(args.gold)
    records = []
    for line in args.runs.read_text().splitlines():
        if not line.strip():
            continue
        record = Record(**json.loads(line))
        scored = score_task(record.task, record.answer, gold[record.task])
        record.score = scored["score"]
        record.correct = scored["score"] >= 1.0
        record.details = scored["details"]
        record.model = record.model or args.model
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
