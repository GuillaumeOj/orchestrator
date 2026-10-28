#!/bin/sh
# A stand-in for `claude` (no quota): runs the real orch-hook/orch-status the
# way Claude Code does, one turn per pasted line. Test fixture only.
if [ "$1" = "--version" ]; then echo "2.1.302 (Claude Code)"; exit 0; fi
SID=""
while [ $# -gt 0 ]; do
  case "$1" in
    --session-id|--resume) SID="$2"; shift ;;
  esac
  shift
done
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
hook() { printf '%s' "$1" | "$HOOK"; }
status() {
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
  n=$((n+1))
  raw=$(printf '%s' "$line" | tr -d '\033\r' | sed 's/\[200~//; s/\[201~//')
  prompt=$(printf '%s' "$raw" | sed 's/"/'"'"'/g')
  printf '{"type":"user","message":{"role":"user","content":"%s"}}\n' "$prompt" >> "$TR/$SID.jsonl"
  hook "{\"hook_event_name\":\"UserPromptSubmit\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"prompt\":\"$prompt\"}"
  out=$(hook "{\"hook_event_name\":\"PreToolUse\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"echo hi\"},\"tool_use_id\":\"tu$n\"}")
  echo "PRE[$n]:$out"
  case "$prompt" in
    *SLOW*) sleep 30 ;;
    *COMMIT*) echo "work $n" > "work-$n.txt"; echo "MCP[$n]:$(mcp_commit "fake commit $n")" ;;
    *ORCHTOOL*)
      name=$(printf '%s' "$raw" | sed -n 's/.*ORCHTOOL \([a-z_]*\) .*/\1/p')
      args=$(printf '%s' "$raw" | sed -n 's/.*ORCHTOOL [a-z_]* \(.*\)$/\1/p')
      case "$raw" in *WRITE*) echo "work $n" > "work-$n.txt" ;; esac
      echo "MCP[$n]:$(mcp_call "$name" "$args")" ;;
  esac
  post=$(hook "{\"hook_event_name\":\"PostToolUse\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"tool_name\":\"Bash\"}")
  [ -n "$post" ] && echo "POST[$n]:$post"
  cost="0.0$n"
  status $((10 + n)) "$cost"
  stop=$(hook "{\"hook_event_name\":\"Stop\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"background_tasks\":[]}")
  # A blocked Stop keeps the turn going once, like Claude Code.
  case "$stop" in
    *'"block"'*)
      echo "BLOCK[$n]:$stop"
      hook "{\"hook_event_name\":\"Stop\",\"session_id\":\"$SID\",\"prompt_id\":\"p$n\",\"stop_hook_active\":true,\"background_tasks\":[]}" ;;
  esac
  echo "DONE[$n] $prompt"
done
hook "{\"hook_event_name\":\"SessionEnd\",\"session_id\":\"$SID\",\"reason\":\"other\"}"
