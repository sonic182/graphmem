#!/usr/bin/env bash
# Fetch the pinned OpenClaw checkout and build the gmem code index for it.
#
# Usage: setup_corpus.sh [corpus_dir] [index_dir] [gmem_binary]
set -euo pipefail

CORPUS="${1:-$HOME/.cache/gmem-eval/openclaw-bench}"
INDEX="${2:-$HOME/.cache/gmem-eval/index}"
GMEM="${3:-$(command -v gmem || true)}"

BASE=4de57f22aa1dcd118ec156a08588adb1b54404bd
HEAD=8f5c33c30d710d34c01a017a05f0ccbff8315452
REPO=https://github.com/openclaw/openclaw.git

if [[ -z "$GMEM" ]]; then
    echo "gmem binary not found; pass it as the third argument" >&2
    exit 1
fi

if [[ ! -d "$CORPUS/.git" ]]; then
    mkdir -p "$CORPUS"
    git -C "$CORPUS" init -q
    git -C "$CORPUS" remote add origin "$REPO"
fi

# depth 2 of HEAD also brings in its parent, the diff base.
git -C "$CORPUS" fetch --depth 2 origin "$HEAD"
git -C "$CORPUS" checkout -q --detach "$HEAD"
git -C "$CORPUS" cat-file -e "$BASE" || {
    echo "base commit $BASE missing; fetch it explicitly" >&2
    exit 1
}

mkdir -p "$INDEX"
if [[ -d "$HOME/.graphmem/models" && ! -e "$INDEX/models" ]]; then
    ln -s "$HOME/.graphmem/models" "$INDEX/models"
fi

echo "indexing $CORPUS ..."
GRAPHMEM_HOME="$INDEX" GRAPHMEM_CODE_MAX_FILES=100000 "$GMEM" code index "$CORPUS"
echo "corpus ready at $CORPUS ($HEAD)"
