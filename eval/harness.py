#!/usr/bin/env python3
"""Minimal Strands agent benchmark for the gmem code tools.

Runs the tasks in ``tasks.json`` against two otherwise identical Strands
agents and records real token usage, tool calls, source lines read and wall
time for each run:

  control      shell + read_file
  gmem         shell + read_file + the gmem MCP code tools
  gmem-guided  as gmem, plus the shipped plugin guidance and a skill loader

The control keeps rg, sed, git and every other shell workflow: gmem is purely
additive, so the comparison answers "does an agent with gmem reach the same
answer using less context?", not "can gmem beat an agent we disabled".

The model is any LiteLLM provider; the default is Fireworks
``deepseek-v4p1-flash``. The API key is read from ``FIREWORKS_API_KEY``.

Examples:

  # one task, both variants, smoke test
  uv run harness.py --repo /path/to/openclaw-bench \\
      --task-filter symbol-001 --runs 1

  # full matrix: 5 tasks x 2 variants x 5 runs, 4 runs at a time
  uv run harness.py --repo /path/to/openclaw-bench --runs 5 --jobs 4
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass, field
from pathlib import Path

from mcp import StdioServerParameters, stdio_client
from strands import Agent
from strands.models.litellm import LiteLLMModel
from strands.tools.mcp import MCPClient

from score import load_gold, score_task
from tools import RunMetrics, make_skill_tool, make_tools

HERE = Path(__file__).resolve().parent
DEFAULT_MODEL = "fireworks_ai/accounts/fireworks/models/deepseek-v4p1-flash"
CODE_TOOLS = ["find_symbol", "code_outline", "code_imports", "code_diff"]
VARIANTS = ["control", "gmem", "gmem-guided"]


@dataclass
class Record:
    variant: str
    task: str
    run: int
    score: float
    correct: bool
    input_tokens: int
    output_tokens: int
    total_tokens: int
    cache_read_tokens: int
    cycles: int
    wall_ms: int
    tool_calls: int
    tool_calls_by_name: dict[str, int] = field(default_factory=dict)
    shell_calls: int = 0
    file_reads: int = 0
    tool_output_chars: int = 0
    source_lines_read: int = 0
    answer: str = ""
    details: dict = field(default_factory=dict)
    error: str | None = None
    model: str = ""


def load_tasks(path: Path, only: set[str] | None) -> list[dict]:
    tasks = json.loads(path.read_text())["tasks"]
    if only:
        tasks = [task for task in tasks if task["id"] in only]
    return tasks


def mcp_env(gmem_home: Path, max_files: int) -> dict[str, str]:
    return dict(os.environ) | {
        "GRAPHMEM_HOME": str(gmem_home),
        "GRAPHMEM_CODE_MAX_FILES": str(max_files),
    }


def build_agent(args, repo: Path, variant: str, metrics: RunMetrics):
    model = LiteLLMModel(
        model_id=args.model,
        params={"temperature": args.temperature, "max_tokens": args.max_tokens},
    )
    agent_tools = make_tools(repo, metrics)
    client = None
    if variant in ("gmem", "gmem-guided"):
        params = StdioServerParameters(
            command=str(args.gmem),
            args=["mcp"],
            env=mcp_env(args.gmem_home, args.gmem_max_files),
            cwd=str(repo),
        )
        client = MCPClient(lambda: stdio_client(params), tool_filters={"allowed": CODE_TOOLS})
        client.start()
        agent_tools = agent_tools + client.list_tools_sync()
    if variant == "gmem-guided":
        agent_tools = [*agent_tools, make_skill_tool(args.skills_dir, metrics)]
    agent = Agent(
        model=model,
        system_prompt=args.system_prompt,
        tools=agent_tools,
        callback_handler=None,
    )
    return agent, client


def collect_tool_chars(messages: list[dict]) -> dict[str, int]:
    names: dict[str, str] = {}
    chars: dict[str, int] = {}
    for message in messages:
        for block in message.get("content", []):
            if "toolUse" in block:
                use = block["toolUse"]
                names[use.get("toolUseId")] = use.get("name", "?")
            elif "toolResult" in block:
                result = block["toolResult"]
                name = names.get(result.get("toolUseId"), "?")
                text = "".join(part.get("text", "") for part in result.get("content", []))
                chars[name] = chars.get(name, 0) + len(text)
    return chars


def extract_answer(messages: list[dict]) -> str:
    texts = []
    for message in messages:
        if message.get("role") != "assistant":
            continue
        for block in message.get("content", []):
            if "text" in block:
                texts.append(block["text"])
    return "\n".join(texts).strip()


def run_once(args, task: dict, variant: str, run: int, gold: dict) -> Record:
    metrics = RunMetrics()
    started = time.monotonic()
    agent = None
    client = None
    result = None
    error = None
    try:
        agent, client = build_agent(args, args.repo, variant, metrics)
        prompt = task["prompt"]
        if variant == "gmem-guided":
            prompt = f"{args.guidance_text}\n\n---\n\n{prompt}"
        result = agent(prompt)
    except Exception as exception:
        error = f"{type(exception).__name__}: {exception}"
    finally:
        if client is not None:
            client.stop(None, None, None)
    wall_ms = int((time.monotonic() - started) * 1000)

    messages = agent.messages if agent is not None else []
    answer = extract_answer(messages)
    scored = score_task(task["id"], answer, gold[task["id"]])
    summary = result.metrics.get_summary() if result is not None else {}
    usage = summary.get("accumulated_usage", {})
    tool_usage = {
        name: stats["execution_stats"]["call_count"]
        for name, stats in summary.get("tool_usage", {}).items()
    }
    chars = collect_tool_chars(messages)
    return Record(
        variant=variant,
        task=task["id"],
        run=run,
        score=scored["score"],
        correct=scored["score"] >= 1.0,
        input_tokens=int(usage.get("inputTokens", 0)),
        output_tokens=int(usage.get("outputTokens", 0)),
        total_tokens=int(usage.get("totalTokens", 0)),
        cache_read_tokens=int(usage.get("cacheReadInputTokens", 0)),
        cycles=int(summary.get("total_cycles", 0)),
        wall_ms=wall_ms,
        tool_calls=sum(tool_usage.values()),
        tool_calls_by_name=tool_usage,
        shell_calls=metrics.shell_calls,
        file_reads=metrics.file_reads,
        tool_output_chars=sum(chars.values()),
        source_lines_read=metrics.source_lines_read,
        answer=answer,
        details=scored["details"],
        error=error,
        model=args.model,
    )


def run_and_log(args, run: int, task: dict, variant: str, gold: dict) -> Record:
    print(f"[run {run}] {task['id']} / {variant} ...", flush=True)
    record = run_once(args, task, variant, run, gold)
    status = "ok" if record.error is None else record.error
    print(
        f"  [run {run}] {task['id']} / {variant}: score={record.score:.2f} "
        f"total={record.total_tokens} in={record.input_tokens} out={record.output_tokens} "
        f"cycles={record.cycles} tools={record.tool_calls} "
        f"src_lines={record.source_lines_read} {record.wall_ms}ms {status}",
        flush=True,
    )
    return record


def mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


def summarize(records: list[Record]) -> str:
    lines = [
        "| task | variant | success | score | total tok | input | output | cycles | tool calls | src lines | wall ms |",
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    present = [v for v in VARIANTS if any(r.variant == v for r in records)]
    for task in sorted({record.task for record in records}):
        for variant in present:
            group = [r for r in records if r.task == task and r.variant == variant]
            if not group:
                continue
            lines.append(
                f"| `{task}` | {variant} | {sum(r.correct for r in group)}/{len(group)} | "
                f"{mean([r.score for r in group]):.2f} | {mean([r.total_tokens for r in group]):.0f} | "
                f"{mean([r.input_tokens for r in group]):.0f} | {mean([r.output_tokens for r in group]):.0f} | "
                f"{mean([r.cycles for r in group]):.1f} | {mean([r.tool_calls for r in group]):.1f} | "
                f"{mean([r.source_lines_read for r in group]):.0f} | {mean([r.wall_ms for r in group]):.0f} |"
            )
    others = [v for v in present if v != "control"]
    header = " | ".join(f"{v} tok" for v in others)
    separators = " | ".join("---:" for _ in others)
    lines += [
        "",
        f"| task | control tok | {header} | " + " | ".join(f"{v} red." for v in others) + " |",
        f"|---|---:|{separators}|" + "---:|" * len(others),
    ]
    for task in sorted({record.task for record in records}):
        control = [r for r in records if r.task == task and r.variant == "control"]
        if not control:
            continue
        control_tokens = mean([r.total_tokens for r in control])
        cells = []
        for variant in others:
            group = [r for r in records if r.task == task and r.variant == variant]
            if not group:
                cells.append(("n/a", "n/a"))
                continue
            tokens = mean([r.total_tokens for r in group])
            reduction = 1 - tokens / control_tokens if control_tokens else 0.0
            cells.append((f"{tokens:.0f}", f"{reduction * 100:.0f}%"))
        lines.append(
            f"| `{task}` | {control_tokens:.0f} | "
            + " | ".join(tokens for tokens, _ in cells)
            + " | "
            + " | ".join(reduction for _, reduction in cells)
            + " |"
        )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--repo", required=True, type=Path, help="Pinned checkout to run the agents in"
    )
    parser.add_argument("--tasks", type=Path, default=HERE / "tasks.json")
    parser.add_argument("--gold", type=Path, default=HERE / "gold.json")
    parser.add_argument("--system-prompt", type=Path, default=HERE / "prompts" / "system.txt")
    parser.add_argument("--out", type=Path, default=HERE / "results" / "runs.jsonl")
    parser.add_argument("--summary", type=Path, default=HERE / "results" / "summary.md")
    parser.add_argument("--gmem", type=Path, default=Path("gmem"))
    parser.add_argument(
        "--gmem-home", type=Path, default=Path.home() / ".cache" / "gmem-eval" / "index"
    )
    parser.add_argument("--gmem-max-files", type=int, default=100_000)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--temperature", type=float, default=0.0)
    parser.add_argument("--max-tokens", type=int, default=8192)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--variants", default="control,gmem,gmem-guided")
    parser.add_argument("--skills-dir", type=Path, default=HERE.parent / "plugin" / "skills")
    parser.add_argument("--guidance", type=Path, default=HERE / "prompts" / "guidance.txt")
    parser.add_argument("--task-filter", help="Comma-separated task ids")
    parser.add_argument(
        "--jobs",
        type=int,
        default=4,
        help="Runs to execute concurrently; 1 keeps the interleaved order exactly",
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    args.repo = args.repo.resolve()
    args.gmem = args.gmem.resolve()
    args.system_prompt = args.system_prompt.read_text()
    args.guidance_text = args.guidance.read_text()
    variants = [name.strip() for name in args.variants.split(",") if name.strip()]
    only = {name.strip() for name in args.task_filter.split(",")} if args.task_filter else None
    tasks = load_tasks(args.tasks, only)
    gold = load_gold(args.gold)

    if args.dry_run:
        for run in range(1, args.runs + 1):
            order = variants if run % 2 == 1 else list(reversed(variants))
            for task in tasks:
                print(f"run {run}: {task['id']} -> {order}")
        print(f"\nmodel={args.model} repo={args.repo} gmem={args.gmem}")
        return 0

    if not os.environ.get("FIREWORKS_API_KEY") and args.model.startswith("fireworks"):
        return int(bool(sys.stderr.write("FIREWORKS_API_KEY is not set\n")) or 1)

    units: list[tuple[int, dict, str]] = []
    for run in range(1, args.runs + 1):
        order = variants if run % 2 == 1 else list(reversed(variants))
        units += [(run, task, variant) for task in tasks for variant in order]

    if args.jobs <= 1:
        records = [run_and_log(args, run, task, variant, gold) for run, task, variant in units]
    else:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = {
                pool.submit(run_and_log, args, run, task, variant, gold): (run, task, variant)
                for run, task, variant in units
            }
            records = [future.result() for future in as_completed(futures)]
    records.sort(key=lambda record: (record.run, record.task, record.variant))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as handle:
        for record in records:
            handle.write(json.dumps(asdict(record)) + "\n")
    table = summarize(records)
    args.summary.write_text(table + "\n")
    print("\n" + table)
    print(f"\nwrote {args.out} and {args.summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
