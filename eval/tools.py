"""Shell and file-read tools shared by both benchmark variants.

The tools are intentionally small and identical for the control and gmem
variants. Each tool records what the agent actually ingested (output
characters and source lines) into a per-run :class:`RunMetrics`, and every
result is truncated the way a real coding harness truncates tool output.
"""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

from strands import tool
from strands.hooks import AfterToolCallEvent, HookProvider, HookRegistry

MAX_CHARS = 50_000
MAX_LINES = 2_000


@dataclass
class RunMetrics:
    """What the agent read during one run, independent of the model."""

    tool_output_chars: int = 0
    source_lines_read: int = 0
    shell_calls: int = 0
    file_reads: int = 0
    tool_chars: dict[str, int] = field(default_factory=dict)

    def record(self, name: str, text: str) -> None:
        self.tool_output_chars += len(text)
        self.tool_chars[name] = self.tool_chars.get(name, 0) + len(text)


def truncate(text: str, max_chars: int = MAX_CHARS) -> str:
    """Keep the head and tail of long output, like a real agent harness."""
    if len(text) <= max_chars:
        return text
    marker = "\n…characters truncated…\n"
    if max_chars <= len(marker):
        return text[:max_chars]
    available = max_chars - len(marker)
    head = (available + 1) // 2
    tail = available - head
    return text[:head] + marker + (text[-tail:] if tail else "")


class ToolOutputHook(HookProvider):
    """Cap and measure all delivered tool text, including MCP results."""

    def __init__(self, metrics: RunMetrics) -> None:
        self.metrics = metrics

    def register_hooks(self, registry: HookRegistry, **kwargs: object) -> None:
        registry.add_callback(AfterToolCallEvent, self.after_tool_call)

    def after_tool_call(self, event: AfterToolCallEvent) -> None:
        content = event.result.get("content", [])
        text_blocks = [part["text"] for part in content if "text" in part]
        if not text_blocks:
            return
        text = truncate("\n".join(text_blocks))
        # Apply one cap to the entire result, not independently to each
        # MCP block. Preserve non-text content and result status/identity.
        event.result["content"] = [
            {"text": text},
            *(part for part in content if "text" not in part),
        ]
        name = event.tool_use["name"]
        self.metrics.record(name, text)
        if name == "read_file":
            self.metrics.source_lines_read += sum(
                bool(re.match(r"^\s*\d+\t", line)) for line in text.splitlines()
            )
        elif name == "shell" and not text.startswith("command timed out after"):
            self.metrics.source_lines_read += sum(
                not line.startswith(("[exit ", "…characters truncated…"))
                for line in text.splitlines()
            )


def make_tools(repo: Path, metrics: RunMetrics) -> list:
    """Build the two shared tools bound to ``repo`` and ``metrics``."""

    @tool
    def shell(command: str, timeout_seconds: int = 120) -> str:
        """Run a shell command from the repository root and return its output.

        Use this for search and history: `rg`, `grep`, `find`, `sed`, `git
        diff`, `git show`, `git log`, and similar. The combined stdout and
        stderr are returned, followed by the exit code. Long output is
        truncated.

        Args:
            command: The shell command to run.
            timeout_seconds: Kill the command after this many seconds.
        """
        metrics.shell_calls += 1
        try:
            completed = subprocess.run(
                ["bash", "-lc", command],
                cwd=repo,
                capture_output=True,
                text=True,
                errors="replace",
                timeout=timeout_seconds,
            )
        except subprocess.TimeoutExpired:
            result = f"command timed out after {timeout_seconds}s"
            return result
        body = completed.stdout
        if completed.stderr:
            body += ("\n" if body else "") + completed.stderr
        separator = "" if not body or body.endswith("\n") else "\n"
        result = truncate(f"{body}{separator}[exit {completed.returncode}]")
        return result

    @tool
    def read_file(path: str, start_line: int = 1, end_line: int = 0) -> str:
        """Read a file (or a line range) from the repository, with line numbers.

        Args:
            path: File path relative to the repository root.
            start_line: First line to return (1-based).
            end_line: Last line to return, or 0 for the end of the file.
        """
        metrics.file_reads += 1
        target = (repo / path).resolve()
        if repo.resolve() not in target.parents and target != repo.resolve():
            result = f"error: {path} is outside the repository"
            return result
        try:
            lines = target.read_text(errors="replace").splitlines()
        except OSError as error:
            result = f"error: cannot read {path}: {error}"
            return result
        start = max(1, start_line)
        end = len(lines) if end_line <= 0 else min(end_line, len(lines))
        selected = lines[start - 1 : end]
        truncated_note = ""
        if len(selected) > MAX_LINES:
            selected = selected[:MAX_LINES]
            truncated_note = f"\n…truncated at {MAX_LINES} lines…"
        numbered = "\n".join(f"{start + offset:>6}\t{line}" for offset, line in enumerate(selected))
        result = truncate(numbered + truncated_note)
        return result

    return [shell, read_file]


def make_skill_tool(skills_dir: Path, metrics: RunMetrics):
    """Build a tool that loads a skill's markdown on demand.

    This mirrors the Pi plugin's skill discovery: the model sees the skill
    names up front and pulls the full instructions only when it wants them.
    """

    @tool
    def skill(name: str) -> str:
        """Load the full instructions of a Graphmem skill by name.

        Available skills are `graphmem-code-analysis` (which gmem code tool to
        use for a task and how to read its output) and `graphmem-mcp-for-dev`
        (the memory contract).

        Args:
            name: The skill directory name, for example "graphmem-code-analysis".
        """
        available = sorted(
            path.name for path in skills_dir.iterdir() if (path / "SKILL.md").is_file()
        )
        target = skills_dir / name / "SKILL.md"
        if not target.is_file():
            result = f"error: no skill {name!r}; available: {', '.join(available)}"
            return result
        result = truncate(target.read_text(errors="replace"))
        return result

    return skill
