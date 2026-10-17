# P0 spike: `claude` CLI control and signal channels

Throwaway harness used for [docs/spikes/p0-claude-cli.md](../../docs/spikes/p0-claude-cli.md). This is not product code.

| File | Role |
|---|---|
| `run.sh` | Spawns `claude` in a PTY with a scrubbed env (`env -i`), as the app would: per-agent `--settings`, OTEL to a local receiver, `DISABLE_AUTOUPDATER=1`. |
| `ptydrive.py` | PTY driver. Runs a JSON step script (paste, type, keys, wait for a hook event, measure memory, send a signal). |
| `hook.py` | Hook command for every event. Logs payloads to `hooks.jsonl` and answers using `policy.json` (PreToolUse allow/deny with delay, PostToolUse `additionalContext`, Stop "continue", PermissionRequest). |
| `statusline.py` | Statusline command. Logs the JSON to `statusline.jsonl`. |
| `otlp_receiver.py` | Minimal OTLP/HTTP-JSON receiver. Logs to `otlp.jsonl`. |
| `mcp_mock.py`, `mcp.json` | Stub `orch-mcp` stdio server with one `whoami` tool, which reads the agent id from the env. |
| `screen.py` | Renders a `pty.raw` capture to the final screen text (needs `pyte`). |
| `settings.*.json` | Per-agent settings variants (base, 30 s hook timeout, statusline refresh, sandbox variants). They contain absolute paths to this checkout. |
| `samples/` | Scrubbed sample payloads (hooks, statusline, OTEL). |

Run outputs go to `out/`, a gitignored symlink to a scratch dir. Example:

```bash
SPIKE_ROOT=/path/to/scratch ./run.sh run1 /path/to/steps.json settings.base.json --model haiku --session-id "$(uuidgen | tr A-Z a-z)"
```
