# Memory usage in real sessions

`scripts/memory_usage.py` answers one question about the memory store: when an
agent recalls memories, does it go on to use them? It complements
[`gmem health`](../cli.md#gmem-health), which only sees what gmem served, and
the retrieval benchmarks in [results.md](results.md), which score ranking on
public datasets rather than on your own sessions.

```sh
python3 scripts/memory_usage.py                 # all transcripts, 30-recall sample
python3 scripts/memory_usage.py --window 15     # watch fewer calls after each recall
python3 scripts/memory_usage.py --sample 0      # report only
```

It reads Claude Code transcripts (`~/.claude/projects` by default) and the store
(`$GRAPHMEM_HOME/memory.sqlite`, read-only). Nothing is sent anywhere. The sample
is written to `.data/memory_usage/sample.jsonl`, which git ignores; it contains
your prompts, queries and memory text, so do not commit or share it as is.

## What it measures

Every recall is paired with the tool calls after it, until the next user prompt
or `--window` calls (25 by default). A returned memory counts as **followed**
when one of those calls mentions a path from its `**Where**` section. The path
rules are the ones `gmem health` uses to flag missing paths.

- **verifiable**: a returned memory cites at least one path. The rates below
  use only these recalls, because a memory without a path cannot be checked.
- **followed / top-1 followed / memories followed**: how often the agent touched
  a cited path, for any returned memory, for the first result, and per memory.
- **revised after recall**: an `update` or `forget` of a returned memory in the
  same window, which usually means it was outdated or incomplete.
- **noisy memories**: cite paths, were returned five or more times, and were
  never followed.
- **never returned**: stored memories that no transcript ever received.

## How to read it

Following a path is evidence that a recall was on topic, not proof that it was
useful, and the reverse does not hold either. A memory whose value is knowing
what *not* to touch, such as a retired tool, is never followed and shows up as
noisy. Treat the rates as a lower bound, and use the sample to judge individual
recalls: each row has the prompt, the query, the top three results with their
cited paths, and the actions that followed.

Each transcript file, including those of subagents, is paired on its own: a
recall is never matched with calls made in another file. Only Claude Code
transcripts are supported.
