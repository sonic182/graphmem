import json
import tempfile
import unittest
from pathlib import Path

import memory_usage

RECALL = "mcp__plugin_graphmem_gmem__recall"
UPDATE = "mcp__plugin_graphmem_gmem__update"


def tool_use(identifier: str, name: str, args: dict) -> dict:
    block = {"type": "tool_use", "id": identifier, "name": name, "input": args}
    return {"type": "assistant", "message": {"content": [block]}}


def tool_result(identifier: str, payload: object) -> dict:
    block = {"type": "tool_result", "tool_use_id": identifier, "content": json.dumps(payload)}
    return {"type": "user", "message": {"content": [block]}}


def prompt(text: str) -> dict:
    return {"type": "user", "message": {"content": text}}


def memory(identifier: int, where: str) -> dict:
    return {
        "id": identifier,
        "memory_type": "decision",
        "score": 0.5,
        "content": f"**What**: memory {identifier}\n**Where**: {where}\n**Learned**: none",
    }


class MemoryUsageTest(unittest.TestCase):
    def test_referenced_paths_matches_the_rust_implementation(self):
        content = (
            "**What**: ignored/in/what.rs\n**Where**: `lib/a/b.ex`, lib/tools/{list,set}.ex, "
            "scripts/gmem.js:12, docs/, Foo.update/2, MCP.Tools.Update, "
            "~/.pi/agent/settings.json, results/<name>/, release.yml.\n"
            "**Learned**: later/path.rs"
        )
        self.assertEqual(
            memory_usage.referenced_paths(content),
            [
                "lib/a/b.ex",
                "lib/tools/list.ex",
                "lib/tools/set.ex",
                "scripts/gmem.js",
                "docs/",
                "release.yml",
            ],
        )
        self.assertEqual(memory_usage.referenced_paths("**What**: at src/x.rs"), [])

    def test_scan_pairs_each_recall_with_the_calls_that_follow_it(self):
        events = [
            prompt("fix the parser"),
            tool_use("r1", RECALL, {"query": "parser"}),
            tool_result("r1", {"memories": [memory(1, "src/a.rs"), memory(2, "src/b.rs")]}),
            tool_use("c1", "Read", {"file_path": "/repo/src/a.rs"}),
            tool_use("c2", UPDATE, {"id": 2, "content": "revised"}),
            prompt("next task"),
            tool_use("r2", RECALL, {"query": "nothing"}),
            tool_result("r2", {"memories": []}),
            tool_use("c3", "Read", {"file_path": "/repo/src/b.rs"}),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "project").mkdir()
            transcript = root / "project" / "session.jsonl"
            transcript.write_text("\n".join(json.dumps(event) for event in events))

            recalls, unparsed = memory_usage.collect(root, window=25)

        self.assertEqual(unparsed, 0)
        first, second = recalls
        self.assertEqual(
            (first.project, first.session, first.prompt), ("project", "session", "fix the parser")
        )
        self.assertEqual(first.query, "parser")
        self.assertEqual(first.followed, {1})
        self.assertEqual(first.revised, {2})
        self.assertEqual(second.memories, [])
        self.assertEqual(second.followed, set())


if __name__ == "__main__":
    unittest.main()
