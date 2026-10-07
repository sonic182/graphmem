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


def group_cost(pricing: dict, group: list[Record]) -> float | None:
    costs = [
        cost_usd(
            pricing,
            record.model,
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens,
        )
        for record in group
    ]
    if not costs or any(cost is None for cost in costs):
        return None
    return sum(costs) / len(costs)


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
        tokens = [r.total_tokens for r in group]
        median = "n/a" if any(value is None for value in tokens) else f"{st.median(tokens):,.0f}"
        return f"{median}<br>{correct}/{len(group)}"

    def mean_cost(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        cost = group_cost(pricing, group) if group else None
        if not group:
            return "n/a"
        correct = sum(r.correct for r in group)
        formatted = "n/a" if cost is None else f"${cost:.4f}"
        return f"{formatted}<br>{correct}/{len(group)}"

    def token_reduction(task: str, model: str, variant: str) -> str:
        group = cell(task, model, variant)
        control = cell(task, model, "control")
        if not group or not control or any(r.total_tokens is None for r in [*group, *control]):
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

    def setting(group: list[Record], name: str) -> str:
        values = {getattr(record, name) for record in group}
        if len(values) != 1:
            return "mixed"
        value = values.pop()
        if value is None:
            return "not recorded"
        return f"${value:.8f}" if name == "max_matrix_cost_usd" else str(value)

    lines = [
        "# Cross-model comparison",
        "",
        "## Run settings and limit stops",
        "",
        "Accuracy and spend include all attempts, including limit stops. Resource",
        "limits may differ across models; these are not controlled model-quality rankings.",
        "Generation values are ceilings; a remaining token budget can lower a call's cap.",
        "",
        "| model | attempts | generation ceiling | temperature | jobs | matrix cost ceiling | run token caps | limit stops | other errors |",
        "|---|---:|---:|---:|---:|---:|---|---:|---:|",
    ]
    for model in models:
        group = [record for record in records if short_model(record.model) == model]
        limits = sum(record.budget_stop_reason is not None for record in group)
        other_errors = sum(
            record.error is not None and record.budget_stop_reason is None for record in group
        )
        token_caps = (
            "enabled"
            if all(record.max_run_tokens is not None for record in group)
            else "not recorded"
        )
        values = [
            setting(group, name)
            for name in ("max_tokens", "temperature", "jobs", "max_matrix_cost_usd")
        ]
        lines.append(
            f"| `{model}` | {len(group)} | "
            + " | ".join(values)
            + f" | {token_caps} | {limits} | {other_errors} |"
        )
    lines += ["", "## Median total tokens and accuracy", ""]
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
