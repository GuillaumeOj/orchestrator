#!/bin/bash
# usage: run.sh <run-name> <driver-script.json> <settings.json> [extra claude args...]
# Spawns `claude` in a PTY with a scrubbed env (as the app would), per-agent --settings, OTEL to a local receiver.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${SPIKE_ROOT:?set SPIKE_ROOT}"
NAME=$1; SCRIPT=$2; SETTINGS=$3; shift 3
OUT="$ROOT/runs/$NAME"; WS="${SPIKE_WS:-$ROOT/ws}"
mkdir -p "$OUT" "$WS"
cp "$SETTINGS" "$OUT/settings.json"
[ -f "$OUT/policy.json" ] || echo '{}' > "$OUT/policy.json"
cd "$WS"
exec env -i HOME="$HOME" USER="$USER" LOGNAME="$USER" SHELL=/bin/zsh LANG=en_US.UTF-8 TERM=xterm-256color \
  PATH="$HOME/.local/bin:$HOME/.nvm/versions/node/v24.15.0/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  SPIKE_OUT="$OUT" SPIKE_POLICY="$OUT/policy.json" ORCH_AGENT_ID="agent-$NAME" \
  CLAUDE_CODE_ENABLE_TELEMETRY=1 OTEL_METRICS_EXPORTER=otlp OTEL_LOGS_EXPORTER=otlp \
  OTEL_EXPORTER_OTLP_PROTOCOL=http/json OTEL_EXPORTER_OTLP_ENDPOINT="http://127.0.0.1:${OTLP_PORT:-4318}" \
  DISABLE_AUTOUPDATER=1 OTEL_METRIC_EXPORT_INTERVAL=5000 OTEL_LOGS_EXPORT_INTERVAL=2000 \
  ${SPIKE_EXTRA_ENV:-} \
  python3 "$HERE/ptydrive.py" "$SCRIPT" "$OUT" -- claude --settings "$OUT/settings.json" "$@"
