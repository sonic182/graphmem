"""Offline fixtures at the model transport and run-record boundaries."""

import copy
import json
from argparse import Namespace
from collections.abc import AsyncIterator, Callable
from pathlib import Path
from typing import Any

import pytest
from strands.models.litellm import LiteLLMModel

from budget import MatrixBudget
from harness import DEFAULT_MODEL, Record
from pricing import load_pricing


class ScriptedModel(LiteLLMModel):
    """Replay provider stream events through the real Strands agent loop."""

    def __init__(self, turns: list[list[dict] | Exception]) -> None:
        super().__init__(model_id=DEFAULT_MODEL)
        self.turns = iter(turns)
        self.requests: list[list[dict]] = []

    async def count_tokens(self, *args: Any, **kwargs: Any) -> int:
        # Match the independent input usage returned by this fake transport.
        return 100

    async def stream(
        self,
        messages: list[dict],
        tool_specs: list[dict] | None = None,
        system_prompt: str | None = None,
        **kwargs: Any,
    ) -> AsyncIterator[dict]:
        self.requests.append(copy.deepcopy(messages))
        turn = next(self.turns)
        if isinstance(turn, Exception):
            raise turn
        for event in turn:
            yield event


def model_turn(
    text: str,
    *,
    stop_reason: str = "end_turn",
    tool_name: str | None = None,
    tool_input: dict | None = None,
) -> list[dict]:
    """Build a provider response with independently fixed usage counters."""
    events = [
        {"messageStart": {"role": "assistant"}},
        {"contentBlockDelta": {"contentBlockIndex": 0, "delta": {"text": text}}},
        {"contentBlockStop": {"contentBlockIndex": 0}},
    ]
    if tool_name is not None:
        events += [
            {
                "contentBlockStart": {
                    "contentBlockIndex": 1,
                    "start": {"toolUse": {"toolUseId": "request-1", "name": tool_name}},
                }
            },
            {
                "contentBlockDelta": {
                    "contentBlockIndex": 1,
                    "delta": {"toolUse": {"input": json.dumps(tool_input or {})}},
                }
            },
            {"contentBlockStop": {"contentBlockIndex": 1}},
        ]
    return [
        *events,
        {"messageStop": {"stopReason": stop_reason}},
        {
            "metadata": {
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 20,
                    "totalTokens": 120,
                    "cacheReadInputTokens": 40,
                },
                "metrics": {"latencyMs": 1},
            }
        },
    ]


@pytest.fixture
def eval_args(tmp_path: Path) -> Namespace:
    """Use a disposable checkout without provider credentials or a gmem process."""
    return Namespace(
        repo=tmp_path,
        model=DEFAULT_MODEL,
        max_tokens=8192,
        temperature=0.0,
        jobs=1,
        system_prompt="Answer the task.",
        guidance_text="Use the code tools.",
        gmem=tmp_path / "gmem",
        gmem_home=tmp_path / "index",
        gmem_max_files=100,
        skills_dir=tmp_path / "skills",
        matrix_budget=MatrixBudget(1.15, load_pricing()[DEFAULT_MODEL]),
        run_token_limits={"imports-001": 17099},
    )


@pytest.fixture
def record_factory() -> Callable[..., Record]:
    """Create records with explicit, independent usage for report tests."""

    def make_record(**changes: Any) -> Record:
        fields = {
            "variant": "control",
            "task": "imports-001",
            "run": 1,
            "score": 1.0,
            "correct": True,
            "input_tokens": 100,
            "output_tokens": 20,
            "total_tokens": 120,
            "cache_read_tokens": 40,
            "cycles": 1,
            "wall_ms": 1,
            "tool_calls": 0,
            "model": DEFAULT_MODEL,
        }
        return Record(**(fields | changes))

    return make_record
