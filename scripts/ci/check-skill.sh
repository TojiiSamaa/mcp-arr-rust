#!/usr/bin/env bash
# Contract: --list-tools == tool names in SKILL.md == tool names in README.md.
# Fails (non-zero) on any missing or phantom tool. Usage: check-skill.sh [binary]
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN="${1:-}"
# Write and destructive tools are only listed when a switch is on (writes: ALLOW_WRITES or DRY_RUN; destructive: ALLOW_DELETE or DRY_RUN): open both.
export ARR_ALLOW_WRITES=true ARR_ALLOW_DELETE=true
if [ -n "$BIN" ]; then real=$("$BIN" --list-tools); else real=$(cargo run --quiet -- --list-tools); fi
[ "$(printf '%s\n' "$real" | grep -c 'arr_')" -eq 22 ] || echo "check-skill: warning, expected 22 tools" >&2
real_names=$(printf '%s\n' "$real" | grep -oE '\barr_[a-z_]+\b' | sort -u)
[ -n "$real_names" ] || { echo "check-skill: --list-tools returned no arr_ tool" >&2; exit 2; }
rc=0
for f in skill/mcp-arr-rust/SKILL.md README.md; do
  doc_names=$(grep -oE '\barr_[a-z_]+\b' "$f" | sort -u)
  missing=$(comm -23 <(printf '%s\n' "$real_names") <(printf '%s\n' "$doc_names"))
  phantom=$(comm -13 <(printf '%s\n' "$real_names") <(printf '%s\n' "$doc_names"))
  [ -z "$missing" ] || { echo "check-skill: $f lacks: $(echo $missing)" >&2; rc=1; }
  [ -z "$phantom" ] || { echo "check-skill: $f has phantom: $(echo $phantom)" >&2; rc=1; }
done
for s in '## Possible' '## Impossible' '## Comment faire' '## Mode test' '## Stats'; do
  grep -q "^$s" skill/mcp-arr-rust/SKILL.md || { echo "check-skill: SKILL.md lacks section $s" >&2; rc=1; }
done
grep -q '^## \[1\.0\.0\]' CHANGELOG.md || { echo "check-skill: CHANGELOG lacks [1.0.0]" >&2; rc=1; }
[ $rc -eq 0 ] && echo "check-skill: OK ($(echo "$real_names" | wc -l) tools)"
exit $rc
