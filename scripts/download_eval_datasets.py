#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = ["datasets"]
# ///
"""Download the open multi-hop QA datasets used to evaluate retrieval quality.

HotpotQA, 2WikiMultiHopQA and MuSiQue are the three benchmarks the HippoRAG
(2) paper itself evaluates on, which is the algorithm graphmem implements
today (see todos/graph_tune.md). Run with: uv run scripts/download_eval_datasets.py
"""

from __future__ import annotations

from pathlib import Path

from datasets import load_dataset

OUTPUT_DIR = Path(__file__).resolve().parent.parent / ".data" / "eval"


def download() -> None:
    """Fetch the validation split of each benchmark and save it as JSONL."""
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    hotpotqa = load_dataset("hotpotqa/hotpot_qa", "distractor", split="validation")
    hotpotqa.to_json(OUTPUT_DIR / "hotpotqa_distractor_validation.jsonl")

    two_wiki = load_dataset(
        "voidful/2WikiMultihopQA", data_files={"validation": "dev.json"}
    )["validation"]
    two_wiki.to_json(OUTPUT_DIR / "2wikimultihopqa_validation.jsonl")

    musique = load_dataset("dgslibisey/MuSiQue", split="validation")
    musique.to_json(OUTPUT_DIR / "musique_validation.jsonl")

    for name, dataset in [
        ("hotpotqa", hotpotqa),
        ("2wikimultihopqa", two_wiki),
        ("musique", musique),
    ]:
        print(f"{name}: {len(dataset)} examples -> {OUTPUT_DIR}")


if __name__ == "__main__":
    download()
