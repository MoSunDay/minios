#!/usr/bin/env bash
# check-no-binaries.sh — refuse binary blobs in the whole git history.
# Text extension allowlist + size threshold (exit 1 on violation).
set -euo pipefail

MAX_TEXT_BYTES=1048576  # 1 MiB cap even for allowed text types
ALLOW='\.(rs|c|h|S|s|ld|toml|lock|md|txt|sh|py|pl|json|yml|yaml|patch|diff|expect|cfg)$|(^|/)(\.gitignore|\.cvsignore|Makefile|LICENSE|README[^/]*|BUGS|Notes)$'

bad=0
while IFS= read -r line; do
  size=${line%% *}
  path=${line#* }
  [ -z "$path" ] && continue
  if ! printf '%s' "$path" | grep -qiE "$ALLOW"; then
    echo "VIOLATION(non-allowlist): $path ($size bytes)"; bad=1
  elif [ "$size" -gt "$MAX_TEXT_BYTES" ]; then
    echo "VIOLATION(>1MiB text): $path ($size bytes)"; bad=1
  fi
done < <(git rev-list --objects --all 2>/dev/null \
        | git cat-file --batch-check='%(objecttype) %(objectsize) %(rest)' 2>/dev/null \
        | awk '$1=="blob" {print $2" "$3}')

if [ "$bad" -ne 0 ]; then echo "FAIL: binary/large blobs found in history"; exit 1; fi
echo "OK: no binary blobs in history"
