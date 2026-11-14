# Allow User to Fine Tune Sandbox Permissions · UX brief

> For `ux` (flows, copy, states) then `ui` (Figcraft screens in the Drive `Orchestrator.figcraft`, exports under `design/`). Companions: [spec.md](spec.md), [api-contract.md](api-contract.md).

## The user

Runs teams of agents. Today learns about sandbox failures from Overwatch messages ("back can't run the socket tests", "front can't reach api.github.com", "git fetch fails") and has no button to fix them. Opened `policy.toml`, found `[[rule]] match = "Bash(…)"` and didn't understand it — and rules couldn't have fixed it anyway. Goal: **from a blocked agent to "allowed" in one decision, and a settings page where what is allowed is readable without knowing TOML.**

## Two concepts, one sentence each

The copy must make the difference obvious wherever both appear:
- **Permissions**: which tool calls need someone's approval (rules: Allow / The Overwatch decides / Ask me / Deny).
- **Sandbox**: what an agent's shell commands can reach at all (internet hosts, folders to write, local ports, sockets). Nothing in Permissions can widen it.

## Surfaces

### 1. The `sandbox` card (Inbox, and wherever permission cards show today)
- Title in plain words by kind: "back wants to reach **api.github.com**" / "…to write **~/.cache/ms-playwright**" / "…to open local ports (test servers)" / "…to use the socket **/tmp/x.sock**".
- The agent's reason, quoted. Project · role.
- Primary: **Allow for this project**. Secondary choice (menu or segmented): Allow everywhere · Allow for role `back` only. Then Reject, Reply.
- A line under the actions: what happens next ("back restarts and retries"). For `network` on `*.github.com`: "Without your GitHub login: issues and PRs still go through the orchestrator."
- Invalid value (never allowed): the reason in place of Allow; Reject / Reply only.
- Decided state: "Allowed for project X by you · Undo" (Undo = remove the entry).

### 2. Settings → Sandbox (new section in the Settings window)
- Header + one-sentence explainer (above), link "What can't be allowed" (expands the fixed limits: credentials, other worktrees, commands outside the sandbox, push/PR/commit go through the orchestrator).
- Scope switch or list: **All projects**, then one group per project.
- Per scope, grouped by kind (Internet hosts · Folders Bash can write · Local ports · Sockets). Each row: value, role chip (if any), note (muted), Remove. Built-in rows (package registries, role hosts) are visible, muted, labelled "built in", no Remove.
- **Add** (inline form or small sheet): kind, value (placeholder per kind), live validation message, scope, role (optional, from the project's team roles), note. Example chips for common needs. After adding: "Applies when an agent next starts · 2 agents running" (or "Applies now" when the core says `live`).
- States: loading, file parse error ("policy.toml line 4: …, using the last good version" + Open in editor), warnings on skipped entries, empty scope ("Nothing extra: agents get the built-in hosts only").
- "Open in editor" stays available for power users (it's the same file as rules).

### 3. Settings → Permissions: rules list and editor
- Keep files, defaults and hold as they are; add a **Rules** block per file: each rule as a sentence ("Bash commands starting with `cargo test`") + effect chip + role chip + note + Remove. The raw pattern shown small/monospace on hover or second line.
- **Add rule**: "When an agent wants to…" [tool picker: run a shell command / read a file / edit a file / fetch a web page / use an MCP tool / anything] + value + "…then" [effect] + scope + role + note. Live preview: generated pattern + sentence (from `rule_preview`).
- **Test a call**: pick role, tool, type an input → "Allowed by rule r12 (project file)" / "Asks you (default for solo sessions)".
- A hint linking to Sandbox: "Command failing with 'Operation not permitted' or a blocked host? That's the sandbox, not a permission."

## Open questions for ux to settle (decide, don't escalate unless it's product scope)
- Sandbox as its own Settings section vs. a tab inside Permissions (the brief suggests its own section; keep the explainer either way).
- How the scope choice on the card is presented without making the primary action ambiguous.
- Whether the decided card's Undo is worth it (it's a remove).
