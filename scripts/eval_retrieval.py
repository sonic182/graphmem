#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Compare gmem recall quality with and without embeddings on multi-hop QA.

Drives `gmem mcp` over stdio. Candidate paragraphs are stored as memories:

Corpus (--corpus):
  shared        every paragraph of every selected question, deduplicated, in
                one `repo:/eval/<dataset>` scope. Each question ranks the whole
                corpus, the setting HippoRAG evaluates (default)
  per-question  each question's own 10-20 paragraphs in its own
                `repo:/eval/<dataset>/<n>` scope (the datasets' "distractor"
                setting). Recall's scope filter runs before ranking, so a
                question only ranks its own paragraphs; recall@10 is ~1 there

Each (dataset, graph) pair starts from a flushed store and is ingested once by
a server with embeddings off (fast inserts). `gmem reembed` then embeds everything (in batches when
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
  spacy     entities and subject-verb-object triples extracted per paragraph
            by spaCy (scripts/extract_eval_graphs.py), the LLM-free stand-in
            for HippoRAG's OpenIE. Applied to every paragraph alike
  oracle    the dataset's own reasoning triples (2Wiki `evidences`, MuSiQue
            `question_decomposition`). They only describe the gold path, so
            this LEAKS the answer: read it as a ceiling, not a result.
            HotpotQA has no triples and skips it

Metrics (averaged over questions): recall@k = share of supporting paragraphs
in the top k, MRR = 1/rank of the first supporting paragraph.

Setup, from the repo root (this script is stdlib only):
  uv run scripts/download_eval_datasets.py
  uv run scripts/extract_eval_graphs.py --questions 100   # for --graphs spacy
  cargo build --release
  uv run scripts/eval_retrieval.py --questions 100
Use the same --datasets/--questions for extraction and evaluation.
"""

from __future__ import annotations

import argparse
import hashlib
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
sys.path.insert(
    0, str(REPO_ROOT / ".agents" / "skills" / "gmem-development" / "scripts")
)
from mcp_smoke import CLIENT_INFO, Mcp, resolve_binary

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
GRAPHS = ("none", "mentions", "spacy", "oracle")
CORPORA = ("shared", "per-question")
GRAPH_CACHE_DIR = DATA_DIR / "graphs"
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
            question = re.sub(
                r"#(\d+)", lambda m: answers[int(m.group(1)) - 1], step["question"]
            )
            answers.append(step["answer"])
            # ponytail: steps phrased as plain questions (no ">>") have no triple; skipped
            if " >> " in question and step["paragraph_support_idx"] is not None:
                subject, relation = question.split(" >> ", 1)
                triples.append(
                    (subject, relation, step["answer"], step["paragraph_support_idx"])
                )
        return {
            "question": row["question"],
            "titles": titles,
            "texts": texts,
            "gold": gold,
            "triples": triples,
        }

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
    return {
        "question": row["question"],
        "titles": titles,
        "texts": texts,
        "gold": gold,
        "triples": triples,
    }


def base_title(title: str) -> str:
    return re.sub(r"\s*\([^)]*\)$", "", title)


def paragraph_key(title: str, text: str) -> str:
    """Content key shared with extract_eval_graphs.py; dedupes the shared corpus."""
    return hashlib.sha1(f"{title}\n{text}".encode()).hexdigest()[:16]


def unique_paragraphs(examples: list[dict]) -> dict[str, tuple[str, str]]:
    """Every distinct paragraph of `examples` as key -> (title, text), in first-seen order."""
    paragraphs = {}
    for example in examples:
        for title, text in zip(example["titles"], example["texts"]):
            paragraphs.setdefault(paragraph_key(title, text), (title, text))
    return paragraphs


def build_corpus(dataset: str, examples: list[dict], corpus: str) -> tuple[list, dict]:
    """Assign scopes and gold keys to `examples`.

    Returns the ingestion units, [(scope, [(key, example_index, paragraph_index)])],
    with each paragraph once per scope, and the oracle triples by paragraph key.
    """
    units: dict[str, dict[str, tuple[int, int]]] = {}
    oracle: dict[str, list[tuple[str, str, str]]] = {}
    for n, example in enumerate(examples):
        scope = (
            f"repo:/eval/{dataset}"
            if corpus == "shared"
            else f"repo:/eval/{dataset}/{n}"
        )
        keys = [
            paragraph_key(title, text)
            for title, text in zip(example["titles"], example["texts"])
        ]
        example["scope"] = scope
        example["gold_keys"] = {keys[i] for i in example["gold"]}
        paragraphs = units.setdefault(scope, {})
        for index, key in enumerate(keys):
            # The first occurrence decides which question's titles `mentions` sees.
            paragraphs.setdefault(key, (n, index))
        for subject, relation, obj, index in example["triples"]:
            triples = oracle.setdefault(keys[index], [])
            if (subject, relation, obj) not in triples:
                triples.append((subject, relation, obj))
    ingestion = [
        (scope, [(key, *where) for key, where in paragraphs.items()])
        for scope, paragraphs in units.items()
    ]
    return ingestion, oracle


def load_graph_cache(dataset: str, model: str) -> dict[str, dict]:
    path = GRAPH_CACHE_DIR / f"{dataset}.{model}.jsonl"
    if not path.is_file():
        return {}
    with path.open() as lines:
        return {row["key"]: row for row in map(json.loads, lines)}


def entity(name: str) -> dict:
    return {"kind": "entity", "name": name}


def relation(source: str, name: str, target: str) -> dict:
    return {"source": entity(source), "relation": name, "target": entity(target)}


def graph_arguments(
    graph: str, example: dict, index: int, key: str, lookups: dict
) -> dict:
    """Entities/relations to attach to one paragraph for the given graph."""
    if graph == "mentions":
        title, text = example["titles"][index], example["texts"][index]
        # ponytail: substring match on the bare title; short titles over-link
        others = {t for t in example["titles"] if t != title and base_title(t) in text}
        return {
            "entities": [entity(title)],
            "relations": [
                relation(title, "mentions", other) for other in sorted(others)
            ],
        }
    if graph == "spacy":
        row = lookups["spacy"].get(key)
        if row is None:
            raise SystemExit(
                f"paragraph {key} has no spaCy graph; run `uv run scripts/extract_eval_graphs.py` "
                "with the same --datasets/--questions (and --model) first"
            )
        return {
            "entities": [entity(name) for name in row["entities"]],
            "relations": [relation(*triple) for triple in row["relations"]],
        }
    if graph == "oracle":
        return {
            "relations": [
                relation(*triple) for triple in lookups["oracle"].get(key, [])
            ]
        }
    return {}


def call(mcp: Mcp, tool: str, arguments: dict) -> dict:
    response = mcp.request("tools/call", {"name": tool, "arguments": arguments})
    result = response.get("result") or {}
    if "error" in response or result.get("isError"):
        raise SystemExit(f"{tool} failed: {json.dumps(response)}")
    return result["structuredContent"]


def ingest(
    mcp: Mcp, label: str, graph: str, examples: list[dict], units: list, lookups: dict
) -> dict[int, str]:
    """Store every paragraph of `units`; returns memory id -> paragraph key."""
    total = sum(len(paragraphs) for _, paragraphs in units)
    key_by_id: dict[int, str] = {}
    started = time.monotonic()
    for scope, paragraphs in units:
        for key, n, index in paragraphs:
            example = examples[n]
            arguments = {
                "content": f"{example['titles'][index]}\n{example['texts'][index]}",
                "scopes": [scope],
            }
            memory = call(
                mcp,
                "remember",
                arguments | graph_arguments(graph, example, index, key, lookups),
            )
            key_by_id[memory["id"]] = key
            if len(key_by_id) % progress_step(total) == 0 or len(key_by_id) == total:
                elapsed = time.monotonic() - started
                log(
                    f"{label}: ingested {len(key_by_id)}/{total} memories "
                    f"in {elapsed:.1f}s ({len(key_by_id) / elapsed:.1f}/s)"
                )
    return key_by_id


def evaluate(
    mcp: Mcp, label: str, examples: list[dict], key_by_id: dict[int, str], mode: str
) -> dict[str, float]:
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
        ranking = [key_by_id[memory["id"]] for memory in recalled["memories"]]
        gold = example["gold_keys"]
        for k in KS:
            totals[f"recall@{k}"] += len(gold & set(ranking[:k])) / len(gold)
        first_hit = next(
            (rank for rank, key in enumerate(ranking, 1) if key in gold), None
        )
        totals["mrr"] += 1 / first_hit if first_hit else 0.0
        if (n + 1) % progress_step(len(examples)) == 0 or n + 1 == len(examples):
            log(
                f"{label}/{mode}: recalled {n + 1}/{len(examples)} in {time.monotonic() - started:.1f}s"
            )
    return {name: total / len(examples) for name, total in totals.items()}


def self_check() -> None:
    """Corpus building on synthetic rows; no gmem or dataset files needed."""
    shared_page = ["Shared page", ["Appears in both questions."]]
    rows = [
        {
            "question": "q1",
            "context": [shared_page, ["Only one", ["First question only."]]],
            "supporting_facts": [["Shared page", 0]],
            "evidences": [["Shared page", "links", "Only one"]],
        },
        {
            "question": "q2",
            "context": [["Only two", ["Second question only."]], shared_page],
            "supporting_facts": [["Only two", 0], ["Shared page", 0]],
            "evidences": [["Shared page", "links", "Only two"]],
        },
    ]
    shared_key = paragraph_key("Shared page", "Appears in both questions.")

    examples = [parse_row("2wikimultihopqa", row) for row in rows]
    units, oracle = build_corpus("demo", examples, "shared")
    assert [scope for scope, _ in units] == ["repo:/eval/demo"], units
    assert len(units[0][1]) == 3, "the shared page is stored once"
    assert examples[0]["gold_keys"] == {shared_key}
    assert examples[1]["gold_keys"] == {
        shared_key,
        paragraph_key("Only two", "Second question only."),
    }
    assert oracle[shared_key] == [
        ("Shared page", "links", "Only one"),
        ("Shared page", "links", "Only two"),
    ]

    examples = [parse_row("2wikimultihopqa", row) for row in rows]
    units, _ = build_corpus("demo", examples, "per-question")
    assert [len(paragraphs) for _, paragraphs in units] == [2, 2], (
        "each question keeps its own copy"
    )
    assert examples[1]["scope"] == "repo:/eval/demo/1"
    print("self-check OK")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--datasets", nargs="+", choices=DATASET_FILES, default=list(DATASET_FILES)
    )
    parser.add_argument("--modes", nargs="+", choices=MODES, default=list(MODES))
    parser.add_argument("--graphs", nargs="+", choices=GRAPHS, default=list(GRAPHS))
    parser.add_argument("--corpus", choices=CORPORA, default="shared")
    parser.add_argument(
        "--spacy-model",
        default="en_core_web_sm",
        help="which spaCy graph cache to read",
    )
    parser.add_argument(
        "--questions",
        type=int,
        default=100,
        help="questions per dataset (default: 100)",
    )
    parser.add_argument(
        "--bin",
        help="gmem binary (default: target/release/gmem, then target/debug/gmem or PATH)",
    )
    parser.add_argument(
        "--home",
        help="GRAPHMEM_HOME to use, e.g. ~/.graphmem-dev. It is FLUSHED before every dataset. "
        "Default: a fresh temp dir",
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        help="embedding batch size for reembed (default: gmem's, 1 on CPU / 16 on CUDA)",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=600.0,
        help="per-request seconds (model download)",
    )
    parser.add_argument(
        "--self-check", action="store_true", help="test corpus building and exit"
    )
    args = parser.parse_args()
    if args.self_check:
        self_check()
        return

    release = REPO_ROOT / "target" / "release" / "gmem"
    # Release first: a debug build embeds far too slowly for an eval.
    binary = resolve_binary(
        args.bin or (release if release.is_file() else None)
    ).resolve()
    home = (
        Path(args.home).expanduser()
        if args.home
        else Path(tempfile.mkdtemp(prefix="gmem-eval-"))
    )
    home.mkdir(parents=True, exist_ok=True)
    # Share one model download across temp homes; an explicit override wins.
    os.environ.setdefault("GRAPHMEM_EMBEDDING_CACHE_DIR", str(MODEL_CACHE_DIR))
    os.environ["GRAPHMEM_HOME"] = str(home)
    batch_args = (
        ["--embedding-batch-size", str(args.batch_size)] if args.batch_size else []
    )

    def start_server(embeddings: bool) -> Mcp:
        # Mcp inherits os.environ; any value but "off" enables embeddings.
        os.environ["GRAPHMEM_EMBEDDINGS"] = "on" if embeddings else "off"
        mcp = Mcp(binary, home, timeout=args.timeout)
        mcp.request("initialize", CLIENT_INFO)
        return mcp

    def gmem(*command: str) -> str:
        env = {**os.environ, "GRAPHMEM_EMBEDDINGS": "on"}
        done = subprocess.run(
            [binary, *command], env=env, capture_output=True, text=True, check=False
        )
        if done.returncode != 0:
            raise SystemExit(f"gmem {' '.join(command)} failed:\n{done.stderr}")
        return done.stdout.strip()

    log(f"binary={binary} home={home} questions={args.questions}")
    log(
        f"datasets={args.datasets} corpus={args.corpus} graphs={args.graphs} "
        f"modes={args.modes} batch_size={args.batch_size}"
    )
    rows = []
    try:
        for dataset in args.datasets:
            examples = load_examples(dataset, args.questions)
            units, oracle = build_corpus(dataset, examples, args.corpus)
            lookups = {"oracle": oracle, "spacy": {}}
            if "spacy" in args.graphs:
                lookups["spacy"] = load_graph_cache(dataset, args.spacy_model)
            memories = sum(len(paragraphs) for _, paragraphs in units)
            log(
                f"{dataset}: loaded {len(examples)} questions, {memories} memories in {len(units)} scope(s)"
            )
            for graph in args.graphs:
                modes = [m for m in args.modes if graph == "none" or MODES[m][0]]
                label = f"{dataset}/{args.corpus}/{graph}"
                if not modes or (graph == "oracle" and not oracle):
                    log(f"{label}: skipped (no applicable modes or no triples)")
                    continue
                # Empty store per run, so nothing from an earlier run
                # (memories, graph, ids) affects the ranking.
                log(f"{label}: flushing store")
                gmem("flush", "--yes")
                mcp = start_server(embeddings=False)
                try:
                    key_by_id = ingest(mcp, label, graph, examples, units, lookups)
                finally:
                    mcp.close()
                embeddings = any(MODES[m][0] for m in modes)
                if embeddings:
                    log(
                        f"{label}: reembedding (batch size {args.batch_size or 'default'})"
                    )
                    started = time.monotonic()
                    summary = gmem(*batch_args, "reembed")
                    log(f"{label}: {summary} in {time.monotonic() - started:.1f}s")
                mcp = start_server(embeddings=embeddings)
                try:
                    for mode in modes:
                        scores = evaluate(mcp, label, examples, key_by_id, mode)
                        rows.append(
                            (dataset, args.corpus, graph, mode, len(examples), scores)
                        )
                        log(
                            f"{label}/{mode}: "
                            + " ".join(f"{k}={v:.3f}" for k, v in scores.items())
                        )
                finally:
                    mcp.close()
    finally:
        if not args.home:
            shutil.rmtree(home, ignore_errors=True)

    metrics = list(rows[0][-1]) if rows else []
    print("\t".join(["dataset", "corpus", "graph", "mode", "n", *metrics]))
    for *labels, count, scores in rows:
        print("\t".join([*labels, str(count), *(f"{scores[m]:.3f}" for m in metrics)]))


if __name__ == "__main__":
    main()
