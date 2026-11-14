#!/usr/bin/env bash
# check-no-secrets.sh — scan every blob in git history for sensitive
# or environment-disclosing content patterns. Exit 1 on any hit.
#
# NOTE: this scanner excludes its own path (its filename/patterns
# would otherwise match themselves); risky literals below are split ('pa''ssword') so the
# scanner's own source does not match its own patterns.
set -euo pipefail

# Keyword-looking identifiers only matter as assignments/values; bare words
# such as xv6's gettoken() or prose 'address token' are not credentials.
KW1='\b(pass''word|pass''wd|se''cret|to''ken|cre''dential|api[_-]?key)\b[[:space:]]*[:=][[:space:]]*[^[:space:]]+'
KW2='PRIVATE''[[:space:]]KEY|BEGIN (RSA|OPENSSH|EC|DSA)'
KW3='ssh-''rsa[[:space:]]AAAA|ssh-''ed25519[[:space:]]AAAA'
PATTERNS="(${KW1}|${KW2}|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|xox[baprs]-|/root/[a-z]|/home/[a-z]+/|/Users/[a-z]|${KW3})"

hits=0
while IFS= read -r match; do
  echo "VIOLATION: $match"
  hits=$((hits+1))
done < <(git grep -inE "$PATTERNS" $(git rev-list --all) -- ':(exclude)scripts/check-no-secrets.sh' 2>/dev/null || true)

if [ "$hits" -ne 0 ]; then echo "FAIL: $hits suspicious blob(s) in history"; exit 1; fi
echo "OK: history clean"
