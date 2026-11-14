# Allow User to Fine Tune Sandbox Permissions · UX flows and copy

> For `ui` (Figcraft screens) and `front`. Inputs: [spec.md](spec.md), [ux-brief.md](ux-brief.md), [api-contract.md](api-contract.md). Reuses `PermissionsSettings.svelte` (file rows, `Select`, `.row`/`.file` blocks, `.err`/`.warn`/`.muted` styles), `SettingsWindow.svelte` (section nav) and the `DecisionCard` (kind line, title, subject, context blocks, actions, marker). Copy is final: sentence case, no trailing period on labels, a period on full sentences. `{…}` marks a value filled in at run time.

## 0. Decisions on the open questions

| Question | Decision | Why |
|---|---|---|
| Sandbox: own section or tab in Permissions? | **Own section "Sandbox", right after "Permissions" in the nav.** The explainer line repeats at the top of both. | The two have different data, different "Add" forms and a different mental model (what needs approval vs what can be reached); a tab would hide the second one and the nav already scales to 10 sections. |
| Scope choice on the card | **One primary button "Allow for this project" and, attached to it, a caret that opens a 2-item menu: "Allow everywhere", "Allow for {role} only".** Reject and Reply stay as separate buttons. | The default is the safe, narrow choice and is one click; wider or narrower choices are visible but can't be hit by mistake, and the primary never changes label. |
| Undo on the decided card | **Yes, "Undo" on the decided line, kept while the entry still exists.** It calls `sandbox_remove`. | An allowance is persistent and security-relevant; a wrong click should be fixable where it happened, not by finding it in Settings. Cost is small: it's the existing remove. |

Other decisions: the kind picker only offers kinds returned by `sandbox_kinds`; "Local ports" has no value field; a project's rows show role-hosts (`source: role:<id>`) read-only under that project; the Add forms are **inline** (an "Add" button expands the form in place, one open at a time), not a modal.

## 1. Vocabulary (use these words everywhere)

- **Sandbox**: what an agent's shell commands can reach at all.
- **Allowance**: one thing added to the sandbox (a host, a folder, local ports, a socket).
- **Rule**: a permission rule (does a tool call need approval).
- Kind labels: `network` → **Internet host**; `write` → **Folder Bash can write**; `local_ports` → **Local ports**; `unix_socket` → **Socket**. Group titles (plural): "Internet hosts", "Folders Bash can write", "Local ports", "Sockets".
- Scope labels: `global` → **All projects**; `project` → **This project** (on the card, in forms) / the project name (in lists); role → **{role} only**.
- Effects: **Allow**, **The Overwatch decides**, **Ask me**, **Deny**.

### Permissions vs Sandbox explainer (final)

Short (top of both sections, and under the "Add" forms):

> Permissions decide which calls need approval. The sandbox decides what an agent's shell commands can reach. A rule can't widen the sandbox.

Hint at the top of Permissions (link = switches to Settings → Sandbox):

> A command fails with "Operation not permitted", or a host is blocked? That's the sandbox, not a permission. **Open Sandbox settings**

Hint at the top of Sandbox (link = Settings → Permissions):

> Want fewer approval prompts instead? That's a permission rule. **Open Permissions settings**

### "What can't be allowed" (final, expandable under the Sandbox explainer; link text "What can't be allowed")

> Some limits stay, whatever you allow:
> - Credentials, the app's own data, the main checkout and other agents' worktrees can't be read or written.
> - Folders that hold code run outside the sandbox (for example `.git`, `.claude`, hooks) can't be written, nor `/`, your home folder itself or system folders.
> - "All hosts" (`*`) can't be allowed.
> - Commands can't run outside the sandbox.
> - Commits, pushes, pull requests and GitHub issues go through the orchestrator (`mcp__orch__commit`, `request_pr`, `read_issue`, `update_issue`). Allowing `api.github.com` only gives unauthenticated access, without your GitHub login.

## 2. Surface 1: the `sandbox` Inbox card

Same card frame as the permission `DecisionCard`: kind line · title · subject · context · actions · marker. Priority 30, never to the Overwatch (so no Overwatch note, no "deciding" ring).

### 2.1 Anatomy and copy

| Element | Copy |
|---|---|
| Kind line | `🛡 Sandbox` (+ ` · parked` when the agent is parked), then the age, as on other cards |
| Title by kind | network: "{agent} wants to reach **{host}**" · write: "{agent} wants to write **{path}**" · local_ports: "{agent} wants to open local ports (test servers)" · unix_socket: "{agent} wants to use the socket **{path}**" |
| Subject line | "{project} · {role}" (solo session: "{project} · solo") |
| Reason (quote block) | the agent's `why`, quoted, as written, clamped to 3 lines with "Show more" |
| Explainer under the reason (muted) | "Widens what {agent}'s shell commands can reach. Permissions rules can't do this." |
| Primary | **Allow for this project** (caret menu: **Allow everywhere** · **Allow for {role} only**; in a solo session the role item is absent) |
| Secondary | **Reject** · **Reply** |
| Line under the actions (muted) | "{agent} restarts and retries." (fixed: an approved request always restarts the agent) |
| Extra line when the host matches `github.com` / `*.github.com` / `api.github.com` / `*.githubusercontent.com` | "Without your GitHub login: issues and pull requests still go through the orchestrator." |
| Menu item help (muted, second line) | Allow everywhere: "Every project's agents." · Allow for {role} only: "Only {role} agents of this project." |
| Keyboard | `a` allow for this project (same slot as other cards' primary), `r` reject, `y` reply; the caret menu opens with `⇧a`. `u` stays the card's existing undo of a just-sent decision; the decided line's **Undo** (§2.4) is a button only and takes no key |

The agent's reason is shown even for an invalid request.

### 2.2 Invalid value (`invalid` is set)

- Kind line stays `🛡 Sandbox`. Title and subject unchanged.
- The reason in a red-bordered block in place of the Allow button: "Can't be allowed: {reason}" (the core's sentence, e.g. "`~/.ssh` is a credential store: never allowed").
- Only **Reject** and **Reply** are shown. Line under the actions: "The agent is told no. Reply to suggest something narrower."
- (The core does not even create the card when the request is invalid at ask time; this state is only for a value that became invalid, e.g. a folder that now resolves to a forbidden place.)

### 2.3 States

- **Pending** (a button pressed, the core not answered): the buttons are disabled, the pressed one reads "Allowing…" / "Rejecting…", as the other cards do.
- **Error on Allow** (`sandbox_add` refused or failed): the card stays open, a red line above the actions: "Could not allow it: {reason}. Nothing was changed." The buttons come back.
- **Parked** (agent waiting at Stop): the `· parked` tag on the kind line; no other change.
- **Stale** (the agent went away): standard stale card, "Dismiss" only; line: "{agent} is no longer running. Nothing to retry."
- **Already allowed** is never a card (the tool answers the agent at once).

### 2.4 Decided state (the card collapses to a line, expands read-only like other cards)

| Decision | Marker / line text |
|---|---|
| Allow for this project | "Allowed for {project} by you · **Undo**" |
| Allow everywhere | "Allowed for all projects by you · **Undo**" |
| Allow for role only | "Allowed for {role} in {project} by you · **Undo**" |
| Reject | "Rejected by you" |
| Reply | "Replied by you: {message, clamped}" |

- **Undo** runs `sandbox_remove` on the entry the decision wrote. It is offered only when the decision created that entry: when the allowance already existed (`sandbox_add` returns the existing entry on a duplicate), the line reads "Already allowed for {project}" with no Undo, so Undo never removes an allowance the user set earlier. (Back: `decide` must report whether the entry is new.) On success the line becomes "Allowance removed · {agent} keeps it until it next starts" when `applies` is `next_start`, else "Allowance removed". It is not offered once the entry is gone (removed in Settings or by hand) — the line then reads "Allowed for {project} by you · removed".
- Undo does not message the agent. (It edits the allowance; it is not a new decision.)
- Undo failing: inline red "Could not remove it: {reason}." and the line keeps its Undo.
- Restart outcome follows the existing lifecycle marker ("{agent} restarted", failures as other cards).

### 2.5 Flow: blocked agent → card → allowed → agent retries

1. An agent's Bash hits the sandbox (host blocked, folder read-only, port bind refused, socket denied).
2. The agent calls `request_sandbox_access(kind, value, why)` and stops its turn (told "asked the user; stop your turn and wait").
3. One `sandbox` card appears in the Inbox / chat column. The Overwatch is not involved; no Overwatch message about it.
4. The user reads the title and reason and presses **Allow for this project** (or picks another scope from the caret).
5. The core writes the `[[sandbox]]` entry (note = the reason), audits it, recompiles settings, restarts the agent with `--resume` (or nothing, when settings reload live).
6. The card collapses to "Allowed for {project} by you · Undo", with the restart marker; the agent receives "approved … Retry." and retries.
7. Alternative branches: **Reject** → the agent is told "denied"; **Reply** → the agent gets the user's words with the denial; **invalid** → §2.2; **the same request again** → no second card (the agent is told "already asked").

## 3. Surface 2: Settings → Sandbox

Nav label **Sandbox**, between Permissions and MCP servers. Same section frame as `PermissionsSettings`: `h2` + sub line, then `h3` blocks on `.file`/`.row` surfaces, max width 720.

### 3.1 Layout

1. Header: **Sandbox** · sub: "What an agent's shell commands can reach: internet hosts, folders, local ports, sockets."
2. Explainer (§1) · links: **What can't be allowed** (expands §1 list, collapsed by default) · **Open Permissions settings**.
3. Scope blocks: **All projects** first, then one block per project, in the sidebar's project order. Each block is a `.file` surface with a title row (scope name, file path muted and mid-cut, **Open in editor** button as in Permissions), then the groups.
4. Per block, groups in the order: Internet hosts · Folders Bash can write · Local ports · Sockets. A group with no rows is not shown (the empty state is block-level, §3.3). A kind not in `sandbox_kinds` is not shown anywhere, not in the picker and not as a group header.
5. In **All projects**, built-in rows come first inside each group, muted, tagged **built in**, no Remove; the explanatory line above them: "Every agent gets these." Role hosts (`role:<id>`), when a project's team uses them, are shown in that project's block, muted, tagged "built in · {role}", no Remove.
6. Below each block: **Add** button → inline form (§3.2). A single applies line under the block after a change (§3.5).

Row anatomy: value (monospace) · role chip when `role` is set ("{role} only") · note (muted, one line, ellipsis, full on hover) · **Remove**. `local_ports` row: "Local ports (test servers)" with no monospace value.

### 3.2 Add form (inline)

Fields, in order. Labels and placeholders final:

| Field | Label | Placeholder / options | Help line (muted, under the field) |
|---|---|---|---|
| Kind | "What to allow" | Internet host · Folder Bash can write · Local ports · Socket (only the kinds `sandbox_kinds` returns) | — |
| Value (network) | "Host" | `api.github.com` | "A host, or `*.example.com` for all its subdomains." |
| Value (write) | "Folder" | `~/.cache/ms-playwright` | "An absolute folder, or one starting with `~`." |
| Value (unix_socket) | "Socket path" | `/tmp/my-test.sock` | "An absolute path to the socket." |
| Value (local_ports) | not shown | — | "Lets commands listen on local ports, for example test servers." (shown in place of the field) |
| Scope | "Applies to" | **All projects** · {each project} (the form opened inside a block defaults to that block) | — |
| Role | "Only for role" | "Any role" (default) · the roles of the chosen project's team (all of them when the scope is All projects); hidden for a solo project | "Leave on Any role unless only one role needs it." |
| Note | "Why" | "Optional, for whoever reads the file" | — |

- **Example chips** under the Kind picker, shown for the matching kind only; clicking a chip fills the value:
  - Internet host: `github.com`, `api.github.com`, `*.githubusercontent.com`
  - Folder: `~/.cache/ms-playwright`
  - Local ports: a single chip "Allow local ports" that selects the kind (no value).
  - Socket: none.
- Chip for GitHub hosts adds the muted line under the form: "Without your GitHub login: issues and pull requests still go through the orchestrator."
- **Buttons**: **Add** (primary) · **Cancel**. Add is disabled until the value (when the kind has one) is non-empty and the last `sandbox_check` was `ok`.
- **Live validation**: `sandbox_check` runs after the user stops typing (300 ms) and on scope change. While waiting: nothing shown (no spinner under 300 ms; after that "Checking…" muted). `ok` with a different `value` shows "Will be saved as `{value}`" muted. `ok` equal: a green check with no text. Not ok: the core's `reason` in red under the field, `aria-invalid` on the field.
- Local, immediate messages shown before calling the core (only for the empty case, the core's answer is final for everything else): empty host → "Enter a host."; empty folder → "Enter a folder."; empty socket → "Enter a socket path."
- On **Add**: the form closes, the new row is highlighted for 2 s, the applies line shows (§3.5). Duplicate (the core returns the existing entry): no new row, the existing row is highlighted and the muted line says "Already allowed here."
- Refused by the core after the check (race): the form stays open with a red line above the buttons: "Could not add it: {reason}."
- Opening the form in a block whose file has a parse error: the Add button is disabled with the title/tooltip "Fix the file first: line {n}." (writes would be refused).

### 3.3 States

| State | Copy / behaviour |
|---|---|
| Loading | "Loading…" (muted, `role=status`), as in Permissions |
| Load error | red: "Could not read the sandbox settings: {error}" + **Retry** |
| Empty scope (no entries, nothing built in shown for it) | "Nothing extra: agents get the built-in hosts only." (project blocks) · for **All projects** with defaults visible, the empty line under the defaults: "Nothing added for all projects." |
| No projects | "No projects yet. Add one to give it its own allowances." under All projects |
| Parse error on a file | red, in that block's title row: "Parse error at line {line}: {message}. Using the last good version." + **Open in editor**. The entries shown are the last good ones; Remove and Add are disabled with the tooltip "Fix the file first." |
| Warnings (skipped entries) | an amber list under the title row, same as Permissions, max 3 then "Show {n} more": "Allowance {id}: {message}. Skipped." Each warning has no Remove (the entry isn't parsed); the line ends with "Edit the file to fix it." once, below the list |
| No file yet | the block shows "no policy file yet" in the title row (like Permissions) and the empty line above; the file is created with the first allowance |
| Sandbox kinds missing | the section still works with the kinds returned; if `sandbox_kinds` is empty or fails: "This version of Claude Code can't widen the sandbox." and no Add buttons |

### 3.4 Remove

- **Remove** on a row removes at once (`sandbox_remove`), no confirmation (it only narrows, and the file keeps history in git/audit). The row is replaced by a muted line "Removed {value} · **Undo**" for 6 s; Undo calls `sandbox_add` with the same fields (role, note kept).
- Failure: red line in place of the row's actions: "Could not remove it: {reason}."
- The applies line (§3.5) follows a removal too.

### 3.5 Applies line

After any add/remove from this section, under the block (muted, `role=status`, kept until the next change or 10 s after the last one):
- `applies: "live"`: "Applies now."
- `applies: "next_start"` and `running` = 0: "Applies when an agent next starts."
- `applies: "next_start"` and `running` ≥ 1: "Applies when an agent next starts. {running} agent{s} running."
- Other open windows: on `sandbox_changed` the lists reload silently and the changed row is highlighted; no applies line (the actor knows).

### 3.6 Flows

**Add an allowance.** Settings → Sandbox → in the block → **Add** → pick what to allow (or a chip) → type the value, see "Will be saved as…"/the reason → pick scope/role/why if needed → **Add** → row appears highlighted → applies line.

**Remove an allowance.** Row → **Remove** → the row turns into the Undo line → (Undo within 6 s restores it) → applies line.

**Fix a skipped or broken file.** Warning or parse error → **Open in editor** → fix → return to the window (visibility change reloads) → the warning is gone.

## 4. Surface 3: Settings → Permissions, rules list, Add rule, Test a call

Keep the existing blocks (Policy files, When no rule matches, Hold) unchanged, in their order. Changes:
- The sub line stays "Which permissions are settled without you." and the explainer + the Sandbox hint (§1) go right under it.
- The last help line "Rules are added from cards and removed from their lines. The files are plain text." is replaced by "Rules are also added from cards and removed from chat lines. The files are plain text."
- New h3 **Rules** between Policy files and When no rule matches, then **Test a call** right after Rules, before "When no rule matches".

### 4.1 Rules list

One block per policy file (same rows as Policy files, in the same order: global, then projects), each with its rules, in file order.

Rule row: sentence (from `summary`) · effect chip · role chip ("{role} only") when set · note (muted) · **Remove**. The raw pattern (`match`) is on a second muted monospace line. Rule id muted at the end ("r12").

Effect chips use the existing status colours: Allow (green), The Overwatch decides (neutral), Ask me (amber), Deny (red).

| State | Copy |
|---|---|
| Loading | "Loading…" |
| Load error | "Could not read the rules: {error}" + **Retry** |
| No rules in a file | "No rules here. Anything not matched follows the defaults below." |
| Parse error | the file's existing error line; rules shown are the last good ones; Remove and Add rule disabled with "Fix the file first." |
| Warning (neutralised rule) | existing "Rule {id}: {message}" amber lines under the file row |
| Remove | no confirmation; row becomes "Removed {summary} · **Undo**" for 6 s (Undo = `add_rule` with the same fields); failure: "Could not remove it: {reason}." |

### 4.2 Add rule (inline form per file block, button **Add rule**)

| Field | Label | Options / placeholder | Help line |
|---|---|---|---|
| Tool | "When an agent wants to…" | run a shell command · read a file · edit a file · fetch a web page · use an MCP tool · do anything | — |
| Value | by tool: run a shell command → "Command starts with", placeholder `cargo test` · read a file / edit a file → "Path", placeholder `/Users/me/project/docs` · fetch a web page → "Domain", placeholder `docs.rs` · use an MCP tool → "Tool", placeholder `server__tool` · do anything → not shown | shell: "Matches commands that start with this." · path: "An absolute path. It covers the folder and everything in it." · domain: "Just the domain, without https://." · MCP: "The server and tool name, joined by two underscores." |
| Effect | "…then" | Allow · The Overwatch decides · Ask me · Deny | Allow: "Settled without asking." · Overwatch: "The Overwatch decides; you can undo it." · Ask me: "Makes a card." · Deny: "Refused. The agent is told." |
| Scope | "Applies to" | All projects · {each project} (defaults to the block's file) | — |
| Role | "Only for role" | Any role (default) · roles | — |
| Note | "Why" | "Optional" | — |

- **Live preview** under the value, updated 300 ms after typing via `rule_preview` (the UI never builds patterns): a monospace line with `pattern`, then the `summary` sentence under it. Label above: "This rule matches". While waiting: muted "Checking…". Error: the core's message in red; **Add rule** disabled.
- "Do anything" + Allow shows an extra amber line: "This allows every call." and requires ticking **I understand** before **Add rule** is enabled.
- Buttons: **Add rule** (primary), **Cancel**. After adding: the form closes, the new rule row highlights 2 s.
- Add failure: "Could not add the rule: {reason}." above the buttons, form stays open.
- A rule that the core would neutralise (e.g. Allow on a hard-listed call) is refused with the core's reason, inline.

### 4.3 Test a call

A block **Test a call**, always open, between Rules and "When no rule matches":

- Help: "Check what would happen, without running anything."
- Fields on one row: **Role** (Any role / roles / Solo) · **Tool** (same picker as Add rule, plus "do anything" absent) · **Input** (one line; label by tool as in the Add rule "Value" field, but the shell command is a full command: "Command", placeholder `cargo test --workspace`) · **Project** (optional; defaults to the first project when a role is chosen; "None" for global only).
- Button **Test**. The result appears under the row in a result block (`role=status`):
  - Matched a rule: "**{Effect}** · rule {id} ({file name}): {summary}" — e.g. "Allow · rule r12 (this project's file): Bash commands starting with `cargo test`".
  - No rule: "**{Effect}** · no rule matches; the default for {team roles|solo sessions} applies."
  - Decided by the core: "**Only you can allow this** · {core reason}."
  - Sandbox hint: for a Bash call to a host/path that is outside the effective sandbox, an extra muted line: "Passing this test doesn't mean the command can reach it. Check the sandbox: **Open Sandbox settings**." (shown only when the tool is a shell command and the input mentions a URL; otherwise not shown).
- Errors: "Could not test the call: {reason}." Empty input: the Test button is disabled; no inline error.
- Flow: Settings → Permissions → Test a call → choose role and tool → type the input → **Test** → read the effect and the deciding rule → optional jump to the rule's row (the id is a link that scrolls to it) or to Sandbox.

### 4.4 Flow: add a rule through the form

1. Settings → Permissions → **Rules** → the file block → **Add rule**.
2. Pick "run a shell command", type `cargo test`; the preview shows `Bash(cargo test:*)` and "Bash commands starting with `cargo test`".
3. Pick **Allow**, scope = this project, role = back, note "tests".
4. **Add rule** → the row appears highlighted; the file got the `[[rule]]` with the note.
5. (Optional) **Test a call** with role back, `cargo test --workspace` → "Allow · rule r12 …".

## 5. Cross-cutting

- **Refresh**: `sandbox_changed` reloads Sandbox lists; existing policy reload events and window visibility reload Rules (front reloads `policy_rules`). A change made elsewhere (card, file) highlights the affected row for 2 s.
- **Accessibility**: inline errors are `role=alert`; the applies/validation/test results are `role=status`; Remove and Undo buttons carry the value in their `aria-label` ("Remove api.github.com"); the caret menu on the card is a menu button with arrow-key navigation; example chips are buttons.
- **Audit/permissions**: no copy mentions the audit; it is silent.
- **Card and Settings are separate**: the card never links to Settings; it is self-contained.

## 6. Hand-off notes for ui

Frames to design (Figcraft, Drive `Orchestrator.figcraft`): the `sandbox` card (network, write, local_ports, socket; default, caret menu open, invalid, pending, error, decided with Undo, rejected, stale); Settings → Sandbox (loaded with two projects, empty project, parse error, warnings, Add form with validation error / ok / normalised, applies line, removed-with-Undo row, loading, load error, kinds missing); Settings → Permissions with Rules (rows with effect chips, empty, parse error), Add rule (with preview, do-anything confirmation), Test a call (matched, default, core-only, sandbox hint). Reuse the existing tokens and the permission card frame; the new kind icon is 🛡 only as a placeholder, `ui` picks the glyph.
