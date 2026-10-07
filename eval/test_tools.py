"""Check delivered tool output through real Strands tool execution."""

from argparse import Namespace
from pathlib import Path
from unittest.mock import AsyncMock, Mock

import pytest
from mcp.types import Tool
from strands.tools.mcp import MCPClient
from strands.tools.mcp.mcp_agent_tool import MCPAgentTool

import harness
from budget import RunBudget
from conftest import ScriptedModel, model_turn
from tools import MAX_CHARS, RunMetrics


@pytest.mark.parametrize("status", ["success", "error"])
def test_mcp_output_has_one_cap_before_the_next_model_call(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, status: str
) -> None:
    client = Mock(spec=MCPClient)
    client.call_tool_async = AsyncMock(
        return_value={
            "toolUseId": "request-1",
            "status": status,
            "content": [{"text": "HEAD" + "a" * MAX_CHARS}, {"text": "b" * MAX_CHARS + "TAIL"}],
        }
    )
    client.list_tools_sync.return_value = [
        MCPAgentTool(
            Tool(name="code_outline", inputSchema={"type": "object", "properties": {}}), client
        )
    ]
    monkeypatch.setattr(harness, "MCPClient", lambda *args, **kwargs: client)
    model = ScriptedModel(
        [
            model_turn("Find imports.", stop_reason="tool_use", tool_name="code_outline"),
            model_turn('["real"]'),
        ]
    )
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: model)

    record = harness.run_once(
        eval_args,
        {"id": "imports-001", "prompt": "Find imports."},
        "gmem",
        1,
        {"imports-001": {"specifiers": ["real"]}},
    )

    assert record.error is None
    delivered = next(
        block["toolResult"]
        for message in model.requests[1]
        for block in message["content"]
        if "toolResult" in block
    )
    text = "".join(part["text"] for part in delivered["content"])
    assert delivered["status"] == status
    assert text.startswith("HEAD")
    assert text.endswith("TAIL")
    assert "characters truncated" in text
    assert len(text) == MAX_CHARS
    assert record.tool_output_chars == MAX_CHARS
    assert record.source_lines_read == 0


def test_file_metrics_count_only_visible_numbered_lines(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    (eval_args.repo / "large.txt").write_text("\n".join("x" * 1000 for _ in range(2000)))
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: ScriptedModel([]))
    metrics = RunMetrics()
    budget = RunBudget(eval_args.matrix_budget, 17099, eval_args.max_tokens)
    agent, _ = harness.build_agent(eval_args, eval_args.repo, "control", metrics, budget)
    try:
        result = agent.tool.read_file(path="large.txt")
    finally:
        agent.shutdown()
    text = result["content"][0]["text"]
    visible_lines = [
        line
        for line in text.splitlines()
        if "\t" in line and line.partition("\t")[0].strip().isdigit()
    ]

    assert metrics.source_lines_read == len(visible_lines)
    assert len(text) == MAX_CHARS
    assert 0 < len(visible_lines) < 2000
    assert metrics.tool_output_chars == len(text)
    assert metrics.file_reads == 1


def test_file_tool_rejects_a_symlink_outside_the_repository(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace, tmp_path: Path
) -> None:
    outside = tmp_path.parent / f"{tmp_path.name}-outside.txt"
    outside.write_text("not repository content")
    (eval_args.repo / "link.txt").symlink_to(outside)
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: ScriptedModel([]))
    metrics = RunMetrics()
    budget = RunBudget(eval_args.matrix_budget, 17099, eval_args.max_tokens)
    agent, _ = harness.build_agent(eval_args, eval_args.repo, "control", metrics, budget)
    try:
        result = agent.tool.read_file(path="link.txt")
    finally:
        agent.shutdown()
        outside.unlink()

    assert "outside the repository" in result["content"][0]["text"]
    assert "not repository content" not in result["content"][0]["text"]
    assert metrics.source_lines_read == 0


def test_shell_metrics_exclude_exit_metadata(
    monkeypatch: pytest.MonkeyPatch, eval_args: Namespace
) -> None:
    monkeypatch.setattr(harness, "LiteLLMModel", lambda **kwargs: ScriptedModel([]))
    metrics = RunMetrics()
    budget = RunBudget(eval_args.matrix_budget, 17099, eval_args.max_tokens)
    agent, _ = harness.build_agent(eval_args, eval_args.repo, "control", metrics, budget)
    try:
        result = agent.tool.shell(command="printf 'one\\ntwo\\n'")
    finally:
        agent.shutdown()

    assert result["content"][0]["text"] == "one\ntwo\n[exit 0]"
    assert metrics.source_lines_read == 2
    assert metrics.tool_output_chars == len(result["content"][0]["text"])
