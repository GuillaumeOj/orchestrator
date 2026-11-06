#!/bin/sh
# A stand-in for `claude` (no quota): runs the real orch-hook/orch-status the
# way Claude Code does, one turn per pasted line. Test fixture only.
if [ "$1" = "--version" ]; then echo "2.1.302 (Claude Code)"; exit 0; fi
SID=""
RESUMED=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id) SID="$2"; shift ;;
    --resume) SID="$2"; RESUMED=1; shift ;;
  esac
  shift
done
# Test sentinels in the worktree (FR-415): a start that dies at once, a
# resume that dies at once, a start stuck before SessionStart.
if [ -f .fake-start-dies ] || { [ -n "$RESUMED" ] && [ -f .fake-resume-dies ]; }; then
  echo "FAKE CLAUDE: no conversation found with session ID: $SID"
  exit 1
fi
if [ -f .fake-no-session-start ]; then
  echo "FAKE CLAUDE: a dialog nobody answers"
  while true; do sleep 1; done
fi
ROOT=$(dirname "$ORCH_SOCK")
HOOK="$ROOT/bin/orch-hook"
STATUS="$ROOT/bin/orch-status"
MCP="$ROOT/bin/orch-mcp"
# A pasted "COMMIT <msg>" writes a file and commits it through orch-mcp, the
# way the model calls mcp__orch__commit.
mcp_commit() {
  printf '%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"commit\",\"arguments\":{\"message\":\"$1\"}}}" \
    | "$MCP" | tail -1
}
# A pasted "ORCHTOOL <name> <json>" calls that orchestrator tool through
# orch-mcp (the agent's ORCH_TOOLS apply), like the model would.
mcp_call() {
  printf '%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" \
    | "$MCP" | tail -1
}
# Chaos switches (spec §3.8): sentinel files in the worktree, set per test.
# "Consumed" ones are removed when used, so they hit one turn only.
#   .fake-swallow-enter    the next Enter is lost: the pasted text sits in the box
#   .fake-modal            a dialog eats every key until a lone Esc (not consumed: the test removes it)
#   .fake-drop-hooks       the next turn's hooks never reach the core (the transcript has it)
#   .fake-drop-stop        the next turn's Stop never reaches the core
#   .fake-stop-failure     the next turn ends in StopFailure <content: error>
#   .fake-stop-failure-sticky   every turn does (not consumed)
#   .fake-bg               the next Stop carries a running background task
#   .fake-exit-mid-turn    the process dies after UserPromptSubmit (consumed)
#   .fake-crash-always     ... on every turn (not consumed)
#   .fake-delay-post       <secs>: the next PostToolUse waits that long
#   .fake-subagent         the next PostToolUse comes from a subagent
#   .fake-read-full        the model reads any "full text: <path>" it is shown (not consumed)
#   .fake-usage            <pct>: the statusline's 5 h usage (not consumed)
DROP=""
hook() { [ -n "$DROP" ] && return 0; printf '%s' "$1" | "$HOOK"; }
# Takes a consumed switch: succeeds (and removes it) when it is set.
take() { [ -f "$1" ] && { rm -f "$1"; return 0; }; return 1; }
status() {
  [ -n "$DROP" ] && return 0
  [ -f .fake-usage ] && set -- "$(cat .fake-usage)" "$2"
  printf '{"session_id":"%s","rate_limits":{"five_hour":{"used_percentage":%s,"resets_at":1792489800},"seven_day":{"used_percentage":34,"resets_at":1792983600}},"cost":{"total_cost_usd":%s}}' \
    "$SID" "$1" "$2" | "$STATUS" >/dev/null
}
# A transcript like Claude Code's (by session id), in a test-only folder.
TR="$ROOT/fake-projects/-fake"
mkdir -p "$TR"
echo "FAKE CLAUDE $SID env-api-key=${ANTHROPIC_API_KEY:-unset} autoupdate=${DISABLE_AUTOUPDATER:-unset}"
hook "{\"hook_event_name\":\"SessionStart\",\"session_id\":\"$SID\",\"source\":\"startup\"}"
status 0 0
n=0
cost=0
while IFS= read -r line; do
  # A bracketed paste is one message, whatever its newlines (like Claude Code).
  case "$line" in
    *'[200~'*)
      while ! printf '%s' "$line" | grep -q '\[201~'; do
        IFS= read -r more || break
        line="$line $more"
      done ;;
  esac
  # The Enter was lost: the text stays in the box until another Enter comes.
  if take .fake-swallow-enter; then IFS= read -r _again; fi
  # A modal ate the keys: nothing is read as input until a lone Esc.
  if [ -f .fake-modal ]; then
    echo "FAKE MODAL: press Esc"
    stty -echo -icanon min 0 time 1
    while :; do
      b=$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' ')
      [ "$b" = "1b" ] || continue
      [ -z "$(dd bs=1 count=1 2>/dev/null | od -An -tx1 | tr -d ' ')" ] && break
    done
    stty sane
    echo "FAKE MODAL: closed"
    continue
  fi
  n=$((n+1))
  raw=$(printf '%s' "$line" | tr -d '\033\r' | sed 's/\[200~//; s/\[201~//')
  [ -z "$raw" ] && { n=$((n-1)); continue; }
  prompt=$(printf '%s' "$raw" | sed 's/"/'"'"'/g')
  take .fake-drop-hooks && DROP=1
  printf '{"type":"user","message":{"role":"user","content":"%s"}}\n' "$prompt" >> "$TR/$SID.jsonl"
  hook "{\"hook_event_name\":\"UserPromptSubmit\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"prompt\":\"$prompt\"}"
  if take .fake-exit-mid-turn || [ -f .fake-crash-always ]; then echo "FAKE CLAUDE: crashed"; exit 1; fi
  out=$(hook "{\"hook_event_name\":\"PreToolUse\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"echo hi\"},\"tool_use_id\":\"tu$n\"}")
  echo "PRE[$n]:$out"
  # A pasted "PERMIT <cmd>" (or the core's "retry the call Bash <cmd>") asks
  # for a permission the way Claude Code's dialog would. Several queued turns
  # come in one paste, joined by "---": a command ends at the next turn.
  permit=""
  case "$raw" in
    *'retry the call Bash '*) permit=$(printf '%s' "$raw" | sed 's/.*retry the call Bash //; s/ *---.*$//') ;;
    *PERMIT*) permit=$(printf '%s' "$raw" | sed 's/.*PERMIT //; s/ *---.*$//') ;;
  esac
  if [ -n "$permit" ]; then
    out=$(hook "{\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"$permit\"},\"tool_use_id\":\"tp$n\",\"cwd\":\"$PWD\"}")
    echo "PERM[$n]:$out"
  fi
  case "$prompt" in
    *SLOW*) sleep 30 ;;
    # The Overwatch's turn carrying "permission inbox-<n>" of a call marked
    # OWALLOW: it allows it inside that turn, like its model would.
    *OWALLOW*)
      d=$(printf '%s' "$raw" | sed -n 's/.*permission \(inbox-[0-9]*\).*/\1/p')
      # In a variable first: bash brace-expands `{a,b}` in a command's words.
      args="{\"decision\":\"$d\",\"effect\":\"allow\",\"reason\":\"routine install in the task\"}"
      [ -n "$d" ] && echo "MCP[$n]:$(mcp_call decide_permission "$args")" ;;
    *COMMIT*) echo "work $n" > "work-$n.txt"; echo "MCP[$n]:$(mcp_commit "fake commit $n")" ;;
    *ORCHTOOL*)
      name=$(printf '%s' "$raw" | sed -n 's/.*ORCHTOOL \([a-z_]*\) .*/\1/p')
      args=$(printf '%s' "$raw" | sed -n 's/.*ORCHTOOL [a-z_]* \(.*\)$/\1/p' | sed 's/ *---.*$//')
      case "$raw" in *WRITE*) echo "work $n" > "work-$n.txt" ;; esac
      echo "MCP[$n]:$(mcp_call "$name" "$args")" ;;
  esac
  # The model reads a file the message pointed to ("full text: <path>").
  case "$raw" in
    *'full text: '*)
      if [ -f .fake-read-full ]; then
        path=$(printf '%s' "$raw" | sed 's/.*full text: \([^ ]*\).*/\1/')
        hook "{\"hook_event_name\":\"PostToolUse\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Read\",\"tool_input\":{\"file_path\":\"$path\"}}" >/dev/null
      fi ;;
  esac
  if [ -f .fake-delay-post ]; then sleep "$(cat .fake-delay-post)"; rm -f .fake-delay-post; fi
  sub=""
  take .fake-subagent && sub=',"agent_id":"sub1"'
  post=$(hook "{\"hook_event_name\":\"PostToolUse\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Bash\"$sub}")
  [ -n "$post" ] && echo "POST[$n]:$post"
  cost="0.0$n"
  status $((10 + n)) "$cost"
  printf '{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":"ok"}}\n' >> "$TR/$SID.jsonl"
  if take .fake-drop-stop; then
    DROP=""
  elif [ -f .fake-stop-failure-sticky ] || [ -f .fake-stop-failure ]; then
    err=$(cat .fake-stop-failure-sticky .fake-stop-failure 2>/dev/null | head -1)
    rm -f .fake-stop-failure
    hook "{\"hook_event_name\":\"StopFailure\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"error\":\"$err\",\"last_assistant_message\":\"FAKE $err\"}"
    # The statusline refreshes: the plan reads under 100 % again.
    [ "$err" = rate_limit ] && status 20 0.01
  else
    bg="[]"
    take .fake-bg && bg='[{"id":"b1","type":"shell","status":"running"}]'
    stop=$(hook "{\"hook_event_name\":\"Stop\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"background_tasks\":$bg}")
    # A blocked Stop keeps the turn going once, like Claude Code.
    case "$stop" in
      *'"block"'*)
        echo "BLOCK[$n]:$stop"
        hook "{\"hook_event_name\":\"Stop\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"stop_hook_active\":true,\"background_tasks\":[]}" ;;
    esac
  fi
  DROP=""
  echo "DONE[$n] $prompt"
done
hook "{\"hook_event_name\":\"SessionEnd\",\"session_id\":\"$SID\",\"reason\":\"other\"}"
