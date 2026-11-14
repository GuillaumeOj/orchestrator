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

- `id` absent (hand-written): the core gives it a positional id `s<n>` (n = 1-based index among the file's `[[sandbox]]` tables, suffixed `x` when an explicit id already took it), exactly as rules get `r<n>`. `sandbox_remove` and warnings use it; the file is not rewritten.
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
  "defaults": [                  // read-only, never removable. Global: the core's (what every agent gets).
    {"kind": "network", "value": "registry.npmjs.org", "source": "core"}   // Project: the hosts of its team's roles (`network_allow`),
  ]                                                                          // `source: "role:<role id>"`, `role: "<role id>"`; [] when none
}
```
`kind`: `network` | `write` | `local_ports` | `unix_socket` (`value` is `null` for `local_ports`). `source`: `core` (global defaults), `role:<role id>` (project defaults: a role's `network_allow` for the roles of the team(s) used in that project), `file` (entries).

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
- `add_rule {scope, scope_id, pattern, effect, role?, note?}`. The Settings form uses `scope: "global"` (`scope_id` ignored) or `scope: "project"` (`scope_id` = project id), plus `role` (written as the rule's `role`; absent = any role) and `note`. Existing `role` / `agent` scopes (cards) unchanged. Returns `{rule, file, path}`, the rule with its id (for Undo).
- `rule_preview {tool, value?}` → `{pattern, summary}` | error. `tool`: `bash` | `read` | `edit` | `webfetch` | `mcp` | `any`. Mapping: `bash` + `cargo test` → `Bash(cargo test:*)`; `webfetch` + `docs.rs` → `WebFetch(domain:docs.rs)`; `read`/`edit` + a path → `Read(//abs/**)` / `Edit(//abs/**)` (`~` expanded); `mcp` + `server__tool` → `mcp__server__tool`, + `server` → `mcp__server__*`, + empty → `mcp__*`; `any` (value ignored) → `*` (summary "Any tool call"). The UI never builds patterns itself.
- `policy_test {project?, role?, solo?, tool, value}` (new form, the old `{role, tool, input}` stays for the CLI): `tool` as in `rule_preview` minus `any`; the core builds the input (`bash` → `{command: value}` — a full command, not a prefix; `read`/`edit` → `{file_path}`; `webfetch` → `{url: "https://<value>/"}`; `mcp` → tool name `mcp__<value>`, `{}`). Role: `solo: true` → a solo session; else `role` = the role id, or absent = a team agent of no specific role (role-scoped rules don't match). Returns today's `{tool, input, role, evaluation, hard}`; front renders `evaluation` (effect, deciding rule id/file, or default) and `hard` (core-only).
- `revoke_rule`, `policy_files` unchanged.

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
- Restart timing: an agent with an approved request is restarted at its turn's **Stop** (if it is already stopped, at once). If it keeps working after "stop your turn", the restart waits for Stop (the turn ceiling still applies); nothing interrupts it. Several approvals before Stop **coalesce into one restart** and one message listing every allowance ("ORCH: approved: network api.github.com, local ports … Retry."). A Reject/Reply is delivered as a normal turn and never restarts.
- `Approve` with `scope`: `project` (default) | `global` | `role` (project file, `role` = agent's role) → `sandbox_add` (note = `why`, audited actor `user`), then the agent is restarted with `--resume` on the recompiled settings (or not, when the CLI reloads live) and gets "ORCH: approved: <kind> <value> is now allowed by the sandbox. Retry.".
- `Reject` → "ORCH: denied: sandbox access to <kind> <value>." ; `Reply` → the same with the user's words.
- Inbox item shape for front: same as other items, `kind: "sandbox"`, `payload` above, plus `invalid?: string` when validation fails at decision time (then Approve is refused).
- After an Approve the decided item carries `outcome: {entry: {id, file, …}, created: bool, scope}`: `created: false` when the allowance already existed (duplicate) — front offers Undo (`sandbox_remove {id, file}`) only when `created` is true.

Preamble (solo and team) gains: "If the sandbox blocks a host, a folder, a local port or a socket you need, ask with mcp__orch__request_sandbox_access (say why), then stop and wait."

`request_sandbox_access` is added to `SOLO_TOOLS` and to every team role's default `orch_tools` (starter roles and compiled nodes), never to `OVERWATCH_TOOLS`.
