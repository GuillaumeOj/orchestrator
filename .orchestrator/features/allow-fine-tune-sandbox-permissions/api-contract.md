# Allow User to Fine Tune Sandbox Permissions · API contract

> Between `back` (agent-runtime, orch-protocol, orch-mcp, orch-app commands) and `front` (ui). Companion of [spec.md](spec.md). JSON shapes are what the Tauri commands return; the ctl requests carry the same fields.

## 1. `policy.toml`: `[[sandbox]]` tables

```toml
version = 1

[[rule]]
id = "r12"
match = "Bash(cargo test:*)"
effect = "allow"
role = "back"            # optional
note = "tests"           # optional

[[sandbox]]
id = "s3"                # written by the core (s<n>, unique per file); optional by hand
network = "api.github.com"
role = "back"            # optional: only agents of this role id
note = "public GitHub API for the release check"   # optional

[[sandbox]]
write = "~/.cache/ms-playwright"

[[sandbox]]
local_ports = true

[[sandbox]]
unix_socket = "/tmp/my-test.sock"
```

- Exactly one of `network` (string), `write` (string), `local_ports` (`true`), `unix_socket` (string) per table. Otherwise a parse error at the table's line (same handling as rules: last good version kept, one `policy_parse` alert).
- A table whose value is never allowed (spec §3.1) parses but is **skipped** and listed in the file's `warnings` (`{id, message}`), like a neutralised rule.
- `~` is expanded to the user's home; paths are canonicalised at compile time (symlinks resolved) and re-validated at each spawn (like project files).
- Written with `toml_edit` (comments and order kept), under the file's write lock.
- `HEADER` (new files) gains a commented example of a rule and of each sandbox kind.

## 2. Effective sandbox

`effective_sandbox(agent)` (core, used by `compile` and by the hard list):
- `network`: `DEFAULT_NETWORK_ALLOW` ∪ role `network_allow` ∪ matching entries' hosts (deduplicated, order: defaults, role, global, project).
- `allowWrite`: today's list ∪ matching `write` entries.
- `allowLocalBinding`: `true` when a matching `local_ports` entry exists (key absent otherwise).
- `allowUnixSockets`: matching `unix_socket` paths (key absent when empty).
- Matching = global entries + entries of the agent's project; `role` absent or equal to the agent's role id. The Overwatch never gets any.
- Hard list `network_allow` uses the same network list.

## 3. Commands (Tauri `invoke` = ctl request)

### `sandbox_list {project?}` → `SandboxScope[]`
Global first, then each registered project (only `project` when given).
```json
{
  "scope": "global" | "project",
  "project": "p-id",            // project scope only
  "file": "global" | "project:p-id",
  "path": "/…/policy.toml",
  "exists": true,
  "error": {"line": 4, "message": "…"},   // when the file doesn't parse
  "warnings": [{"id": "s2", "message": "~/.ssh is a credential store: never allowed"}],
  "entries": [
    {"id": "s3", "kind": "network", "value": "api.github.com", "role": "back", "note": "…", "source": "file"}
  ],
  "defaults": [                  // global scope only: read-only, what every agent gets
    {"kind": "network", "value": "registry.npmjs.org", "source": "core"}
  ]
}
```
`kind`: `network` | `write` | `local_ports` | `unix_socket` (`value` is `null` for `local_ports`). `source`: `core` (defaults), `role:<role id>` (a role's `network_allow`, listed under the project scope when a team uses it, optional), `file`.

### `sandbox_kinds` → `["network", "write", "local_ports", "unix_socket"]`
The kinds the installed CLI supports (spec §3.1 "Supported keys"); the UI offers only these.

### `sandbox_check {kind, value, scope, project?}` → `{ok: true, value: "<normalised>"} | {ok: false, reason: "…"}`
Validation only (no write), for inline feedback while typing. `reason` is a full sentence for the user.

### `sandbox_add {scope, project?, kind, value?, role?, note?}` → `{entry, file, path, applies: "live" | "next_start", running: <n>}`
Validates (same as `sandbox_check`, error = the reason), appends, audits (actor `user`, action `sandbox_add`), emits `sandbox_changed`. A duplicate (same kind, value, role in the same file) returns the existing entry. `running`: agents in scope currently running.

### `sandbox_remove {id, file}` → `{removed: entry, applies, running}`
Audited (`sandbox_remove`), emits `sandbox_changed`.

### Rules (existing, extended)
- `policy_rules {project?}` (existing) → each rule gains `"summary"`: a plain-language sentence built by the core ("Bash commands starting with `cargo test`").
- `add_rule {scope, scope_id, pattern, effect, note?}`: `scope` also accepts `global` (`scope_id` ignored); `note` written to the rule.
- `rule_preview {tool, value}` → `{pattern, summary}` | error: builds the pattern from the form's tool and value (`Bash` + `cargo test` → `Bash(cargo test:*)`; `WebFetch` + `docs.rs` → `WebFetch(domain:docs.rs)`; `Read`/`Edit` + a path → `Read(//abs/**)`; `mcp` + `server__tool`). The UI never builds patterns itself.
- `revoke_rule`, `policy_test`, `policy_files` unchanged.

## 4. Events

- `sandbox_changed {file}`: after an add, a remove, a decided card, or a file reload that changed `[[sandbox]]`. The settings reload `sandbox_list`.
- Existing policy reload events keep refreshing the rules list (front: reload `policy_rules` on them).

## 5. MCP tool `request_sandbox_access`

Input: `{kind: "network" | "write" | "local_ports" | "unix_socket", value?: string, why: string}` (`why` required, ≤ 500 chars).

Core:
1. Validates. Never-allowed → the tool returns an error with the reason (no card).
2. Already in the agent's effective sandbox → returns `"already allowed"` (+ `"; it applies when you restart: stop your turn"` when its running settings don't have it yet, and the core restarts it at Stop as in step 4).
3. Same pending request (agent, kind, value) → returns "already asked; stop and wait".
4. Else creates Inbox item kind **`sandbox`**, priority 30, payload `{kind, value, why, project, role, agent}`, never routed to the Overwatch, and returns "ORCH: asked the user; stop your turn and wait". The agent is parked at Stop like a parked permission (no hold: the tool returns at once).

Decide (`decide(item, decision, scope, message)`):
- `Approve` with `scope`: `project` (default) | `global` | `role` (project file, `role` = agent's role) → `sandbox_add` (note = `why`, audited actor `user`), then the agent is restarted with `--resume` on the recompiled settings (or not, when the CLI reloads live) and gets "ORCH: approved: <kind> <value> is now allowed by the sandbox. Retry.".
- `Reject` → "ORCH: denied: sandbox access to <kind> <value>." ; `Reply` → the same with the user's words.
- Inbox item shape for front: same as other items, `kind: "sandbox"`, `payload` above, plus `invalid?: string` when validation fails at decision time (then Approve is refused).

Preamble (solo and team) gains: "If the sandbox blocks a host, a folder, a local port or a socket you need, ask with mcp__orch__request_sandbox_access (say why), then stop and wait."

`request_sandbox_access` is added to `SOLO_TOOLS` and to every team role's default `orch_tools` (starter roles and compiled nodes), never to `OVERWATCH_TOOLS`.
