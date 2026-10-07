#!/usr/bin/env python3
"""Merge per-model run files into one wide comparison table.

Reads every ``results/**/runs.jsonl`` (or the paths given on the command
line), groups by model, task and variant, and prints, for each cell:

1. median total tokens and accuracy,
2. mean cost in USD (from ``pricing.json``) and accuracy,
3. token reduction against the model's own control (median), and
4. cost reduction against the model's own control (mean).

Medians are used for tokens because a single runaway agent loop (a weak model
that never converges) can otherwise dominate the mean. Cost is a mean because
it is the money actually spent.

Use one ``runs.jsonl`` per model, produced with ``harness.py --model ...``.
"""

from __future__ import annotations

import argparse
import json
import statistics as st
from pathlib import Path

from harness import Record
from pricing import cost_usd, load_pricing

HERE = Path(__file__).resolve().parent
VARIANTS = ["control", "gmem", "gmem-guided"]


def short_model(model: str) -> str:
    return model.rsplit("/", 1)[-1] or "unknown"


def load(paths: list[Path]) -> list[Record]:
    records = []
    for path in paths:
        for line in path.read_text().splitlines():
            if line.strip():
                records.append(Record(**json.loads(line)))
    return records


def mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def group_cost(pricing: dict, group: list[Record]) -> float | None:
    costs = [
        cost
        for record in group
        if (
            cost := cost_usd(
                pricing,
                record.model,
                record.input_tokens,
                record.output_tokens,
                record.cache_read_tokens,
            )
        )
        is not None
    ]
    return sum(costs) / len(costs) if costs else None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--runs",
        type=Path,
        nargs="*",
        help="runs.jsonl files; defaults to every results/**/runs.jsonl",
    )
    parser.add_argument("--out", type=Path, default=HERE / "results" / "comparison.md")
    args = parser.parse_args()

    paths = args.runs or sorted((HERE / "results").rglob("runs.jsonl"))
    records = load(paths)
    if not records:
        raise SystemExit(f"no run records found in {paths}")
    pricing = load_pricing()

    models = sorted({short_model(record.model) for record in records})
    tasks = sorted({record.task for record in records})
    columns = [(model, variant) for model in models for variant in VARIANTS]
    change_columns = [(model, variant) for model, variant in columns if variant != "control"]

    def cell(task: str, model: str, variant: str) -> list[Record]:
        return [
            r
            for r in records
            if r.task == task and short_model(r.model) == model and r.variant == variant
        ]

    def pivot(columns, value) -> list[str]:
        header = " | ".join(f"{m}<br>{v}" for m, v in columns)
        lines = [f"| task | {header} |", "|---|" + "---:|" * len(columns)]
        for task in tasks:
            cells = [value(task, model, variant) for model, variant in columns]
            lines.append(f"| `{task}` | " + " | ".join(cells) + " |")
        return lines

    def median_tokens(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        if not group:
            return "n/a"
        correct = sum(r.correct for r in group)
        return f"{st.median([r.total_tokens for r in group]):,.0f}<br>{correct}/{len(group)}"

    def mean_cost(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        cost = group_cost(pricing, group) if group else None
        if cost is None:
            return "n/a"
        correct = sum(r.correct for r in group)
        return f"${cost:.4f}<br>{correct}/{len(group)}"

    def token_reduction(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        control = cell(task, model, "control")
        if not group or not control:
            return "n/a"
        control_median = st.median([r.total_tokens for r in control])
        if not control_median:
            return "n/a"
        median = st.median([r.total_tokens for r in group])
        return f"{(1 - median / control_median) * 100:+.0f}%"

    def cost_reduction(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        control_cost = group_cost(pricing, cell(task, model, "control"))
        cost = group_cost(pricing, group) if group else None
        if cost is None or not control_cost:
            return "n/a"
        return f"{(1 - cost / control_cost) * 100:+.0f}%"

    lines = ["# Cross-model comparison", "", "## Median total tokens and accuracy", ""]
    lines += pivot(columns, median_tokens)
    lines += ["", "## Mean cost USD and accuracy", ""]
    lines += pivot(columns, mean_cost)
    lines += ["", "## Token reduction vs control (median, higher is better)", ""]
    lines += pivot(change_columns, token_reduction)
    lines += ["", "## Cost reduction vs control (mean, higher is better)", ""]
    lines += pivot(change_columns, cost_reduction)

    table = "\n".join(lines) + "\n"
    args.out.write_text(table)
    print(table)
    print(f"wrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
