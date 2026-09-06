#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
baseline_file="$repo_root/scripts/warning_budget.txt"

usage() {
  cat <<'USAGE'
Usage:
  scripts/check_warning_budget.sh            # fail if warnings exceed baseline
  scripts/check_warning_budget.sh --update   # update baseline to current warning count

Notes:
  - Counts Rust compiler lines that begin with "warning:" from `cargo check -q`
  - Baseline is stored in scripts/warning_budget.txt
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ ! -f "$baseline_file" ]]; then
  echo "error: missing baseline file: $baseline_file" >&2
  exit 1
fi

if ! command -v cargo > /dev/null 2>&1; then
  echo "error: cargo not found" >&2
  exit 1
fi

# Keep the compiler status separate from warning counting. A failed build can
# emit no warnings, and must never become a successful zero-warning check.
check_output=$(mktemp)
trap 'rm -f "$check_output"' EXIT
check_status=0
(cd "$repo_root" && CARGO_TERM_COLOR=never cargo check -q) > "$check_output" 2>&1 || check_status=$?
if (( check_status != 0 )); then
  cat "$check_output" >&2
  echo "error: cargo check failed with exit status $check_status" >&2
  exit "$check_status"
fi
# POSIX awk returns a count of zero successfully when there are no warnings;
# unlike grep -c, it needs no fallback that could conceal a command failure.
current=$(awk '/^warning:/ { count += 1 } END { print count + 0 }' "$check_output")
baseline=$(tr -d '[:space:]' < "$baseline_file")

if [[ "${1:-}" == "--update" ]]; then
  printf '%s\n' "$current" > "$baseline_file"
  echo "Updated warning baseline: $baseline"
  echo "New warning baseline: $current"
  exit 0
fi

if ! [[ "$baseline" =~ ^[0-9]+$ ]]; then
  echo "error: invalid warning baseline in $baseline_file: '$baseline'" >&2
  exit 1
fi

if (( current > baseline )); then
  echo "Warning budget exceeded: current=$current baseline=$baseline" >&2
  echo "Run scripts/check_warning_budget.sh --update only after intentional cleanup." >&2
  exit 1
fi

if (( current < baseline )); then
  echo "Warning budget improved: current=$current baseline=$baseline"
  echo "Consider running: scripts/check_warning_budget.sh --update"
else
  echo "Warning budget OK: current=$current baseline=$baseline"
fi
