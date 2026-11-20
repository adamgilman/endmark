#!/bin/sh
# Runs the replay over every document in corpus.txt and prints one table.
# Usage: eval/run.sh DIRECTORY   (where the repositories are, or will be, cloned)
# Needs git, python3 and a release build: cargo build --release
set -eu
here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: eval/run.sh DIRECTORY}
mkdir -p "$work"
rows="$work/rows.jsonl"
: > "$rows"
grep -v '^#' "$here/corpus.txt" | while read -r url head files; do
  [ -n "$url" ] || continue
  name=$(basename "$url")
  [ -d "$work/$name" ] || git clone -q --filter=blob:none --no-checkout "$url" "$work/$name"
  git -C "$work/$name" cat-file -e "$head^{commit}" 2>/dev/null || git -C "$work/$name" fetch -q origin "$head"
  # shellcheck disable=SC2086
  python3 "$here/replay.py" "$work/$name" $files --head "$head" --endmark "$here/../target/release/endmark" \
    --pairs 25 --samples 15 --out "$work/$name.jsonl" > /dev/null
  cat "$work/$name.jsonl" >> "$rows"
done
python3 "$here/replay.py" --summarise "$rows"
