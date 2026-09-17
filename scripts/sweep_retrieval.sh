#!/usr/bin/env bash
# Grid sweep for gmem's [retrieval] defaults over the eval harness.
#
# Drives scripts/eval_retrieval.py once per config in the cartesian product of
# the value lists passed on the command line. The harness spawns `gmem mcp`
# children that inherit os.environ, and gmem resolves config as
# env > flag > file > default, so exporting GRAPHMEM_RETRIEVAL_* here reaches
# the server that answers `recall` without touching gmem.
#
# Each value list defaults to the built-in default, so with no arguments this
# runs exactly the baseline config once.
#
# Every run writes its own file: .data/eval/<timestamp>_<name>.tsv, so results
# from different matrices never overwrite each other.
#
# Usage:
#   scripts/sweep_retrieval.sh --dry-run
#   scripts/sweep_retrieval.sh --name damping --damping 0.3 0.5 0.7
#   scripts/sweep_retrieval.sh --name focus --damping 0.6 0.7 --weight 0.6 0.7 --top-k 10 20
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

bin="${GMEM_BIN:-$HOME/.cargo/bin/gmem}"
datasets=(hotpotqa 2wikimultihopqa musique)
graphs=(mentions)
questions=100
offset=0
name="sweep"
out=""
dry_run=false

# Built-in RetrievalConfig::default(); each dimension defaults to its baseline.
damping_vals=(0.5)
weight_vals=(0.5)
temperature_vals=(0.05)
top_k_vals=(20)
anchor_vals=(0.2)

usage() {
    sed -n '2,19p' "${BASH_SOURCE[0]}" | cut -c3-
    exit "${1:-0}"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --damping) shift; damping_vals=(); while [[ $# -gt 0 && "$1" != --* ]]; do damping_vals+=("$1"); shift; done ;;
        --weight) shift; weight_vals=(); while [[ $# -gt 0 && "$1" != --* ]]; do weight_vals+=("$1"); shift; done ;;
        --temperature) shift; temperature_vals=(); while [[ $# -gt 0 && "$1" != --* ]]; do temperature_vals+=("$1"); shift; done ;;
        --top-k) shift; top_k_vals=(); while [[ $# -gt 0 && "$1" != --* ]]; do top_k_vals+=("$1"); shift; done ;;
        --anchor) shift; anchor_vals=(); while [[ $# -gt 0 && "$1" != --* ]]; do anchor_vals+=("$1"); shift; done ;;
        --datasets) shift; datasets=(); while [[ $# -gt 0 && "$1" != --* ]]; do datasets+=("$1"); shift; done ;;
        --graphs) shift; graphs=(); while [[ $# -gt 0 && "$1" != --* ]]; do graphs+=("$1"); shift; done ;;
        --questions) questions="$2"; shift 2 ;;
        --offset) offset="$2"; shift 2 ;;
        --bin) bin="$2"; shift 2 ;;
        --name) name="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        --dry-run) dry_run=true; shift ;;
        -h | --help) usage 0 ;;
        *) echo "unknown argument: $1" >&2; usage 2 ;;
    esac
done

total=$(( ${#damping_vals[@]} * ${#weight_vals[@]} * ${#temperature_vals[@]} * ${#top_k_vals[@]} * ${#anchor_vals[@]} ))
out="${out:-$REPO_ROOT/.data/eval/$(date +%Y%m%d-%H%M%S)_${name}.tsv}"

if [[ "$dry_run" == true ]]; then
    echo "$total config(s) x ${#datasets[@]} dataset(s) x ${#graphs[@]} graph(s)"
    echo "output: $out"
    for d in "${damping_vals[@]}"; do
        for w in "${weight_vals[@]}"; do
            for t in "${temperature_vals[@]}"; do
                for k in "${top_k_vals[@]}"; do
                    for a in "${anchor_vals[@]}"; do
                        printf 'damping=%s weight=%s temperature=%s top_k=%s anchor=%s\n' "$d" "$w" "$t" "$k" "$a"
                    done
                done
            done
        done
    done
    exit 0
fi

[[ -x "$bin" ]] || { echo "gmem binary not executable: $bin" >&2; exit 1; }
mkdir -p "$(dirname "$out")"

printf 'damping\tweight\ttemperature\ttop_k\tanchor\tdataset\tgraph\trecall@2\trecall@5\trecall@10\tmrr\n' >"$out"

i=0
for d in "${damping_vals[@]}"; do
    for w in "${weight_vals[@]}"; do
        for t in "${temperature_vals[@]}"; do
            for k in "${top_k_vals[@]}"; do
                for a in "${anchor_vals[@]}"; do
                    i=$((i + 1))
                    echo "sweep $i/$total damping=$d weight=$w temperature=$t top_k=$k anchor=$a" >&2
                    GRAPHMEM_RETRIEVAL_DAMPING="$d" \
                        GRAPHMEM_RETRIEVAL_MEMORY_SEED_WEIGHT="$w" \
                        GRAPHMEM_RETRIEVAL_SEED_TEMPERATURE="$t" \
                        GRAPHMEM_RETRIEVAL_SEED_TOP_K="$k" \
                        GRAPHMEM_RETRIEVAL_ENTITY_ANCHOR_WEIGHT="$a" \
                        uv run scripts/eval_retrieval.py \
                        --bin "$bin" \
                        --datasets "${datasets[@]}" \
                        --graphs "${graphs[@]}" \
                        --modes embeddings \
                        --questions "$questions" \
                        --offset "$offset" \
                        2>/dev/null |
                        awk -F'\t' -v OFS='\t' -v d="$d" -v w="$w" -v t="$t" -v k="$k" -v a="$a" \
                            'NR > 1 { print d, w, t, k, a, $1, $3, $6, $7, $8, $9 }' >>"$out"
                done
            done
        done
    done
done

echo >&2
echo "results: $out" >&2
echo "macro-average recall@5 per config (best first):" >&2
echo -e "damping\tweight\ttemperature\ttop_k\tanchor\tmean_recall@5\tdatasets"
awk -F'\t' 'NR > 1 { key = $1 FS $2 FS $3 FS $4 FS $5; sum[key] += $9; n[key]++ }
    END { for (key in sum) printf "%s\t%.4f\t%d\n", key, sum[key] / n[key], n[key] }' \
    "$out" | sort -t$'\t' -k6,6nr
