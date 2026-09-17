#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "spacy>=3.8",
#     "en_core_web_sm @ https://github.com/explosion/spacy-models/releases/download/en_core_web_sm-3.8.0/en_core_web_sm-3.8.0-py3-none-any.whl",
# ]
# ///
"""Extract an entity graph from the eval paragraphs with spaCy, no LLM involved.

HippoRAG builds its graph with LLM OpenIE; this is the deterministic stand-in
behind `eval_retrieval.py --graphs spacy`. Every selected paragraph, supporting
or distractor, gets:

  entities   its title plus spaCy's named entities (numeric, date, and
             nationality labels dropped, names shorter than 3 characters dropped)
  relations  subject-verb-object triples whose subject and object are both
             entities; `prep` objects become `<verb>_<prep>` relations

Output: .data/eval/graphs/<dataset>.<model>.v<N>.jsonl, one JSON line per unique
paragraph: {"key", "entities", "relations"}. Existing keys are skipped, so
re-runs only parse new paragraphs.

Run with the same --datasets/--questions as the eval:
  uv run scripts/extract_eval_graphs.py --questions 100
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

import spacy
from spacy.tokens import Doc, Span, Token

sys.path.insert(0, str(Path(__file__).resolve().parent))
from eval_retrieval import (
    DATASET_FILES,
    GRAPH_CACHE_DIR,
    base_title,
    graph_cache_path,
    load_examples,
    log,
    unique_paragraphs,
)

# Numbers, dates, and nationalities ("American") link unrelated paragraphs.
DROPPED_LABELS = {
    "CARDINAL",
    "ORDINAL",
    "QUANTITY",
    "PERCENT",
    "MONEY",
    "TIME",
    "DATE",
    "NORP",
}
SUBJECTS = {"nsubj", "nsubjpass"}
OBJECTS = {"dobj", "attr", "oprd"}
PRONOUNS = {"he", "she", "it", "they"}
MIN_NAME_LENGTH = 3


def span_name(span: Span) -> str:
    """Entity text without leading determiners, so "the Eiffel Tower" == "Eiffel Tower"."""
    start = span.start
    while start < span.end and span.doc[start].pos_ == "DET":
        start += 1
    return span.doc[start : span.end].text.strip()


def extract(doc: Doc, title: str) -> tuple[list[str], list[list[str]]]:
    """Entities and [source, relation, target] triples for one parsed paragraph."""
    # Lowercased name -> entity name; the text names "Ed Wood (film)" as "Ed Wood".
    entities = {title.lower(): title, base_title(title).lower(): title}
    spans = [ent for ent in doc.ents if ent.label_ not in DROPPED_LABELS]
    for span in spans:
        name = span_name(span)
        if len(name) >= MIN_NAME_LENGTH:
            entities.setdefault(name.lower(), name)

    def name_of(token: Token) -> str | None:
        # ponytail: heuristic coreference; in Wikipedia a pronoun subject is usually the title
        if token.dep_ in SUBJECTS and token.lower_ in PRONOUNS:
            return title
        span = next((span for span in spans if span.start <= token.i < span.end), None)
        return entities.get(span_name(span).lower()) if span is not None else None

    relations = []
    for verb in doc:
        if verb.pos_ not in {"VERB", "AUX"}:
            continue
        subjects = [child for child in verb.children if child.dep_ in SUBJECTS]
        objects = [
            (child, verb.lemma_) for child in verb.children if child.dep_ in OBJECTS
        ]
        for prep in (child for child in verb.children if child.dep_ == "prep"):
            objects += [
                (child, f"{verb.lemma_}_{prep.lower_}")
                for child in prep.children
                if child.dep_ == "pobj"
            ]
        for subject in subjects:
            source = name_of(subject)
            for obj, name in objects:
                target = name_of(obj)
                triple = [source, name.lower().replace(" ", "_"), target]
                if source and target and source != target and triple not in relations:
                    relations.append(triple)
    return list(dict.fromkeys(entities.values())), relations


def self_check(nlp: spacy.language.Language) -> None:
    title = "Gustave Eiffel"
    doc = nlp("Gustave Eiffel designed the Eiffel Tower. He was born in Dijon.")
    entities, relations = extract(doc, title)
    assert entities == ["Gustave Eiffel", "Eiffel Tower", "Dijon"], entities
    assert relations == [
        ["Gustave Eiffel", "design", "Eiffel Tower"],
        ["Gustave Eiffel", "bear_in", "Dijon"],
    ], relations
    film, _ = extract(nlp("Ed Wood is a 1994 film about Ed Wood."), "Ed Wood (film)")
    # Dates are dropped; the bare title maps to the page.
    assert film == ["Ed Wood (film)"], film
    print(f"self-check OK: entities={entities} relations={relations}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--datasets", nargs="+", choices=DATASET_FILES, default=list(DATASET_FILES)
    )
    parser.add_argument(
        "--questions",
        type=int,
        default=100,
        help="questions per dataset (default: 100)",
    )
    parser.add_argument(
        "--model",
        default="en_core_web_sm",
        help="installed spaCy pipeline (default: en_core_web_sm)",
    )
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="test extraction on a fixed sentence and exit",
    )
    args = parser.parse_args()

    nlp = spacy.load(args.model)
    if args.self_check:
        self_check(nlp)
        return

    GRAPH_CACHE_DIR.mkdir(parents=True, exist_ok=True)
    for dataset in args.datasets:
        path = graph_cache_path(dataset, args.model)
        done = set()
        if path.is_file():
            with path.open() as lines:
                done = {json.loads(line)["key"] for line in lines}
        paragraphs = unique_paragraphs(load_examples(dataset, args.questions))
        pending = [
            (key, title, text)
            for key, (title, text) in paragraphs.items()
            if key not in done
        ]
        log(
            f"{dataset}: {len(paragraphs)} paragraphs, {len(pending)} to parse -> {path}"
        )
        started = time.monotonic()
        entity_total = relation_total = 0
        with path.open("a") as out:
            # The title is always an entity; parsing only the text keeps it out of the first sentence.
            texts = (text for _, _, text in pending)
            for n, ((key, title, _), doc) in enumerate(
                zip(pending, nlp.pipe(texts, batch_size=64)), 1
            ):
                entities, relations = extract(doc, title)
                entity_total += len(entities)
                relation_total += len(relations)
                out.write(
                    json.dumps(
                        {"key": key, "entities": entities, "relations": relations}
                    )
                    + "\n"
                )
                if n % max(1, len(pending) // 10) == 0 or n == len(pending):
                    log(
                        f"{dataset}: parsed {n}/{len(pending)} in {time.monotonic() - started:.1f}s, "
                        f"{entity_total / n:.1f} entities and {relation_total / n:.1f} relations per paragraph"
                    )


if __name__ == "__main__":
    main()
