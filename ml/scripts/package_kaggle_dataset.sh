#!/usr/bin/env bash
# Bundle everything the ML-S1 Kaggle notebook needs into one zip, to upload
# as a PRIVATE Kaggle dataset. The bundle mirrors the repository layout
# (ml/ and schemas/ side by side) so the evaluator finds its files the same
# way it does locally. Base model weights are not included: the notebook
# downloads them from Hugging Face at pinned revisions.
#
# usage (from the repository root): ml/scripts/package_kaggle_dataset.sh [out-dir]
# output: <out-dir>/crisisgraph-ml-s1.zip (default out-dir: ml/internal/kaggle)
set -euo pipefail

OUT_DIR=${1:-ml/internal/kaggle}
NAME=crisisgraph-ml-s1
ADAPTER=ml/artifacts/qwen3-4b-triage-sft-v4
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
B=$STAGE/$NAME

[ -f Cargo.toml ] && [ -d ml/src/crisisgraph_ml ] || { echo "run from the repository root"; exit 1; }

# Fail if a file the study depends on differs from the one the SFT-V4 report used.
check() {   # check <file> <sha256>
  local got; got=$(sha256sum "$1" | cut -d' ' -f1)
  [ "$got" = "$2" ] || { echo "sha256 mismatch: $1 ($got, expected $2)"; exit 1; }
}
check ml/data/triage-eval-v3.1.0.jsonl cc67e434cb8edbd23c66eb2fc71aa576df7ba448bc3d9bb042581a25fc1e6cc8
check ml/prompts/triage-extraction-v3.2.txt 9368283ce56be397fd14fda5f77495f493f99118f4812d4fe00d03c946789623
check schemas/triage-extraction-v3.json c9b3ed10855da712c544ecc998db0f841e250313e31ff6cec953932b759a1051
check "$ADAPTER/adapter-f16.gguf" f31100c925b3b2f4540f481be0f9338d5024ca931c0368265f6b16eb5f520fbc

mkdir -p "$B/ml/data" "$B/ml/prompts" "$B/schemas" "$B/adapter/peft"

# Evaluator package (no caches) and its project file.
cp ml/pyproject.toml "$B/ml/"
cp -r ml/src "$B/ml/"
find "$B/ml/src" -name __pycache__ -type d -prune -exec rm -rf {} +

# Blind test set, its manifest, the leak audit (sensitivity subset) and the urgency vectors.
cp ml/data/triage-eval-v3.1.0.jsonl ml/data/triage-eval-v3.1.0.manifest.json \
   ml/data/triage-sft-v4-leak-audit.json ml/data/urgency-rules-v5.jsonl "$B/ml/data/"
cp ml/prompts/triage-extraction-v3.2.txt "$B/ml/prompts/"
cp schemas/triage-extraction-v3.json "$B/schemas/"

# SFT-V4 adapter: PEFT files for vLLM and merging; GGUF for the llama.cpp reference.
# training_args.bin (a pickle, not needed for serving) is left out.
for f in adapter_config.json adapter_model.safetensors tokenizer.json tokenizer_config.json \
         chat_template.jinja README.md run-manifest.json; do
  cp "$ADAPTER/adapter/$f" "$B/adapter/peft/"
done
cp "$ADAPTER/adapter-f16.gguf" "$B/adapter/"

# Provenance: the commit the bundle was made from, and a checksum for every file.
cat > "$B/BUNDLE.json" <<EOF
{
  "bundle": "$NAME",
  "created": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "git_commit": "$(git rev-parse HEAD)",
  "git_dirty": $([ -z "$(git status --porcelain -- ml/src ml/data ml/prompts schemas)" ] && echo false || echo true),
  "base_model": "Qwen/Qwen3-4B-Instruct-2507",
  "base_model_revision": "cdbee75f17c01a7cc42f958dc650907174af0554",
  "gguf_repo": "lmstudio-community/Qwen3-4B-Instruct-2507-GGUF",
  "gguf_revision": "4edb920b6f14e3b9284d4502a6485103d72cde05",
  "gguf_file": "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
  "gguf_sha256": "8cdb57cbb880d313736a9bc4e3d3d2485f145b5e19cf33783746e753e82641fc"
}
EOF
(cd "$B" && find . -type f ! -name SHA256SUMS | sort | xargs sha256sum > SHA256SUMS)

mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR/$NAME.zip"
bsdtar -a -cf "$OUT_DIR/$NAME.zip" -C "$STAGE" "$NAME"   # zip format, from the .zip suffix (libarchive)
echo "wrote $OUT_DIR/$NAME.zip ($(du -h "$OUT_DIR/$NAME.zip" | cut -f1), $(wc -l < "$B/SHA256SUMS") files)"
