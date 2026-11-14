# Allow User to Fine Tune Sandbox Permissions · Feature spec

> Feature `allow-fine-tune-sandbox-permissions` · Integration branch `orch/allow-fine-tune-sandbox-permissions/integration` · Core + MCP + UI + design. Companions: [ux-brief.md](ux-brief.md), [api-contract.md](api-contract.md).

## 1. Problem

The user gets Overwatch messages about agents that can't run tests, can't reach GitHub, can't run some git commands. They have no way to say "allow this". They tried the policy file, but:

- `policy.toml` only decides **whether a tool call needs approval** (permission rules). It cannot widen the **OS sandbox** that Bash runs in (network hosts, folders Bash may write, local ports, Unix sockets). Sandboxed Bash is auto-approved, so the failures the user sees are sandbox failures that no rule can fix.
- The `[[rule]]` syntax (`match = "Bash(cargo test:*)"`, effects, scopes) is undocumented in the app; Settings → Permissions only offers "Open in editor".

## 2. Today (HEAD 540838fe)

- **Sandbox** (`crates/agent-runtime/src/compile.rs` `agent_rules` / `compile`): every agent's `settings.json` gets `sandbox.network.allowedDomains = role.network_allow` (package registries `role::DEFAULT_NETWORK_ALLOW`, plus a team node's `network_allow`, which no UI edits), `filesystem.allowWrite` = the git common dir + `role.extra_write` (always empty) + project files with write access, fixed `denyRead`/`denyWrite`. No `allowLocalBinding`, no `allowUnixSockets`. Nothing the user can change, globally or per project.
- **Hard lists** (`policy/hard.rs`): WebFetch / curl / wget to a host outside `network_allow` is user-only.
- **Policy files** (`policy/file.rs`): `<app data>/policy.toml` and the project's `.orchestrator/policy.toml` (or `<app data>/projects/<id>/policy.toml`), `[[rule]]` + `[defaults]`. Rules are added from cards ("Always allow", rule proposals) and removed from chat lines. `PermissionsSettings.svelte` lists the files with Open in editor, the defaults and the hold; it has no rule list and no rule editor (P4.4 notes §9.2).

## 3. Scope

### 3.1 Sandbox allowances (new)

A **sandbox allowance** widens what an agent's Bash can reach. Four kinds:

| Kind | Value | Compiled into |
|---|---|---|
| `network` | a host (`api.github.com`) or a wildcard (`*.githubusercontent.com`) | `sandbox.network.allowedDomains` (and the hard list's `network_allow`) |
| `write` | an absolute folder (`~` allowed) | `sandbox.filesystem.allowWrite` |
| `local_ports` | — (on/off) | `sandbox.network.allowLocalBinding: true` |
| `unix_socket` | an absolute socket path | `sandbox.network.allowUnixSockets` |

Each allowance has an optional `role` (only agents of that role id get it) and an optional `note` (why). Allowances live in the **same policy files** as rules, as `[[sandbox]]` tables (format in the api-contract §1): global (all projects) or project (shared with the repo when `.orchestrator/` is tracked). The user never has to write them by hand: the UI adds and removes them. Hand edits work too (the files are watched, same parse-error handling as rules).

**Effective sandbox of an agent** = core defaults (package registries, the git dir, its worktree) + its role's `network_allow` + global allowances + its project's allowances, keeping only the entries whose `role` is absent or equals the agent's role. Allowances only **add**; every core deny stays.

**Never allowed, whatever the file says** (validated on add, and an invalid hand-written entry is skipped with a warning on the file, like a neutralised rule):
- reading or writing a credential store, the app's `state/` or app data, the main checkout, other worktrees;
- writing a path that holds code run outside the sandbox (`project_files::UNSANDBOXED`, `UNSANDBOXED_SYSTEM`, any `.git` or `.claude` component), `/`, `~` itself, a system folder;
- `network = "*"` (all hosts) or an empty host;
- running commands outside the sandbox (`excludedCommands`, `allowUnsandboxedCommands` stay off);
- the hard lists: `git push`, `gh pr create`/`merge`, `gh` writes, destructive git stay as today. GitHub calls that need the user's credentials keep going through the core's tools (`read_issue`, `update_issue`, `request_pr`); an allowance for `api.github.com` only gives unauthenticated access, and the UI says so.

**When it applies**: the core recompiles an agent's `settings.json` at each spawn and resume. On a change, every agent in scope gets its settings rewritten; if the installed CLI reloads `--settings` live (back verifies with 2.1.317), that is enough; otherwise the change applies at the agent's next start and the UI says so ("Applies when an agent next starts. N agents running"). An approved request (3.2) is the exception: the requesting agent is restarted with `--resume` at the end of its turn and told to retry. If it keeps working after being told to stop, the restart waits for its Stop; several approvals before that Stop make one restart (api-contract §5).

**Supported keys**: back checks each compiled key (`allowLocalBinding`, `allowUnixSockets`) against the installed CLI's settings schema. A kind the CLI doesn't support is not offered (the UI hides it) and the reason is noted in the README.

### 3.2 Agents ask for an allowance (new)

New orchestrator MCP tool **`request_sandbox_access(kind, value, why)`**, given to every agent (team roles and solo). The preamble says: "If the sandbox blocks a host, a folder, a local port or a socket you need, ask with mcp__orch__request_sandbox_access, then stop and wait." Instead of the agent explaining the failure to the Overwatch or the lead, the user gets **one card**:

- Inbox item kind `sandbox` (the user decides, never the Overwatch: widening the sandbox is a security call). Shows agent, project, role, the kind and value, the agent's reason, and a validation error if the value is never allowed (then only Reject / Reply).
- Actions: **Allow for this project** (default), **Allow everywhere**, **Allow for this role only** (project file, `role` set), **Reject**, **Reply**. Allow writes the entry (note = the agent's `why`), is audited (FR-709), then the agent is resumed with "ORCH: approved: <kind> <value> is allowed, retry". Reject/Reply send "ORCH: denied: …" with the user's words.
- A pending identical request (same agent, kind, value) is not duplicated; a value already allowed for that agent answers at once ("already allowed; restart pending" when only the restart is missing).

### 3.3 A readable editor (new UI)

**Settings → Sandbox** (new section, next to Permissions):
- An explainer line: Permissions decide which calls need approval; the Sandbox decides what Bash commands can reach.
- Per scope (Global, then each project): the allowances grouped by kind, each with its value, role, note, file of origin, Remove. The core defaults (package registries) and a role's built-in hosts are listed read-only so the user sees the whole effective list.
- **Add**: kind picker, value field with inline validation (same rules as the core, the core's answer is final), scope (global / project), optional role, optional note. Example chips for common needs (`github.com`, `api.github.com`, `*.githubusercontent.com`, local ports).
- A **known limits** note: commits (`mcp__orch__commit`), pushes and PRs (`request_pr`), issue reads/updates go through the orchestrator; no allowance changes them.

**Settings → Permissions** gets a **rules list and editor** (reverses the P4.4 "no rule editor" note, because the brief says the syntax is unclear):
- Per file, the rules in plain language ("Bash commands starting with `cargo test` → Allow · role back · note"), with Remove (`revoke_rule`).
- **Add rule**: tool picker (Bash command, Read path, Edit path, WebFetch domain, MCP tool, any), the matching value, a live preview of the generated pattern (`Bash(cargo test:*)`) and its meaning, effect (Allow / The Overwatch decides / Ask me / Deny), scope (global / project / role), note.
- **Test a call**: tool + input → the effect and what decides it (`policy_test`).
- New policy files start with a commented header that documents both `[[rule]]` and `[[sandbox]]` with one example each.

### 3.4 Out of scope

Per-agent allowances; read allowances outside the worktree (project files, FR-112, already do it); commands outside the sandbox; giving agents GitHub credentials; a temporary (time-limited) allowance; parsing sandbox violations out of tool output automatically (the agent asks instead).

## 4. Acceptance

1. A `[[sandbox]]` network entry in the project file makes the next spawned agent of that project carry the host in `sandbox.network.allowedDomains`, and `curl https://<host>` from its Bash is not user-only any more; an agent of another project does not get it.
2. `write`, `local_ports`, `unix_socket` entries compile into `allowWrite`, `allowLocalBinding`, `allowUnixSockets`; a `role` entry reaches only that role's agents (unit tests on `compile`).
3. Adding a forbidden value (a credential store, `~/.claude`, a `.git/hooks` path, `*`, the main checkout) is refused with a reason, from the UI and from the card; a hand-written one is skipped with a warning shown on the file row.
4. `request_sandbox_access` creates one `sandbox` card; Allow for this project writes the entry to the project file with the note, logs it, and the agent continues after the restart with the host reachable; Reject tells the agent; the Overwatch never decides it.
5. Settings → Sandbox lists defaults (read-only) and allowances per scope, adds and removes them; Settings → Permissions lists rules in plain language, adds one through the form (the generated pattern is shown before saving), removes one, and tests a call.
6. Every add/remove (UI, card, file watcher) refreshes the open settings (event `sandbox_changed` / existing policy events) and is in the audit log.
7. `cargo test --workspace`, `cargo clippy`, and in `ui/` `bun run test`, `bun run check`, `bun run lint` pass. README (agent-runtime Compile and Inbox sections) and SPEC FR-702 / FR-1308 updated.
8. Figcraft: the Sandbox section, the Permissions rules editor and the `sandbox` card are in the Drive `Orchestrator.figcraft`; exports under `.orchestrator/features/allow-fine-tune-sandbox-permissions/design/`.

## 5. Tasks (after approval)

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| UX: flows, copy, states for Settings → Sandbox, the rules editor, the `sandbox` card | ux | — | From [ux-brief.md](ux-brief.md); hands off to ui |
| UI: Figcraft screens + exports | ui | ux | Drive `Orchestrator.figcraft` only |
| Core: `[[sandbox]]` parse/write/validate, effective sandbox in `compile` and the hard list, live rewrite or restart, ctl + Tauri commands, events, audit, README/SPEC | back | — | `policy/`, `compile.rs`, `role.rs`, `orch-protocol`, `orch-app/commands.rs` |
| Core: `request_sandbox_access` MCP tool, `sandbox` Inbox kind and its decide actions, resume-and-retry | back | core task above | `orch-mcp`, `inbox.rs`, preamble |
| Front: Settings → Sandbox, Permissions rules editor, `sandbox` card | front | ui, back core tasks | `ui/src/components/*Settings*.svelte`, Inbox card components, tests; asks ui for review |
