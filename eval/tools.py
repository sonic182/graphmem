"""Shell and file-read tools shared by both benchmark variants.

The tools are intentionally small and identical for the control and gmem
variants. Each tool records what the agent actually ingested (output
characters and source lines) into a per-run :class:`RunMetrics`, and every
result is truncated the way a real coding harness truncates tool output.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass, field
from pathlib import Path

from strands import tool

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
    head = max_chars // 2
    tail = max_chars - head
    removed = len(text) - max_chars
    return f"{text[:head]}\n…{removed} characters truncated…\n{text[-tail:]}"


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
            metrics.record("shell", result)
            return result
        body = completed.stdout
        if completed.stderr:
            body += ("\n" if body else "") + completed.stderr
        result = truncate(f"{body}\n[exit {completed.returncode}]")
        metrics.record("shell", result)
        metrics.source_lines_read += result.count("\n")
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
            metrics.record("read_file", result)
            return result
        try:
            lines = target.read_text(errors="replace").splitlines()
        except OSError as error:
            result = f"error: cannot read {path}: {error}"
            metrics.record("read_file", result)
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
        metrics.record("read_file", result)
        metrics.source_lines_read += len(selected)
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
            metrics.record("skill", result)
            return result
        result = truncate(target.read_text(errors="replace"))
        metrics.record("skill", result)
        return result

    return skill
