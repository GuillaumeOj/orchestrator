#!/usr/bin/env bash
# The contract tests: does the installed Claude Code still behave the way the app relies on?
# See docs/app/CONTRACT-TESTS.md.
#
#   scripts/contract.sh [--no-live] [--yes] [--accept]
#
#   --no-live  only the checks that use no plan quota (a run without live checks never records a version)
#   --yes      do not ask before the live checks (they use a few cents of plan quota)
#   --accept   after every check passed, live included, set tested_max in
#              crates/agent-runtime/src/preflight.rs to the installed version
#
# Environment: ORCH_LIVE_CLAUDE_BIN (another claude), ORCH_LIVE_REPO, ORCH_LIVE_ANSWER_TRUST=1
# (first run only: answers Claude Code's "trust this folder" dialog in the scratch repo).
set -u
cd "$(dirname "$0")/.." || exit 1

live=1 yes=0 accept=0
for a in "$@"; do
  case "$a" in
    --no-live) live=0 ;;
    --yes) yes=1 ;;
    --accept) accept=1 ;;
    -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $a (see --help)" >&2; exit 2 ;;
  esac
done

if [ "$accept" = 1 ] && [ "$live" = 0 ]; then
  echo "--accept needs the live checks: drop --no-live" >&2
  exit 2
fi

preflight=crates/agent-runtime/src/preflight.rs
claude=${ORCH_LIVE_CLAUDE_BIN:-$(command -v claude || true)}
if [ -z "$claude" ]; then
  echo "claude not found: install Claude Code (https://claude.com/claude-code) or set ORCH_LIVE_CLAUDE_BIN" >&2
  exit 1
fi
installed=$("$claude" --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -n 1)
if [ -z "$installed" ]; then
  echo "cannot read the version from: $claude --version" >&2
  exit 1
fi
tested=$(sed -n 's/.*tested_max: Version(\([0-9]*\), *\([0-9]*\), *\([0-9]*\)).*/\1.\2.\3/p' "$preflight" | head -n 1)

echo "claude:      $claude"
echo "version:     $installed"
echo "tested_max:  ${tested:-unknown} (the newest version these checks passed on)"

export ORCH_LIVE_CLAUDE=1 ORCH_LIVE_CLAUDE_BIN="$claude"
results=() failed=0

# check <label> <cargo test --test name>: one line per check.
check() {
  local label=$1 test=$2
  echo
  echo "== $label"
  if cargo test -p agent-runtime --test "$test" -- --ignored --nocapture; then
    results+=("PASS  $label")
  else
    results+=("FAIL  $label (the failure above names what changed: see docs/app/CONTRACT-TESTS.md)")
    failed=1
  fi
}

check "flags and auth status (no quota)" contract_cli

if [ "$live" = 1 ]; then
  if [ "$yes" = 0 ]; then
    echo
    echo "The live checks start real Claude Code agents (haiku): a few cents of plan quota."
    printf 'Run them? [y/N] '
    read -r ans
    case "$ans" in y | Y | yes) ;; *) echo "Not run. Use --no-live to skip them, --yes to skip this question."; exit 1 ;; esac
  fi
  check "live agent: hooks, statusline, sandbox, commit, resume" live_claude
  check "live agent: typed slash command" live_command
fi

echo
echo "== Summary (claude $installed)"
printf '%s\n' "${results[@]}"
[ "$live" = 0 ] && echo "SKIP  live checks (--no-live): this run cannot record a version"

if [ "$failed" = 1 ]; then
  echo
  echo "Some checks failed: tested_max is unchanged. Stay on the version that passed" \
    "(claude install ${tested:-<version>}) or fix the app; docs/app/CONTRACT-TESTS.md says where each check is used."
  exit 1
fi

if [ "$accept" = 1 ]; then
  IFS=. read -r ma mi pa <<<"$installed"
  new="tested_max: Version($ma, $mi, $pa)"
  if [ "$installed" = "$tested" ]; then
    echo "tested_max is already $installed: nothing to change."
    exit 0
  fi
  # Never lower tested_max (an older claude passing proves nothing about the newer one).
  if [ "$(printf '%s\n%s\n' "$tested" "$installed" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)" != "$installed" ]; then
    echo "claude $installed is older than tested_max $tested: nothing changed" >&2
    exit 1
  fi
  # Only the value in the SUPPORTED_CLAUDE definition, and only when it is there exactly once.
  pattern='tested_max: Version([0-9]*, *[0-9]*, *[0-9]*)'
  if [ "$(grep -c "$pattern" "$preflight")" != 1 ]; then
    echo "cannot find a single 'tested_max: Version(...)' in $preflight: nothing changed" >&2
    exit 1
  fi
  sed -i.bak "s/$pattern/$new/" "$preflight" && rm -f "$preflight.bak"
  echo
  git --no-pager diff -- "$preflight"
  echo
  echo "Next: commit this change and open a PR. Builds that include it no longer warn about claude $installed."
elif [ "$live" = 1 ]; then
  echo
  echo "All checks passed. To record claude $installed as tested: make contract-accept"
else
  echo
  echo "The no-quota checks passed. Run without --no-live to check the whole contract."
fi
