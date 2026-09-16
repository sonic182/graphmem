#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Compare gmem recall quality with and without embeddings on multi-hop QA.

Drives `gmem mcp` over stdio. Each question's candidate paragraphs are stored
as memories under their own `repo:/eval/<dataset>/<n>` scope. Recall's scope
filter runs before ranking, so each question only ranks its own paragraphs,
the same "distractor" setting the datasets define. Each (dataset, graph) pair
starts from a flushed store and is ingested once by a server with embeddings
off (fast inserts). `gmem reembed` then embeds everything (in batches when
`--batch-size` is given or gmem runs on CUDA), and a fresh server answers
every mode, so all modes rank the same memories.

Modes:
  embeddings  recall with use_embeddings=true (semantic seeds + PPR)
  fts-raw     use_embeddings=false with the raw question. FTS5 ANDs every
              whitespace term, so this is what an agent gets today
  fts-or      use_embeddings=false with the question's words OR-joined, a
              fair BM25 baseline
FTS modes ignore the graph, so they only run with `--graphs none`.

Graphs (entities/relations attached at `remember` time, no LLM involved):
  none      plain memories
  mentions  one entity per paragraph title, plus `mentions` edges when a
            paragraph's text names another candidate's title. Every paragraph,
            supporting or distractor, gets the same treatment: a fair graph
  oracle    the dataset's own reasoning triples (2Wiki `evidences`, MuSiQue
            `question_decomposition`). They only describe the gold path, so
            this LEAKS the answer: read it as a ceiling, not a result.
            HotpotQA has no triples and skips it

Metrics (averaged over questions): recall@k = share of supporting paragraphs
in the top k, MRR = 1/rank of the first supporting paragraph.

Setup, from the repo root (stdlib only; no dependencies):
  uv run scripts/download_eval_datasets.py
  cargo build --release
  uv run scripts/eval_retrieval.py --questions 100
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DATA_DIR = REPO_ROOT / ".data" / "eval"
MODEL_CACHE_DIR = REPO_ROOT / ".data" / "models"
sys.path.insert(0, str(REPO_ROOT / ".agents" / "skills" / "gmem-development" / "scripts"))
from mcp_smoke import CLIENT_INFO, Mcp, resolve_binary  # noqa: E402

DATASET_FILES = {
    "hotpotqa": "hotpotqa_distractor_validation.jsonl",
    "2wikimultihopqa": "2wikimultihopqa_validation.jsonl",
    "musique": "musique_validation.jsonl",
}
MODES = {
    "embeddings": (True, lambda question: question),
    "fts-raw": (False, lambda question: question),
    "fts-or": (False, lambda question: " OR ".join(re.findall(r"\w+", question))),
}
GRAPHS = ("none", "mentions", "oracle")
KS = (2, 5, 10)


def log(message: str) -> None:
    """Progress goes to stderr so stdout stays a clean results table."""
    print(f"{time.strftime('%H:%M:%S')} {message}", file=sys.stderr, flush=True)


def progress_step(total: int) -> int:
    """Log roughly ten progress lines per loop."""
    return max(1, total // 10)


def load_examples(dataset: str, limit: int) -> list[dict]:
    """Return up to `limit` examples as {question, paragraphs, gold} dicts."""
    examples = []
    with (DATA_DIR / DATASET_FILES[dataset]).open() as lines:
        for line in lines:
            row = json.loads(line)
            example = parse_row(dataset, row)
            if example["gold"]:
                examples.append(example)
            if len(examples) == limit:
                break
    return examples


def parse_row(dataset: str, row: dict) -> dict:
    """Normalize one dataset row.

    `gold` holds indices into `titles`/`texts`; `triples` holds
    (subject, relation, object, paragraph index) for the oracle graph.
    """
    if dataset == "musique":
        titles = [p["title"] for p in row["paragraphs"]]
        texts = [p["paragraph_text"] for p in row["paragraphs"]]
        gold = {i for i, p in enumerate(row["paragraphs"]) if p["is_supporting"]}
        triples, answers = [], []
        for step in row["question_decomposition"]:
            # "#1 >> spouse" refers to the first step's answer.
            question = re.sub(r"#(\d+)", lambda m: answers[int(m.group(1)) - 1], step["question"])
            answers.append(step["answer"])
            # ponytail: steps phrased as plain questions (no ">>") have no triple; skipped
            if " >> " in question and step["paragraph_support_idx"] is not None:
                subject, relation = question.split(" >> ", 1)
                triples.append((subject, relation, step["answer"], step["paragraph_support_idx"]))
        return {"question": row["question"], "titles": titles, "texts": texts, "gold": gold, "triples": triples}

    # hotpotqa (HF) stores columns as dicts of lists, 2wiki keeps the original
    # list of [title, sentences] pairs.
    context, facts = row["context"], row["supporting_facts"]
    if isinstance(context, dict):
        context = list(zip(context["title"], context["sentences"]))
        facts = list(zip(facts["title"], facts["sent_id"]))
    titles = [title for title, _ in context]
    texts = ["".join(sentences) for _, sentences in context]
    supporting = {title for title, _ in facts}
    gold = {i for i, title in enumerate(titles) if title in supporting}
    # 2Wiki evidence names the page without its "(film)"-style suffix.
    triples = [
        (subject, relation, obj, i)
        for subject, relation, obj in row.get("evidences", [])
        for i, title in enumerate(titles)
        if i in gold and base_title(title) in (subject, obj)
    ]
    return {"question": row["question"], "titles": titles, "texts": texts, "gold": gold, "triples": triples}


def base_title(title: str) -> str:
    return re.sub(r"\s*\([^)]*\)$", "", title)


def entity(name: str) -> dict:
    return {"kind": "entity", "name": name}


def graph_arguments(example: dict, index: int, graph: str) -> dict:
    """Entities/relations to attach to paragraph `index` for the given graph."""
    if graph == "mentions":
        title = example["titles"][index]
        # ponytail: substring match on the bare title; short titles over-link
        others = {t for t in example["titles"] if t != title and base_title(t) in example["texts"][index]}
        relations = [{"source": entity(title), "relation": "mentions", "target": entity(t)} for t in sorted(others)]
        return {"entities": [entity(title)], "relations": relations}
    if graph == "oracle":
        return {
            "relations": [
                {"source": entity(s), "relation": r, "target": entity(o)}
                for s, r, o, i in example["triples"]
                if i == index
            ]
        }
    return {}


def call(mcp: Mcp, tool: str, arguments: dict) -> dict:
    response = mcp.request("tools/call", {"name": tool, "arguments": arguments})
    result = response.get("result") or {}
    if "error" in response or result.get("isError"):
        raise SystemExit(f"{tool} failed: {json.dumps(response)}")
    return result["structuredContent"]


def ingest(mcp: Mcp, dataset: str, graph: str, examples: list[dict]) -> None:
    """Store every paragraph; records each example's scope and memory id -> index."""
    started, stored = time.monotonic(), 0
    for n, example in enumerate(examples):
        example["scope"] = f"repo:/eval/{dataset}/{n}"
        example["index_by_id"] = {}
        for index, (title, text) in enumerate(zip(example["titles"], example["texts"])):
            arguments = {"content": f"{title}\n{text}", "scopes": [example["scope"]]}
            memory = call(mcp, "remember", arguments | graph_arguments(example, index, graph))
            example["index_by_id"][memory["id"]] = index
            stored += 1
        if (n + 1) % progress_step(len(examples)) == 0 or n + 1 == len(examples):
            elapsed = time.monotonic() - started
            log(
                f"{dataset}/{graph}: ingested {n + 1}/{len(examples)} questions, "
                f"{stored} memories in {elapsed:.1f}s ({stored / elapsed:.1f}/s)"
            )


def evaluate(mcp: Mcp, label: str, examples: list[dict], mode: str) -> dict[str, float]:
    use_embeddings, rewrite = MODES[mode]
    totals = {f"recall@{k}": 0.0 for k in KS} | {"mrr": 0.0}
    started = time.monotonic()
    for n, example in enumerate(examples):
        recalled = call(
            mcp,
            "recall",
            {
                "query": rewrite(example["question"]),
                "scopes": [example["scope"]],
                "limit": max(KS),
                "use_embeddings": use_embeddings,
            },
        )
        ranking = [example["index_by_id"][memory["id"]] for memory in recalled["memories"]]
        gold = example["gold"]
        for k in KS:
            totals[f"recall@{k}"] += len(gold & set(ranking[:k])) / len(gold)
        first_hit = next((rank for rank, index in enumerate(ranking, 1) if index in gold), None)
        totals["mrr"] += 1 / first_hit if first_hit else 0.0
        if (n + 1) % progress_step(len(examples)) == 0 or n + 1 == len(examples):
            log(f"{label}/{mode}: recalled {n + 1}/{len(examples)} in {time.monotonic() - started:.1f}s")
    return {name: total / len(examples) for name, total in totals.items()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--datasets", nargs="+", choices=DATASET_FILES, default=list(DATASET_FILES))
    parser.add_argument("--modes", nargs="+", choices=MODES, default=list(MODES))
    parser.add_argument("--graphs", nargs="+", choices=GRAPHS, default=list(GRAPHS))
    parser.add_argument("--questions", type=int, default=100, help="questions per dataset (default: 100)")
    parser.add_argument("--bin", help="gmem binary (default: target/release/gmem, then target/debug/gmem or PATH)")
    parser.add_argument(
        "--home",
        help="GRAPHMEM_HOME to use, e.g. ~/.graphmem-dev. It is FLUSHED before every dataset. "
        "Default: a fresh temp dir",
    )
    parser.add_argument(
        "--batch-size", type=int, help="embedding batch size for reembed (default: gmem's, 1 on CPU / 16 on CUDA)"
    )
    parser.add_argument("--timeout", type=float, default=600.0, help="per-request seconds (model download)")
    args = parser.parse_args()

    release = REPO_ROOT / "target" / "release" / "gmem"
    # Release first: a debug build embeds far too slowly for an eval.
    binary = resolve_binary(args.bin or (release if release.is_file() else None)).resolve()
    home = Path(args.home).expanduser() if args.home else Path(tempfile.mkdtemp(prefix="gmem-eval-"))
    home.mkdir(parents=True, exist_ok=True)
    # Share one model download across temp homes; an explicit override wins.
    os.environ.setdefault("GRAPHMEM_EMBEDDING_CACHE_DIR", str(MODEL_CACHE_DIR))
    os.environ["GRAPHMEM_HOME"] = str(home)
    batch_args = ["--embedding-batch-size", str(args.batch_size)] if args.batch_size else []

    def start_server(embeddings: bool) -> Mcp:
        # Mcp inherits os.environ; any value but "off" enables embeddings.
        os.environ["GRAPHMEM_EMBEDDINGS"] = "on" if embeddings else "off"
        mcp = Mcp(binary, home, timeout=args.timeout)
        mcp.request("initialize", CLIENT_INFO)
        return mcp

    def gmem(*command: str) -> str:
        env = {**os.environ, "GRAPHMEM_EMBEDDINGS": "on"}
        done = subprocess.run([binary, *command], env=env, capture_output=True, text=True)
        if done.returncode != 0:
            raise SystemExit(f"gmem {' '.join(command)} failed:\n{done.stderr}")
        return done.stdout.strip()

    log(f"binary={binary} home={home} questions={args.questions}")
    log(f"datasets={args.datasets} graphs={args.graphs} modes={args.modes} batch_size={args.batch_size}")
    rows = []
    try:
        for dataset in args.datasets:
            examples = load_examples(dataset, args.questions)
            log(f"{dataset}: loaded {len(examples)} questions")
            for graph in args.graphs:
                modes = [m for m in args.modes if graph == "none" or MODES[m][0]]
                label = f"{dataset}/{graph}"
                if not modes or (graph == "oracle" and not any(e["triples"] for e in examples)):
                    log(f"{label}: skipped (no applicable modes or no triples)")
                    continue
                # Empty store per run, so nothing from an earlier run
                # (memories, graph, ids) affects the ranking.
                log(f"{label}: flushing store")
                gmem("flush", "--yes")
                mcp = start_server(embeddings=False)
                try:
                    ingest(mcp, dataset, graph, examples)
                finally:
                    mcp.close()
                embeddings = any(MODES[m][0] for m in modes)
                if embeddings:
                    log(f"{label}: reembedding (batch size {args.batch_size or 'default'})")
                    started = time.monotonic()
                    summary = gmem(*batch_args, "reembed")
                    log(f"{label}: {summary} in {time.monotonic() - started:.1f}s")
                mcp = start_server(embeddings=embeddings)
                try:
                    for mode in modes:
                        scores = evaluate(mcp, label, examples, mode)
                        rows.append((dataset, graph, mode, len(examples), scores))
                        log(f"{label}/{mode}: " + " ".join(f"{k}={v:.3f}" for k, v in scores.items()))
                finally:
                    mcp.close()
    finally:
        if not args.home:
            shutil.rmtree(home, ignore_errors=True)

    metrics = list(rows[0][-1]) if rows else []
    print("\t".join(["dataset", "graph", "mode", "n", *metrics]))
    for *labels, count, scores in rows:
        print("\t".join([*labels, str(count), *(f"{scores[m]:.3f}" for m in metrics)]))

if __name__ == "__main__":
    main()
