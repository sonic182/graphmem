#!/usr/bin/env python3
"""Minimal JSON-RPC stdio client for smoke-testing the gmem MCP server.

Speaks the same newline-delimited JSON-RPC 2.0 gmem uses (see tests/mcp.rs):
one JSON object per line on stdin/stdout, responses matched by request id.
No third-party dependencies.

Examples:
  mcp_smoke.py                          full remember/recall/inspect/forget smoke test
  mcp_smoke.py --embeddings             same, but exercises the real embedding model
  mcp_smoke.py -v                       print every request/response
  mcp_smoke.py call recall '{"query":"x","use_embeddings":false}'
  mcp_smoke.py raw tools/list '{}'
  mcp_smoke.py --home ~/.graphmem-dev call stats '{}'
"""

import argparse
import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_BIN_CANDIDATES = [
    REPO_ROOT / "target" / "debug" / "gmem",
    REPO_ROOT / "target" / "release" / "gmem",
]
CLIENT_INFO = {
    "protocolVersion": "2025-06-18",
    "capabilities": {},
    "clientInfo": {"name": "mcp_smoke", "version": "1"},
}


class McpError(RuntimeError):
    pass


class Mcp:
    def __init__(self, binary, home, timeout=30.0):
        self.timeout = timeout
        self._id = 0
        self.proc = subprocess.Popen(
            [str(binary), "mcp"],
            cwd=home,
            env={**os.environ, "GRAPHMEM_HOME": str(home)},
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def request(self, method, params=None):
        self._id += 1
        req_id = self._id
        payload = {"jsonrpc": "2.0", "id": req_id, "method": method, "params": params or {}}
        stdin = self.proc.stdin
        assert stdin is not None
        stdin.write(json.dumps(payload) + "\n")
        stdin.flush()
        while True:
            response = json.loads(self._read_line())
            if response.get("id") == req_id:
                return response
            # a notification or a response to an id we're not waiting on: ignore and keep reading

    def _read_line(self):
        # ponytail: assumes each response arrives as one flushed line, true for
        # this local subprocess pipe; a partial-line reader would be needed for
        # a flakier transport.
        deadline = time.monotonic() + self.timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise McpError(f"no response within {self.timeout}s\nstderr:\n{self._drain_stderr()}")
            stdout = self.proc.stdout
            assert stdout is not None
            ready, _, _ = select.select([stdout], [], [], remaining)
            if not ready:
                continue
            line = stdout.readline()
            if line == "":
                code = self.proc.poll()
                raise McpError(f"server closed stdout (exit={code})\nstderr:\n{self._drain_stderr()}")
            return line

    def _drain_stderr(self):
        if self.proc.stderr is None:
            return ""
        os.set_blocking(self.proc.stderr.fileno(), False)
        try:
            return self.proc.stderr.read() or ""
        except (BlockingIOError, TypeError):
            return ""

    def close(self):
        if self.proc.stdin:
            self.proc.stdin.close()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def resolve_binary(explicit):
    if explicit:
        return Path(explicit)
    for candidate in DEFAULT_BIN_CANDIDATES:
        if candidate.is_file():
            return candidate
    found = shutil.which("gmem")
    if found:
        return Path(found)
    raise SystemExit(
        "no gmem binary found; run `cargo build` (or `cargo build --release`) "
        f"from {REPO_ROOT}, or pass --bin /path/to/gmem"
    )


def make_home(explicit, embeddings):
    if explicit:
        home = Path(explicit).expanduser()
        home.mkdir(parents=True, exist_ok=True)
        return home, False
    home = Path(tempfile.mkdtemp(prefix="gmem-mcp-smoke-"))
    if not embeddings:
        (home / "config.toml").write_text("[embedding]\nenabled = false\n")
    return home, True


def dump(label, response):
    print(f"--- {label} ---")
    print(json.dumps(response, indent=2))


def run_smoke(mcp, verbose):
    initialized = mcp.request("initialize", CLIENT_INFO)
    assert initialized["result"]["serverInfo"]["name"] == "gmem", initialized
    if verbose:
        dump("initialize", initialized)

    tools = mcp.request("tools/list", {})
    names = sorted(tool["name"] for tool in tools["result"]["tools"])
    assert names == ["forget", "graph", "inspect", "recall", "relate", "remember", "stats"], names
    if verbose:
        dump("tools/list", tools)

    remembered = mcp.request(
        "tools/call",
        {"name": "remember", "arguments": {"content": "mcp_smoke test memory", "memory_type": "observation"}},
    )
    memory_id = remembered["result"]["structuredContent"]["id"]
    if verbose:
        dump("remember", remembered)

    recalled = mcp.request(
        "tools/call",
        {"name": "recall", "arguments": {"query": "mcp_smoke", "use_embeddings": False}},
    )
    recalled_ids = [memory["id"] for memory in recalled["result"]["structuredContent"]["memories"]]
    assert memory_id in recalled_ids, recalled
    if verbose:
        dump("recall", recalled)

    inspected = mcp.request("tools/call", {"name": "inspect", "arguments": {"id": memory_id}})
    assert inspected["result"]["structuredContent"]["content"] == "mcp_smoke test memory", inspected
    if verbose:
        dump("inspect", inspected)

    forgotten = mcp.request("tools/call", {"name": "forget", "arguments": {"id": memory_id}})
    assert forgotten["result"]["structuredContent"]["forgotten"] is True, forgotten
    if verbose:
        dump("forget", forgotten)

    print(f"OK: initialize, tools/list ({len(names)} tools), remember, recall, inspect, forget (memory id {memory_id})")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin", help="path to the gmem binary (default: target/{debug,release}/gmem or PATH)")
    parser.add_argument("--home", help="GRAPHMEM_HOME to use (default: a fresh temp dir, removed after unless --keep)")
    parser.add_argument("--keep", action="store_true", help="keep the temp GRAPHMEM_HOME instead of deleting it")
    parser.add_argument(
        "--embeddings", action="store_true", help="leave embeddings enabled (downloads the model on first recall)"
    )
    parser.add_argument("--timeout", type=float, default=30.0, help="seconds to wait for a response (default: 30)")
    parser.add_argument("-v", "--verbose", action="store_true", help="print every request/response")
    sub = parser.add_subparsers(dest="command")

    call_parser = sub.add_parser("call", help="send one tools/call after initializing")
    call_parser.add_argument("tool")
    call_parser.add_argument("arguments", nargs="?", default="{}", help="JSON object of tool arguments")

    raw_parser = sub.add_parser("raw", help="send one raw JSON-RPC method after initializing")
    raw_parser.add_argument("method")
    raw_parser.add_argument("params", nargs="?", default="{}", help="JSON object of params")

    args = parser.parse_args()

    binary = resolve_binary(args.bin)
    home, is_temp = make_home(args.home, args.embeddings)
    mcp = Mcp(binary, home, timeout=args.timeout)
    try:
        if args.command is None:
            run_smoke(mcp, args.verbose)
        else:
            init = mcp.request("initialize", CLIENT_INFO)
            if args.verbose:
                dump("initialize", init)
            if args.command == "call":
                response = mcp.request("tools/call", {"name": args.tool, "arguments": json.loads(args.arguments)})
            else:
                response = mcp.request(args.method, json.loads(args.params))
            dump(args.command, response)
    except McpError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except AssertionError as error:
        print(f"FAIL: unexpected response: {error}", file=sys.stderr)
        sys.exit(1)
    finally:
        mcp.close()
        if is_temp and not args.keep:
            shutil.rmtree(home, ignore_errors=True)
        elif is_temp:
            print(f"GRAPHMEM_HOME kept at {home}")


if __name__ == "__main__":
    main()
