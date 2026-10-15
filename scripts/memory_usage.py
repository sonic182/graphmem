#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Measure how Claude Code sessions use gmem recalls.

Pairs every recall found in the Claude Code transcripts with the tool calls that
follow it, until the next user prompt or --window calls. A recalled memory counts
as followed when one of those calls mentions a path cited in its **Where** section.
A memory with no path cannot be verified this way and is left out of the rates.
"""

import argparse
import json
import os
import random
import sqlite3
from collections import Counter, defaultdict
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path

RECALL_TOOLS = frozenset({"mcp__plugin_graphmem_gmem__recall", "mcp__gmem__recall"})
GMEM_PREFIXES = ("mcp__plugin_graphmem_gmem__", "mcp__gmem__")
MEMORY_TOOLS = frozenset(
    {"recall", "remember", "update", "forget", "inspect", "relate", "graph", "stats", "list_scopes"}
)
REVISE_TOOLS = frozenset({"update", "forget"})
PATH_EXTENSIONS = frozenset(
    {
        "rs", "ex", "exs", "py", "js", "ts", "tsx", "jsx", "md", "toml", "json", "yml", "yaml",
        "sql", "sh", "html", "heex", "eex", "css", "scss", "php", "rb", "go", "lock",
    }
)  # fmt: skip
SEPARATORS = frozenset(",;()`\"'[]")
PLACEHOLDER_CHARACTERS = frozenset("*?{}#=<>")
SUMMARY_WIDTH = 160
NOISY_RETURNS = 5


@dataclass
class Recall:
    session: str
    project: str
    at: str
    prompt: str
    query: str
    memories: list[dict]
    cited: dict[int, list[str]]
    actions: list[str] = field(default_factory=list)
    calls: int = 0
    followed: set[int] = field(default_factory=set)
    revised: set[int] = field(default_factory=set)


def section_tokens(section: str) -> list[str]:
    tokens: list[str] = []
    current: list[str] = []
    depth = 0
    for character in section:
        if character == "{":
            depth += 1
            current.append(character)
        elif character == "}":
            depth = max(depth - 1, 0)
            current.append(character)
        elif character == "," and depth:
            current.append(character)
        elif character in SEPARATORS or character.isspace():
            tokens.append("".join(current))
            current = []
        else:
            current.append(character)
    tokens.append("".join(current))
    return [token for token in tokens if token]


def expand_braces(token: str) -> list[str]:
    start = token.find("{")
    end = token.find("}", start) if start >= 0 else -1
    if end < 0:
        return [token]
    prefix, suffix = token[:start], token[end + 1 :]
    return [
        path
        for alternative in token[start + 1 : end].split(",")
        for path in expand_braces(f"{prefix}{alternative}{suffix}")
    ]


def strip_line_suffix(candidate: str) -> str:
    path, colon, line = candidate.rpartition(":")
    if colon and line and all(character.isdigit() or character == "-" for character in line):
        return path
    return candidate


def looks_like_path(candidate: str) -> bool:
    if not candidate or candidate.startswith(("~", "/")) or "://" in candidate:
        return False
    if PLACEHOLDER_CHARACTERS.intersection(candidate):
        return False
    if candidate.endswith("/"):
        return len(candidate) > 1
    stem, dot, extension = candidate.rsplit("/", 1)[-1].rpartition(".")
    return bool(dot and stem and extension in PATH_EXTENSIONS)


def referenced_paths(content: str) -> list[str]:
    _, marker, after = content.partition("**Where**:")
    if not marker:
        return []
    paths: list[str] = []
    for token in section_tokens(after.split("**")[0]):
        for candidate in expand_braces(token):
            candidate = strip_line_suffix(candidate.rstrip(".:"))
            if looks_like_path(candidate) and candidate not in paths:
                paths.append(candidate)
    return paths


def summary(content: str) -> str:
    lines = (line.strip() for line in content.splitlines())
    line = next((line for line in lines if line), "")
    return line.removeprefix("**What**:").strip()[:SUMMARY_WIDTH]


def read_events(path: Path) -> Iterator[dict]:
    with path.open(encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if "tool_use" not in line and "tool_result" not in line and '"user"' not in line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict):
                yield event


def human_text(content: object) -> str | None:
    if isinstance(content, str):
        return content.strip()
    if not isinstance(content, list):
        return None
    if any(isinstance(block, dict) and block.get("type") == "tool_result" for block in content):
        return None
    texts = [
        block.get("text", "")
        for block in content
        if isinstance(block, dict) and block.get("type") == "text"
    ]
    return "\n".join(texts) if texts else None


def memory_tool(name: str) -> str | None:
    for prefix in GMEM_PREFIXES:
        if name.startswith(prefix) and name.removeprefix(prefix) in MEMORY_TOOLS:
            return name.removeprefix(prefix)
    return None


def describe(name: str, args: dict) -> str:
    for key in ("file_path", "path", "command", "pattern", "query"):
        if isinstance(args.get(key), str):
            return f"{name.rsplit('__', 1)[-1]}: {args[key][:100]}"
    return f"{name.rsplit('__', 1)[-1]}: {json.dumps(args, ensure_ascii=False)[:80]}"


def as_int(value: object) -> int | None:
    try:
        return int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError):
        return None


def observe(windows: list[Recall], name: str, args: dict, window: int) -> None:
    if not windows:
        return
    text = json.dumps(args, ensure_ascii=False)
    tool = memory_tool(name)
    revised_id = as_int(args.get("id")) if tool in REVISE_TOOLS else None
    for recall in windows:
        recall.calls += 1
        recall.actions.append(describe(name, args))
        if tool is None:
            for memory_id, paths in recall.cited.items():
                if any(path in text for path in paths):
                    recall.followed.add(memory_id)
        elif revised_id is not None and revised_id in {m["id"] for m in recall.memories}:
            recall.revised.add(revised_id)
    windows[:] = [recall for recall in windows if recall.calls < window]


def result_memories(content: object) -> list[dict] | None:
    if isinstance(content, str):
        text = content
    elif isinstance(content, list):
        text = "\n".join(block.get("text", "") for block in content if isinstance(block, dict))
    else:
        return None
    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return None
    memories = data.get("memories") if isinstance(data, dict) else None
    if not isinstance(memories, list):
        return None
    return [m for m in memories if isinstance(m, dict) and isinstance(m.get("id"), int)]


def scan(path: Path, project: str, session: str, window: int) -> tuple[list[Recall], int]:
    recalls: list[Recall] = []
    windows: list[Recall] = []
    uses: dict[str, tuple[str, dict]] = {}
    prompt = ""
    unparsed = 0
    for event in read_events(path):
        message = event.get("message")
        if not isinstance(message, dict):
            continue
        content = message.get("content")
        if event.get("type") == "user" and not event.get("isMeta"):
            text = human_text(content)
            if text is not None:
                prompt = text
                windows.clear()
                continue
        if not isinstance(content, list):
            continue
        for block in content:
            if not isinstance(block, dict):
                continue
            if block.get("type") == "tool_use":
                raw = block.get("input")
                args = raw if isinstance(raw, dict) else {}
                uses[block["id"]] = (block["name"], args)
                observe(windows, block["name"], args, window)
            elif block.get("type") == "tool_result":
                name, args = uses.get(str(block.get("tool_use_id")), ("", {}))
                if name not in RECALL_TOOLS:
                    continue
                memories = result_memories(block.get("content"))
                if memories is None:
                    unparsed += 1
                    continue
                cited = {
                    m["id"]: paths
                    for m in memories
                    if (paths := referenced_paths(str(m.get("content", ""))))
                }
                recall = Recall(
                    session=session,
                    project=project,
                    at=str(event.get("timestamp", "")),
                    prompt=prompt,
                    query=str(args.get("query", "")),
                    memories=memories,
                    cited=cited,
                )
                recalls.append(recall)
                windows.append(recall)
    return recalls, unparsed


def collect(root: Path, window: int) -> tuple[list[Recall], int]:
    recalls: list[Recall] = []
    unparsed = 0
    for path in sorted(root.rglob("*.jsonl")):
        relative = path.relative_to(root).parts
        if len(relative) < 2:
            continue
        found, failed = scan(path, relative[0], relative[1].removesuffix(".jsonl"), window)
        recalls.extend(found)
        unparsed += failed
    return recalls, unparsed


def percent(part: int, total: int) -> str:
    return f"{part * 100 / total:.0f}%" if total else "n/a"


def stored_memories(database: Path) -> dict[int, str] | None:
    if not database.exists():
        return None
    with sqlite3.connect(f"file:{database}?mode=ro", uri=True) as connection:
        return dict(connection.execute("SELECT id, memory_type FROM memories"))


def print_report(recalls: list[Recall], unparsed: int, stored: dict[int, str] | None) -> None:
    nonempty = [recall for recall in recalls if recall.memories]
    verifiable = [recall for recall in nonempty if recall.cited]
    followed = [recall for recall in verifiable if recall.followed]
    top_verifiable = [r for r in verifiable if r.memories[0]["id"] in r.cited]
    top_followed = [r for r in top_verifiable if r.memories[0]["id"] in r.followed]
    cited_total = sum(len(recall.cited) for recall in verifiable)
    followed_total = sum(len(recall.followed) for recall in verifiable)
    revised = [recall for recall in nonempty if recall.revised]

    empty = len(recalls) - len(nonempty)
    print(f"recalls: {len(recalls)} ({unparsed} unparsed)")
    print(f"  empty: {empty} ({percent(empty, len(recalls))})")
    print(f"  verifiable (a returned memory cites a path): {len(verifiable)} of {len(nonempty)}")
    rate = percent(len(followed), len(verifiable))
    print(f"  followed (a cited path was touched afterwards): {len(followed)} ({rate})")
    rate = percent(len(top_followed), len(top_verifiable))
    print(f"  top-1 followed: {len(top_followed)} of {len(top_verifiable)} ({rate})")
    rate = percent(followed_total, cited_total)
    print(f"  memories followed: {followed_total} of {cited_total} cited ({rate})")
    print(f"  revised after recall (update or forget): {len(revised)}")

    per_project: dict[str, list[int]] = defaultdict(lambda: [0, 0, 0, 0])
    for recall in recalls:
        row = per_project[recall.project]
        row[0] += 1
        row[1] += not recall.memories
        row[2] += bool(recall.cited)
        row[3] += bool(recall.followed)
    print(f"\n{'project':<48}{'recalls':>8}{'empty':>7}{'verif.':>8}{'follow.':>9}")
    for project, row in sorted(per_project.items(), key=lambda item: -item[1][0])[:8]:
        print(f"{project[-47:]:<48}{row[0]:>8}{row[1]:>7}{row[2]:>8}{row[3]:>9}")

    returned: Counter[int] = Counter()
    sessions: dict[int, set[str]] = defaultdict(set)
    verified: Counter[int] = Counter()
    hits: Counter[int] = Counter()
    summaries: dict[int, str] = {}
    for recall in nonempty:
        for memory in recall.memories:
            memory_id = memory["id"]
            returned[memory_id] += 1
            sessions[memory_id].add(recall.session)
            verified[memory_id] += memory_id in recall.cited
            hits[memory_id] += memory_id in recall.followed
            summaries[memory_id] = summary(str(memory.get("content", "")))
    noisy = [
        memory_id
        for memory_id in returned
        if verified[memory_id] >= NOISY_RETURNS and hits[memory_id] == 0
    ]
    noisy.sort(key=lambda memory_id: -verified[memory_id])
    print(f"\nnoisy memories (cite paths, {NOISY_RETURNS}+ returns, never followed): {len(noisy)}")
    for memory_id in noisy[:10]:
        count, spread = verified[memory_id], len(sessions[memory_id])
        print(f"  #{memory_id} x{count} in {spread} sessions  {summaries[memory_id][:90]}")

    print(f"\nmemories returned at least once: {len(returned)}")
    if stored is not None:
        never = Counter(kind for memory_id, kind in stored.items() if memory_id not in returned)
        print(f"never returned in any transcript: {sum(never.values())} of {len(stored)}")
        for kind, count in never.most_common():
            print(f"  {kind:<16}{count:>4} of {sum(1 for k in stored.values() if k == kind)}")


def write_sample(recalls: list[Recall], count: int, seed: int, output: Path) -> int:
    nonempty = [recall for recall in recalls if recall.memories]
    chosen = random.Random(seed).sample(nonempty, min(count, len(nonempty)))
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("w", encoding="utf-8") as handle:
        for recall in chosen:
            row = {
                "session": recall.session,
                "project": recall.project,
                "at": recall.at,
                "prompt": recall.prompt[:300],
                "query": recall.query,
                "results": [
                    {
                        "id": memory["id"],
                        "type": memory.get("memory_type"),
                        "score": memory.get("score"),
                        "summary": summary(str(memory.get("content", ""))),
                        "cited": recall.cited.get(memory["id"], []),
                        "followed": memory["id"] in recall.followed,
                    }
                    for memory in recall.memories[:3]
                ],
                "actions": recall.actions[:12],
                "revised": sorted(recall.revised),
            }
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    return len(chosen)


def main() -> None:
    home = Path(os.environ.get("GRAPHMEM_HOME", Path.home() / ".graphmem"))
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    parser.add_argument("--transcripts", type=Path, default=Path.home() / ".claude" / "projects")
    parser.add_argument("--db", type=Path, default=home / "memory.sqlite")
    parser.add_argument("--window", type=int, default=25, help="tool calls observed per recall")
    parser.add_argument("--sample", type=int, default=30, help="recalls to export for review")
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument(
        "--sample-out",
        type=Path,
        default=Path(__file__).resolve().parent.parent / ".data" / "memory_usage" / "sample.jsonl",
    )
    args = parser.parse_args()

    recalls, unparsed = collect(args.transcripts, args.window)
    print_report(recalls, unparsed, stored_memories(args.db))
    if args.sample:
        written = write_sample(recalls, args.sample, args.seed, args.sample_out)
        print(f"\nsample: {written} recalls written to {args.sample_out}")


if __name__ == "__main__":
    main()
