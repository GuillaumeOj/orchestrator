# Orchestrator — Product & Technical Specification

> Status: **Draft v0.12** · Owner: Guillaume · Last update: 2026-10-28
> v0.12 changes course after the first real team run ([brief](brief/2026-10-28-overwatch.md), [ADR-011](adr/011-overwatch-and-attention-model.md)). The user still had to open agent panes to keep work moving, mostly because of runtime defects (the app's own message file prompting a Read, holds that expired into pane-only prompts, agents suspended before their first turn, memory-cap thrash): fixed first (P4.0, FR-203/206/415/705/709). Then: one attention model computed in the core (FR-1301), **Mission control** as home in place of the Overview, the Inbox folded into the **Overwatch chat** as decision cards (FR-1303–1305), and a global, event-driven **Overwatch** agent that answers permission prompts within a policy, answers agents' questions from the docs, and unsticks agents (FR-1306–1310). Agent panes become an escape hatch. §9 is revised; P4 is added to §11.
> v0.11 applies what running a full team found (p3-ecosystem, 2026-10-27): the user had to type in the lead's pane twice. Team messages are read only when the turn that carried them ends, a resumed agent's queue entry no longer waits for its own slot, a turn that ends on a question for the user reaches the Inbox, the Lead has a slot of its own and the critical tier, and keeps its team moving (FR-411). New: plan first (FR-412), a review before every merge (FR-413), slash-command delivery, auto mode by default, desktop notifications, and the shell dock.
> v0.10 applies what building P2 (teams) found: three more orchestrator tools forced by the sandbox commit gap (`merge`, `sync_integration`, `request_spec_approval`), no Stop-hook "continue" (the next task is a normal turn), the duration-only turn ceiling for now, and the data model additions (scoped role and team ids, the team seat on agents, task and message fields).
> v0.9 applies the first day of daily use of the installed app (2026-10-26): the same Governor forecast after a restart as live (the reset as a reading of 0, a damped outside-the-app rate, the on-plan burn prior), and shadow logging of every admission action.
> v0.8 applies what building the P1 core services found (`crates/orch-store`, `orch-git`, `orch-mcp`, live run with Claude Code 2.1.302 on 2026-10-26): agents commit through the orchestrator MCP server, Inbox holds for permission prompts, memory-pressure cap levels, retention, restart replay of usage, and the data model additions.
> v0.7 applies what building the first desktop app found ([docs/app](app/README.md), Claude Code 2.1.302): permission prompts answered in the pane or the Inbox, resize debouncing, the measured app baseline, keyboard fixes, worktrees of a project that is itself a linked worktree.
> v0.6 applies what building the headless agent runtime found (`crates/agent-runtime`, live runs with Claude Code 2.1.302 on 2026-10-21): sandbox rules for worktrees, the commit gap, kill and hook details, usage sampling, recovery.
> v0.3 applies the findings of the [P0 CLI spike](spikes/p0-claude-cli.md) (Claude Code 2.1.302). v0.4 applies the [P0 terminal spike](spikes/p0-terminal.md) (Tauri 2.12, xterm.js 6). v0.5 applies the [P0 recovery spike](spikes/p0-recovery.md).
> v0.2 applies the decisions and fixes from [the 2026-10-17 review](reviews/2026-10-17-spec-review.md). Main change: **agents are real Claude Code CLI processes** on the user's Max plan; there is no Agent SDK.
> Scope: *what* we build and *why*, plus the high-level *how*. Implementation choices are in the ADRs (`docs/adr/`).

---

## 1. Vision

Orchestrator is a **personal, local-first macOS app** for running software projects with AI agents. It is built **around the official Claude Code CLI** and adds what a solo developer juggling several projects is missing:

- one place for all **projects** (local folders plus their git and remote state),
- **solo sessions or whole teams of specialist agents** (lead, backend, frontend, UI, UX…) working in parallel, each isolated in its own git worktree,
- each agent is a **real Claude Code session in a terminal pane**, so the chat is Claude Code's own, next to a **terminal that looks like kitty** and **your own nvim**,
- **full visibility on usage** (5h and weekly plan limits, context, tokens) and **pacing**, the key differentiator: the app spreads agent work so the Max plan lasts the whole working week.

### 1.1 Goals
| # | Goal | Success measure |
|---|---|---|
| G1 | Manage many projects and see their git/remote/PR state at a glance. | Git state of every project visible in ≤ 1 screen. |
| G2 | Run many agent sessions in parallel, safely isolated. | Agents never write outside their worktree; no lost work. |
| G3 | Compose teams of agents **visually on a canvas** and let a lead agent coordinate them on a feature. | A feature delivered end to end by a team with 2 gates. |
| G4 | Talk to any agent with the full Claude Code experience. | 100 % of Claude Code's interactive features are available, because it *is* Claude Code. |
| G5 | Keep the developer's own tools: terminal rendered like kitty, nvim with their config. | Side-by-side screenshot: font, colors and spacing match kitty. |
| G6 | Monitor usage live and **pace all work against the 5h and weekly limits**. | No hard limit lockout during working hours; ≥ 85 % of the weekly allowance used by reset. |
| G7 | Integrate with the developer's PM/design stack: GitHub, Linear, Notion, Figma, Penpot. | Tasks and design links flow without copy/paste. |
| G8 | Keep the runtime replaceable (other agent CLIs later). | Provider interface with one implementation (Claude Code CLI). Priority: **C**. |
| G9 | A designed, consistent UI with a Penpot file as its source of truth. | P1 screens designed before they are built. |
| G10 | **One interface**: the user drives all work from Mission control and the Overwatch chat, never from inside an agent's session. | A team run with **0 pane interventions** (FR-1311); every decision answerable from a card. |

### 1.2 Non-goals (for now)
- Multi-user collaboration, hosted service, accounts.
- **Rebuilding Claude Code's chat UI.** We embed the real CLI.
- **The Claude Agent SDK and API-key billing.** Max-plan usage only works through the official Claude Code client; API usage is billed separately (see ADR-002).
- A built-in code editor (the user's editor is used).
- Replacing GitHub/Linear: we sync with them.
- Windows/Linux in the first versions (macOS first, portable core).

---

## 2. Glossary

| Term | Definition |
|---|---|
| **Project** | A local directory registered in the app (usually a git repository). |
| **Feature** | A unit of product work in a project (e.g. "Checkout v2"). It owns docs, tasks, a team run and an integration branch. |
| **Agent** | One running **Claude Code CLI process** (`claude`) in a PTY, launched with a Role's configuration. |
| **Session** | The Claude Code conversation behind an Agent, identified by its Claude session id. It can be resumed after its process stops. |
| **Turn** | One unit of autonomous work the app hands to an agent (a task, a message, a "continue"). It is the unit the Governor admits. |
| **Role** | A reusable agent definition: prompt, model, effort, tools, MCP servers, permission profile, sandbox, budget priority. |
| **Team** | A graph of Roles (nodes) and relations (edges), authored on the canvas. |
| **Lead** | The team role that decomposes work, assigns tasks and integrates results. Exactly one per team. |
| **Task** | A unit of work assigned to a team node. Stored in the project's **task backend** (local / GitHub / Linear). |
| **Worktree** | A git worktree and branch dedicated to one agent, in the project's **sibling worktree folder**. |
| **Gate** | A decision only the user can make: spec approval, merge/PR, out-of-sandbox action, budget. |
| **Inbox** | The core's queue of **decisions**: everything that needs the user (or the Overwatch), ordered by the attention ladder (§9.6). From P4.3 it has no pane of its own: decisions are **cards** in the Overwatch chat. |
| **Decision** | A typed Inbox item: subject, context blocks, actions and a lifecycle from open to its effect (FR-1303). Rendered as a **card**. |
| **Attention** | The core's one record of what an agent or run is doing and what it **waits on** (user, agent, task, governor, limit, startup, external) and since when (FR-1301). Every surface renders it. |
| **Mission control** | The home view: every run and agent across projects, in lanes Needs you · Running · Waiting on · Queued · Done today (FR-1302). Replaces the Overview. |
| **Overwatch** | A global, event-driven Claude Code agent the user talks to. It settles permission prompts within the policy, answers agents' questions from the docs, and unsticks agents; it never starts work or approves gates (FR-1306, ADR-011). |
| **Policy** | The permission policy the core and the Overwatch apply: a hard-deny list, the user's rules (global and per project) and learned rules the user confirmed (FR-1308). |
| **Intake source** | Where a project's work comes from: GitHub, Linear, docs in the repo, or none (FR-1312). |
| **Pane intervention** | The user typing into an agent's pane while it waits or idles with an open task: the measure G10 drives to zero (FR-1311). |
| **Governor** | The scheduler that admits, queues, pauses or defers agent work to respect the weekly plan. |
| **Weekly plan** | The user's budget for the current plan week: working calendar, allocations, reserve. |
| **Signal** | A usage figure observed from Claude Code (statusline `rate_limits`, context, cost; OpenTelemetry metrics; hook events). |

---

## 3. Persona & key user journeys

**Persona: Guillaume.** A senior developer working alone on several projects in parallel. Claude Max subscriber, lives in kitty and nvim, keyboard first, **8 GB laptop that is often under memory pressure**. Uses GitHub, Linear, Notion and Penpot/Figma. He wants leverage (many agents) without losing control (gates, his own tools) or running out of weekly limit.

### J0 — Onboarding (first run)
1. The app checks that `claude` is installed, that its version is in the supported range (NFR-7), and that it is logged in with a Pro/Max subscription. If not: instructions, and the app stays in read-only mode.
2. **Workspace trust**: Claude Code blocks the first launch in an untrusted folder with a trust dialog, before any hook runs. Trust is keyed by the **main repository root**: once the user trusts the project root, every worktree of that repository is trusted wherever it lives (P0 recovery spike). So for each project the app opens one pane in the project root for the user to answer the dialog. After the repo is moved, the dialog comes back once; the app detects it on the PTY ("Quick safety check"), shows that pane to the user, and never answers it itself. The dialog's default choice is "No, exit" (the user moves down to "Yes, I trust this folder"). No hook fires before it is answered, so the first SessionStart clears the detection (its text stays in the PTY output after it closes).
3. Add the first projects (J1). Import `kitty.conf` (fonts, colors). Pick the editor command (default `nvim`).
4. Set the first weekly plan (J8). If last week's data is missing, the app proposes an even split.

### J1 — Add a project and see its state
1. **Add project** (⌘N) → pick a folder or clone a URL.
2. The app detects git, the default branch and the remotes, then runs a background `git fetch`.
3. The project row in the Overview shows: branch, ahead/behind, dirty count, open PRs and CI, live agents, budget share and burn.

### J2 — Solo session in a worktree
1. From the project, **New session** (⌘T) → choose a role (or "plain Claude"), the base branch, and a worktree (default on).
2. The app creates branch `orch/solo/<slug>` and a worktree in the sibling folder, then starts `claude` in a pane in that worktree with the role's configuration (§6.3).
3. The user talks to Claude Code as usual. Paths in its output are clickable and open in nvim (J7). A shell pane can be split next to it (J6).
4. When done: **Create PR** (gate) or **Merge into base**, then the worktree is cleaned up. Agents commit with the orchestrator MCP tool `commit` (the core commits for them, FR-702) and ask for a PR with `request_pr`: an Inbox gate; on approval the app pushes with the user's credentials and opens the PR with `gh`.

### J3 — Team on a feature
1. Open the **Team canvas** and add *Lead*, *Backend*, *Frontend*, *UI*, *UX* from the library, with the mouse or keyboard (§9.7). Connect edges: reports-to Lead, UX → UI hands-off, Frontend reviews UI. The **Rules panel** shows the result as text ("Frontend can message: Lead, UI").
2. **Launch on feature** → pick or create a Feature and write a 3-line brief.
3. The app creates the integration branch `orch/<feature>/integration` and **the Lead's worktree on it**. The Lead drafts `spec.md`, `ux-brief.md` and `api-contract.md` in `.orchestrator/features/<slug>/` on that branch → **Spec gate** in the Inbox.
4. The user reviews (in nvim or the rendered view) and approves. The Lead creates tasks in the task backend and assigns them to team nodes.
5. The Governor starts specialists as budget and the live-agent cap allow. Each one runs in its own worktree branched from the integration branch. UI/UX roles use the Penpot/Figma MCP.
6. Specialists report through the orchestrator MCP tools. The Lead reviews and merges their branches into the integration branch; conflicts go back to the owning agent.
7. **PR gate**: the user reviews the integration diff and approves. The app opens the PR (agents can't push themselves, §6.5).

### J4 — Mission control (was: Overview & monitoring)
1. The app opens on **Mission control** (FR-1302): lanes *Needs you · Running · Waiting on · Queued · Done today* across every project. A team run is one row with its phase stepper and its members; a solo session is a row of its own.
2. Every row says what it waits on and since when ("front-1 → back-1 · API contract · 12 min", "queued · live cap 2/2 · ≈14:20 · Run now ≈ +2 pp").
3. From the keyboard: open the session (the Claude Code pane opens over the board; Esc returns), interrupt, pause, suspend, kill, resume.

### J5 — Decisions in the Overwatch chat (was: Inbox triage)
1. A decision appears at once as a **card** in the Overwatch chat, next to the board, with the context to decide in one look: the tool call and its input, the rendered spec, the diff with line counts, the question's options, the cost (FR-1304).
2. The Overwatch settles what its policy allows (FR-1307, FR-1308): the card collapses into one line ("allowed back-1: `cargo add serde_json` · policy: installs") that expands, with undo where it still means something. On the others it adds its recommendation.
3. The user answers with the keyboard (`a` approve, `r` reject, `⏎` reply, `o` open) or in words to the Overwatch. The card follows the decision to its effect: sent, agent resumed (or queued, with its ETA), PR opened with its link, or failed with why.

### J12 — A day with the Overwatch
1. At the start of the working day the core starts the day's Overwatch session with a briefing (attention snapshot, open decisions, the week's plan). It posts one line: runs in progress, decisions waiting, week against plan.
2. During the day the Overwatch is woken by events (FR-1306): it allows routine prompts within the policy, answers a specialist's question from `api-contract.md` (citing it), restarts an agent whose first session never started, and leaves the user only the spec gate and one out-of-policy network call, each with its recommendation.
3. The user asks "why is checkout slow today?" and gets an answer built from the attention records and the Governor's log.
4. When the user gives the same answer to the same kind of call a third time, the Overwatch proposes a rule; one click adds it to the project's policy.
5. The user never opens a pane. To watch an agent, a click on its row opens its session over the board.

### J6 — Terminal
1. **Open terminal** (leader `t`) → a pane with the user's login shell in the focused agent's worktree or the project root.
2. It renders like the user's kitty (font, colors, cursor, padding from `kitty.conf`). Splits, tabs and zoom are available. **Open in kitty** opens the real kitty at the same cwd.

### J7 — Edit in nvim
1. Click (or select with the keyboard) a file path in an agent pane, a diff or an Inbox preview → **Open in editor**.
2. nvim opens at the file and line, reusing the project's nvim pane if one is open. The user edits and saves.
3. The app detects the change. If the file belongs to an agent's worktree, the agent is told "the user edited `src/x.ts`" at its next turn.

### J8 — Pacing the week
1. When the weekly window resets, a **Week planner** item appears in the Inbox. Working days and hours, projects A 50 % / B 30 % / C 20 % and a 10 % reserve are prefilled from last week. The user adjusts and confirms.
2. The title bar shows both windows: *Week 50 % · agents 41 % · plan 38 % (+3) · 5h 22 % · resets Mon 09:00*. "Week" is the total the plan reports; "agents" is the share the Governor controls (total − your interactive use − usage outside the app), and the plan and the error refer to it.
3. On Wednesday, the agents' usage is 9 pp over plan, so the Governor moves to **Red**. It caps live autonomous agents to 1, queues low-priority team tasks, and proposes in the Inbox to defer "Search v2" until after the reset. Every queued agent shows why and when it will start, with a **Run now** button that shows the cost ("≈ +2 pp").
4. On Friday, the agents' usage is 12 pp under plan with < 24 working hours left, so the Governor moves to **Catch-up** and releases queued work at higher concurrency.
5. The week ends at about 90–100 % used, near the reset. The weekly report summarizes usage per project, feature and role, plus the governor's actions.

### J9 — Plan limit reached anyway
1. Claude Code reports that a window is exhausted: a **StopFailure** hook with `error: "rate_limit"`, and the statusline jumps to `used_percentage: 100` with the reset time (a window the response doesn't report is *absent*, not 0).
2. The app marks all agents **Blocked by limit**, pauses autonomous work at the next turn boundary, and shows the exhausted window and its reset time in a global banner (one variant for the weekly window, one for the 5h window).
3. At the reset the Governor resumes queued work in priority order. Nothing is lost: sessions are resumed by id. The CLI also arms **its own auto-continue at the reset** ("continuing automatically at …"). The app either counts that turn as admitted work or sends Esc to cancel it and reschedules the agent through the Governor (default: cancel and reschedule, so the Governor keeps control). A real limit hit is still untested (R10).

### J10 — Recovery
1. **Login expired**: a global banner appears and autonomous work is paused. The fix is one click: open a pane running `claude /login`.
2. **App crash or restart**: the core owns the agents' PTYs, so a core crash **ends every agent** within about 10 s (SIGHUP): `claude` kills its shells, records an interrupted tool as `Exit code 137`, and exits. Recovery is therefore **reconcile, then `--resume`**, never reattaching to live processes; resuming within the prompt-cache lifetime (1 h) is nearly free. The reconciler runs in this order:
   1. Replay the `orch-hook` spool (events recorded while the core was down).
   2. Reconcile worktrees: finish `creating` rows (worktree creation = add + lock + DB row, each step idempotent), and report unknown or missing worktrees. A worktree whose folder exists but whose links are broken gets a targeted `git worktree repair` and is re-locked; a project whose checkout is gone raises one Inbox alert asking the user to locate it (the app then repairs and re-locks its worktrees).
   3. Detect half-done git operations (`MERGE_HEAD`, `REBASE_HEAD`, `rebase-*`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, unmerged paths): **flag and suspend** the agent and create an Inbox item.
   4. Remove a stale `index.lock` only when no git process runs in that worktree.
   5. Kill leftover processes by process group, argv and **cwd inside the worktree** (FR-207).
   6. Read each transcript by session id and decide: resume, suspend or flag. App-delivered turns carry a **turn id marker** that the reconciler finds in the transcript, so no turn is delivered twice. A resume is recorded as a queued resume turn, so it survives another crash before it happens; a suspended agent with queued work is resumed through the Governor (FR-403).
   7. Open gates come back as pending Inbox items; governor holds are requeued.

   Programs the user ran in shell panes are restarted in their cwd only; their state is lost (NFR-2). The resume message asks the agent to check what an interrupted command already did before redoing it.
3. **Laptop asleep** during off-hours work: nothing runs while the machine sleeps. When off-hours work is queued, the app asks to keep the machine awake on AC power (macOS power assertion) and shows a warning if it isn't on AC.

### J11 — Remove a project
1. **Remove project** → the app lists its worktrees, branches and running agents, and offers to stop agents, delete merged worktrees and keep unmerged ones (never deleted silently).
2. Settings and history are deleted or archived, as the user chooses. **Delete** removes the project, its rules, its worktree rows and its transcript index, and keeps events, usage and the audit log (the weekly report and FR-709 need them); **archive** hides the project and keeps everything. Its agents are stopped and archived. `.orchestrator/` in the repo is left untouched.

---

## 4. Functional requirements

Priority: **M** must, **S** should, **C** could. Phases are listed in §11. Journeys are in brackets.

### 4.1 Projects & Git
| ID | Requirement | P |
|---|---|---|
| FR-101 | Register a local folder as a project; clone from a URL. [J1] | M |
| FR-102 | Show the branch, ahead/behind upstream, dirty/staged/untracked counts, last commit and stash count. [J1] | M |
| FR-103 | Run `git fetch` periodically and on demand, only for projects active in the last 7 days. The interval is configurable (default 15 min); fetching pauses on battery below 20 % (on AC power it always runs) and when offline (no default route). "Active" = registered, an agent started, or a commit made for an agent in the last 7 days. [J1] | M |
| FR-104 | List branches and worktrees, including orphaned ones, with one-click cleanup. Unmerged work is never deleted without explicit confirmation. [J11] | M |
| FR-105 | Worktree lifecycle (ADR-003): created in the **sibling folder** `<parent>/<repo>.worktrees/` with `worktree.useRelativePaths=true` (so moving the repo and its sibling folder together needs no repair; needs git ≥ 2.48, checked at start), `git worktree lock`ed while in use, and cleaned up on completion. Creation is add + lock + DB row, each step idempotent. Branch names: `orch/solo/<slug>`, `orch/<feature>/integration`, `orch/<feature>/<role>-<n>`. The sibling folder is placed next to the **main checkout** (from `git rev-parse --git-common-dir`), even when the registered folder is itself a linked worktree: next to a worktree that lives inside the main checkout (such as `<repo>/.claude/worktrees/x`), it would land inside the checkout, where the sandbox's entry-by-entry read denies (FR-702) cover it and Claude's Write tool is denied in the agent's own worktree (seen 2026-10-26). [J2, J3] | M |
| FR-106 | Show the project's GitHub PRs and CI checks (via `gh`). [J1] | S |
| FR-107 | Diff viewer per worktree (against base or the integration branch) and file tree with change markers. | M |
| FR-108 | Merge/rebase helpers with conflict detection; conflicts can be handed to the owning agent. | S |
| FR-109 | Per-project settings: default base branch, task backend, default team, budget share, setup script run on worktree creation, whether `.orchestrator/` is tracked in the repo (default) or kept in app data. | M |
| FR-110 | **Setup script safety**: copying `.env` or other secrets into worktrees is opt-in, with a warning that agents will read them and send them to the model. | M |
| FR-111 | Remove a project with the guided cleanup of J11. | M |

### 4.2 Agent sessions (Claude Code in a pane)
| ID | Requirement | P |
|---|---|---|
| FR-201 | An agent is the **real `claude` CLI** running interactively in a PTY pane, with a per-agent configuration generated from its role (§6.3). The user interacts with it directly: Claude Code's full UI (slash commands, plan mode, `!`/`#`, vim mode, rewind, subagents…) is available as is. [J2] | M |
| FR-202 | Create, resume (`claude --resume <id>`), fork, rename and archive sessions. Sessions survive app restarts. **Delete** (archive) an agent from the UI: it is killed as in FR-207 first, its queued turns are dropped and it leaves the sidebar (its transcript, usage and audit stay); its worktree and branch go too when the user ticks it (unmerged work needs an explicit discard; a worktree another agent uses stays). **Delete feature** ends its runs (failed, "deleted by the user"), deletes every agent of the feature the same way, optionally removes their worktrees and the integration worktree, and archives the feature. | M |
| FR-203 | **Live agents vs queued work**: any number of sessions can exist, but the number of live `claude` processes is capped: **default 3** (about 190–255 MB each, measured in P0; 3 agents ≈ 0.6 GB). The cap also **drops when macOS reports memory pressure** and recovers when it ends: one less on *warn* (at least 1), 1 on *critical* (`kern.memorystatus_vm_pressure_level`). Pressure only ever suspends **idle** agents, least recently active first; working agents are never stopped. Extra work waits in the Governor's queue, and a suspended agent with queued work resumes only when a live slot is free. Agents the user starts or resumes by hand may exceed the cap. **A team's Lead has a slot of its own** (v0.11): it is not counted under the cap, never suspended for it, and its resume never waits for a slot, so a full team can't block the agent that unblocks it. A turn queued for a slot is released once the agent is started anyway (Resume, restart): it would otherwise wait for the slot the agent itself holds. **Hysteresis** (v0.12): the pressure level that sets the cap must hold for 60 s before the cap drops and for 120 s of *normal* before it rises (on 2026-10-27 the level flipped warn↔normal about 50 times, and each flip released one agent and suspended another). **An agent with a delivered turn that has not started** (pasted, no UserPromptSubmit yet) is not idle and is never suspended for pressure: two agents suspended 1 s after their task was pasted never got a transcript and could not be resumed. The Overwatch has a slot of its own, like a Lead (FR-1306). | M |
| FR-204 | **Suspend/resume**: an agent idle for N minutes (configurable, default 10; an agent with queued work, running background tasks or an attached pane is not idle) or waiting at a gate (P2) is stopped and later resumed with `--resume <id>` (verified in P0, including after a kill mid-tool). Resuming within the prompt-cache lifetime (1 h observed, read from the statusline `prompt_cache.expires_at`) costs almost nothing; later resumes re-pay the context and are counted by the Governor. The UI shows "Suspended" and resumes on focus or on new work. | M |
| FR-205 | Agent metadata pulled from Claude Code (§6.4): model, context %, cost/usage, current tool, last activity, state. It is shown in the pane header, the sidebar and the Overview. | M |
| FR-206 | **Programmatic input** for orchestration: the app can deliver a message or a "continue" to an idle agent (PTY bracketed paste), and add context to a busy one (hook `additionalContext`, §6.4). The user's own typing always wins over the app's. **Delivery rules** (from P0): start a message that begins with `/`, `!` or `#` with a space (otherwise the CLI runs it as a command and no turn starts); keep pastes under 900 characters (longer pastes are wrapped as untrusted `<pasted_content>`), and put longer instructions in a file under the agent's folder with a short message pointing to it; don't rely on tabs; wait 1.5 s after SessionStart before the first input. Input sent during a turn is queued by the CLI, not lost. An agent's queued turns are delivered together (one paste, each with its marker). **Slash commands** the app runs for the review (FR-413) are the one exception to the leading-`/` rule: typed as is, alone, from a fixed list, and recognised as the app's turn by their prompt at UserPromptSubmit. The user's own answers (Inbox replies, gate decisions) are delivered as user turns: past the Governor, like typing in the pane. **The agent can always read the files the app writes for it** (v0.12): the compiled permissions allow Read on the agent's `messages/` folder (on 2026-10-27, 8 of the run's 29 permission prompts were Reads of the app's own message files). A message is **delivered** when it is pasted, not when it is queued (`delivered_at`). | M |
| FR-207 | **Interrupt** (Esc sent to the PTY), **pause** (no new turns delivered), **kill**: SIGTERM to `claude`'s process group **and** to the group of every process with no controlling terminal whose cwd is inside the agent's worktree (`lsof -d cwd`), then SIGKILL after 5 s. A process with its own terminal and another group (the user's shell `cd`'d into the worktree) is not the agent's and is left alone. The process group alone isn't enough: Claude's Bash tool runs each command in its own session, and `cmd &` children outlive `claude` (P0). The agent id in the environment can't be used to find them (macOS hides the environment of system binaries). PTY master file descriptors are close-on-exec, so a leaked one can't keep a headless `claude` alive. The kill is **checked**: the core scans again (up to 2 s after SIGKILL, zombies ignored) and, if a process of the agent survived, the kill fails with its pid and command, the agent keeps its pid and group so a retry finds it, and nothing is deleted. | M |
| FR-208 | Clickable file paths and URLs in agent and shell panes (open in editor or browser). Paths that the terminal wrapped onto several rows are joined before matching (Claude Code wraps long paths). | M |
| FR-209 | Search across session transcripts (Claude Code's JSONL files), per project or global. Transcripts are found **by session id** across `~/.claude/projects/*/`: after a repo move, a resumed session keeps writing to the transcript in the old project folder. The index (SQLite FTS5) covers the sessions of the app's agents and the transcripts in each registered project's folders (its checkout and its worktree root); it is updated incrementally after each turn. | S |
| FR-210 | macOS notifications follow the attention ladder (§9.6), never one per finished turn. The core watches the Inbox every 3 s, also while the window is hidden; new level 1–2 items notify with a sound, several at once batched; a question asked as `fyi` never notifies. Desktop and sound are settings (palette). | M |

### 4.3 Agent & team builder (visual canvas)
| ID | Requirement | P |
|---|---|---|
| FR-301 | **The canvas is the main editor** for teams: add, move and connect nodes, zoom and pan, auto-layout, undo/redo, minimap. Sized for 3–10 nodes. [J3] | M |
| FR-302 | **Full keyboard operation of the canvas**: add a node from the library (`n`), move the selection between nodes (arrows/hjkl), connect (`c` then the target and the edge type), delete, open the inspector (`⏎`). | M |
| FR-303 | Node = a Role configuration in the inspector: name, icon/color, prompt (markdown, can be opened in nvim), model, effort, allowed tools, MCP servers, permission profile, sandbox profile, max concurrency, budget priority. | M |
| FR-304 | Edge types and their meaning: **reports-to** (A → B: A sends reports to B, B can assign tasks to A), **hands-off-to** (A → B: A can send artifacts to B), **reviews** (A → B: A can request reviews from B and B can reply), **can-message** (both ways). The Lead can always message every node. | M |
| FR-305 | **Rules panel**: a text view of the resulting permissions ("Frontend can message Lead, UI; can request review from Reviewer"). Validation errors are shown inline (no lead, unreachable node, cycle of reports-to). | M |
| FR-306 | Library of reusable Roles and Team presets (global or per project): duplicate and version them. Starter presets: *Solo dev*, *Full-stack feature team*, *Bugfix squad*, *Design sprint*. | M |
| FR-307 | Import/export teams and roles as JSON. | S |
| FR-308 | At launch, a team is **compiled** into per-agent Claude Code configurations (§6.3). The compiled files can be inspected read-only, and a TeamRun keeps a snapshot of the team version it used. | M |

### 4.4 Orchestration runtime
| ID | Requirement | P |
|---|---|---|
| FR-401 | Lead protocol: *understand → spec → (spec gate) → decompose → assign → track → review → integrate → (PR gate)*. It is written into the Lead's prompt and enforced by tool availability. The Lead is woken by every report, message and answer as a turn: it never polls, and it ends its turn when it waits. It settles what it can and sends the user only real decisions (product, scope, security, money) through `ask_user`. [J3] | M |
| FR-402 | **Orchestrator MCP server** (stdio, one per agent, ADR-002) with the tools `send_message(to, body)`, `inbox()`, `create_task`, `update_task(status, notes)`, `list_tasks`, `handoff(artifact, to)`, `request_review(branch, to)`, `ask_user(question, options)`, `report_done(summary)`. Because sandboxed agents can't write git from a worktree (FR-702), three more tools let the core do the git work outside the sandbox, as `commit` does: the Lead's `request_spec_approval(summary)` (the spec gate, FR-401), the Lead's `merge(branch, task?)` (FR-405) and a specialist's `sync_integration()` (merges the integration branch into its own; a conflict is left in progress for the agent, whose `commit` concludes it). Each role sees only its own tools (the compiled role lists them for orch-mcp in `ORCH_TOOLS`); the core checks again, and checks recipients against the team graph. v0.11 adds the Lead's `team_status`, `nudge(agent, text)`, `resume_agent(agent)` (FR-411) and `decide_plan(task, approve, note)` (FR-412), and a specialist's `submit_plan(plan)` and `submit_review(task, findings)` (FR-413); `ask_user` takes `kind: decision | fyi`. v0.12 adds the Overwatch's own tool set (FR-1307), which no team role sees. | M |
| FR-403 | **Message delivery without polling**: for a busy agent, messages are injected at its next tool call (PostToolUse `additionalContext`: "2 new messages from Lead"). For an idle agent, a new turn is delivered via the Governor. A suspended agent is resumed through the Governor first. A message is **delivered** when queued or injected and **read only when the turn that carried it ends**; a turn that never runs (cancelled, dropped) gives its messages back, and `inbox()` lists the delivered-but-unread ones. A Stop after messages arrived mid-turn is blocked **once** (`decision: block`), listing them, so a report that lands during the user's turn is acted on. | M |
| FR-404 | Task dependencies (blocked-by): an agent is only given a task once its dependencies are done. | M |
| FR-405 | The Lead owns the integration worktree and does all merges into it, through `merge` (FR-402: the core merges outside the sandbox, hooks off, with the Lead's identity, and audits it); specialists never merge. On a conflict the merge is aborted, the task goes back to its owner, and the owner is told to `sync_integration`, fix and report again. | M |
| FR-406 | Team controls: pause, resume or stop the whole team; restart one agent; reassign a task; add a node mid-run. | M |
| FR-407 | Watchdog: a turn exceeding its ceiling (usage or duration, set per role) is interrupted (Esc), and an Inbox item is created. An agent with no progress for 15 min creates an Inbox item. Once per turn each. **P2 enforces the duration ceiling only**; the usage ceiling (weekly percentage points per turn) needs per-turn usage attribution and comes later. | M |
| FR-408 | Team activity timeline: messages, task changes, merges, gates, governor actions; filterable. | M |
| FR-409 | The user can message the Lead or any agent directly (in its pane, or through the team timeline). | M |
| FR-411 | **The Lead keeps the team moving** (the user talks to the Lead only): the core tells it, once per episode, when a member waits on the user's permission answer for 3 min, sits queued 10 min, idles 10 min with an open task, or fails; `team_status`, `nudge` and `resume_agent` let it act. A Lead runs at the critical tier (past the pacing caps; Pause all and plan limits still hold it) with its own slot (FR-203). Permission prompts are answered by the policy, the Overwatch or the user (FR-1308, ADR-011), never by the Lead. The same signals also reach the Overwatch, which can act across teams (FR-1309). | M |
| FR-412 | **Plan first**, per task (`plan_first` on `create_task`/`update_task`): the owner reads, sends a plan with `submit_plan` and ends its turn; its edits and commits are refused until the Lead approves with `decide_plan` (or sends it back). The Lead asks the user only when a plan changes scope or a recorded decision. App-enforced rather than Claude's plan mode, whose exit waits on a permission prompt. | S |
| FR-413 | **A review before every merge**, chosen per team (ADR-010): **self** (default) — on the first `report_done` the core types `/code-review high --fix --max-findings all`, then `/simplify`, each as its own turn, then asks for the report again with what was fixed and each finding left with why; **node** — a review task for the node a `reviews` edge points to (or a reviewer role), which calls `submit_review`; **none** — the Lead reviews. The Lead's `merge` of a task needs its review. The report lists the findings left; the Lead accepts each (with a reason), sends it back (`nudge`), or asks the user for a product, scope or security call (one question per task). | M |
| FR-414 | **Questions always reach the Inbox**: Claude Code's `AskUserQuestion` in a team agent's autonomous turn becomes an Inbox question (with its options) and the dialog is refused; a turn that ends on a plain-text question is sent back once to use `ask_user`, then (or in a turn the user started in the pane) forwarded as is; typing in the pane closes it. Answers come back with their question. | M |
| FR-415 | **Starts that never happen are recovered** (v0.12). A spawned agent with no SessionStart within 30 s becomes a decision with its PTY tail ("startup blocked", `waiting_on: startup`): a dialog other than the trust dialog is waiting. A resume that exits within 5 s with no transcript for its session id is restarted **once** as a fresh session (new session id) in the same worktree, with its task brief and its queued turns delivered again; a second failure becomes a decision. Seen 2026-10-27: lead-1 and front-1 exited 1 on every Resume and had to be deleted. | M |
| FR-410 | **Subagents and background work**: Claude Code's Agent tool runs subagents **asynchronously by default**, and background shells too; when they finish they start a **new turn by themselves** (a `<task-notification>` prompt). These turns don't come from the app, so the Governor counts them (they still pass PreToolUse admission), and a role can disallow background Bash and async subagents (exact switch to confirm, R10). Subagent usage is attributed to the agent through SubagentStart/SubagentStop (`agent_id`, `agent_type`, transcript path) and the notification's `subagent_tokens`. | M |

### 4.5 Tasks & project management
| ID | Requirement | P |
|---|---|---|
| FR-501 | Task backend abstraction (ADR-004): **Local** (app DB), **GitHub Issues**, **Linear**. Chosen per project. | M (Local, GitHub) / S (Linear) |
| FR-502 | Board view (Backlog / Ready / In progress / Review / Done), the same over every backend. | M |
| FR-503 | Two-way sync with GitHub/Linear (status, assignee label, comments). On conflict the remote wins and the user is notified. | S |
| FR-504 | Feature docs in `.orchestrator/features/<slug>/` (`spec.md`, `ux-brief.md`, `api-contract.md`, `decisions.md`). **The source of truth is the copy on the integration branch**, edited in the Lead's worktree; the Feature page always opens that copy. | M |
| FR-505 | Notion: publish a feature's docs to a page and refresh it; bring comments back as Inbox items. | C |
| FR-506 | Links between tasks, branches, PRs and sessions, so every artifact can be reached from the others. | M |

### 4.6 Design tooling
| ID | Requirement | P |
|---|---|---|
| FR-601 | UI/UX roles can be given the **Penpot MCP** and/or **Figma MCP** in the builder. | M |
| FR-602 | Design artifacts (file/frame links, exported images) are attached to the feature page and handed off to frontend roles. | S |
| FR-603 | Preview of exported frames on the feature page. | C |
| FR-604 | MCP auth status per server; missing auth becomes an Inbox item, never a silent failure. | M |

### 4.7 Gates, permissions, sandbox & Inbox
| ID | Requirement | P |
|---|---|---|
| FR-701 | Default autonomy is **"gates at key points"**: inside its worktree an agent edits files and runs commands freely, within its sandbox. | M |
| FR-702 | **Enforced sandbox** (ADR-009): Claude Code's OS-level sandbox for Bash, with writes limited to the agent's worktree (plus the CLI's scratch folder and caches), **reads denied** for credential stores, the main checkout and the other worktrees, and network limited to an allow-list. Claude's Read/Edit/Write tools are not covered by the OS sandbox, so permission rules (absolute paths written `//…`) deny the same places. The main checkout is denied **entry by entry, except its `.git`**: the CLI drops an `allowWrite` that lies under a `denyRead` path, and the worktree's git needs the repository's git dir (`allowRead` and `allowWrite` on it). Sandbox paths are plain absolute paths; only permission rules use `//`. **Known gap (2026-10-21, 2.1.302, still true 2026-10-26)**: sandboxed Bash still cannot write the repository's git dir from a linked worktree (`index.lock` denied), so agents can't `git commit`. **Agents commit through the orchestrator MCP tool `commit(message, paths?)`**: the core commits outside the sandbox, in that agent's worktree only (never a main checkout), with the agent's generated git identity, literal paths that must stay inside the worktree, and **git hooks off** (a hook is a file the agent can edit and would run outside the sandbox). The tool is allowed in the generated settings; the core applies the permission rules (deny refuses, ask waits for the Inbox) and logs every commit (FR-709). It never pushes. | M |
| FR-703 | **No credentials for agents**: credential env vars and `SSH_AUTH_SOCK` are removed; credential files on disk (`~/.ssh`, `~/.config/gh`, `~/.netrc`, `~/.git-credentials`, cloud CLIs…) are unreadable inside the sandbox; git uses a generated global config with no credential helper (`GIT_CONFIG_GLOBAL`, `GIT_TERMINAL_PROMPT=0`). Removing env vars alone is not enough: in P0 the SSH key and the `gh` keyring token still reached GitHub. `git push`, PR creation and merges to protected branches are only possible through gated orchestrator MCP tools (`request_pr` in P1: an Inbox gate; on approval the app pushes and opens the PR). | M |
| FR-704 | Gate types: **Spec approval**, **Merge/PR**, **Out-of-sandbox action**, **Budget** (governor "ask"). Gates are implemented with **PreToolUse hooks** (they also run for allow-listed tools) and the orchestrator MCP tools. | M |
| FR-705 | **Gates and permission prompts**: a short decision (a permission prompt, a quick gate) is answered from the Inbox through the **PermissionRequest hook** without ending the turn, while `orch-hook` holds it within its own deadline. Long holds work (200 s under a 600 s timeout in P0, with the pane still usable), so ending the turn for a long wait is a **memory** choice: the hook denies with "awaiting user approval", the turn ends, and the agent can be suspended; on approval the app resumes it and delivers "approved: proceed". While it holds a hook, the core **watches the connection**: Esc in the pane kills `orch-hook` and no hook event reports it, so on EOF the Inbox item is closed as "interrupted by the user". **Claude Code shows its own permission dialog in the pane while the hook is held** (2.1.302, seen 2026-10-26), so the pane and the Inbox can both answer: when the user answers in the pane, the next hook for that call (PostToolUse, PostToolUseFailure, PermissionDenied) or the end of the turn closes the Inbox item with no decision. The Inbox also offers "answer in the pane" (no decision), needed for the dialog's "don't ask again" choice. The store's permission rules answer first (deny over ask over allow); otherwise the hook is held for an Inbox decision for up to **300 s** by default (configurable, always inside `orch-hook`'s deadline). If nobody answers in time, **the agent is parked** (v0.12, ADR-011): the hook denies with "ORCH: awaiting the user's approval", the turn ends, the agent may be suspended and its slot is freed. The decision stays open with the call and its input (never pane-only: on 2026-10-27, five prompts outlived the hold and could then only be answered in the pane, ~121 min). On Allow the core adds a one-shot allow for that exact call and resumes the agent with "approved: retry the call"; on Deny it delivers the reason. Before the user, the policy and then the Overwatch get their turn (FR-1308). "Always allow" creates a scoped rule (FR-708); it is not offered for a call an *ask* rule sent to the Inbox, since an allow rule never wins over it. | M |
| FR-706 | Permission profiles (strict / gated / autonomous) per role, overridable per session. **Autonomous (`--permission-mode auto`) is the default** (v0.11): Claude Code's classifier lets routine commands through and stops only risky ones, so team agents don't stall on prompts; the worktree sandbox (FR-702) and the core-only push, merge and PR gates hold in every profile. | M |
| FR-707 | Inbox: one list ordered by the attention ladder (§9.6), with filters (project, type, agent) and keyboard triage (`j/k` move, `a` approve, `r` reject, `⏎` reply, `o` open, `x` select for batch, `u` undo for 5 s). Items whose context has changed are marked **stale**. From P4.3 the same keys act on cards in the Overwatch chat, and the Inbox pane is removed (FR-1304). | M |
| FR-708 | "Always allow this for this project/role" creates a scoped rule that is visible and revocable in settings. | S |
| FR-709 | Every gate decision, permission answer and permission change is logged, with **who decided**: `user` (card), `pane` (answered in Claude Code's own dialog), `rule:<id>`, `overwatch` (with its reason and sources), or `expired` (v0.12: answers through the app's permission broker were not audited, so it was unknown where a prompt had been answered). | M |

### 4.8 Usage pacing & budgets (core)
The control law is specified in **ADR-007**. The requirements below are what the user sees.

| ID | Requirement | P |
|---|---|---|
| FR-801 | **Usage collector**: record the plan-limit signals from Claude Code (statusline `rate_limits.five_hour` / `seven_day` used % and reset times, StopFailure `rate_limit` errors) and per-session usage (OpenTelemetry token/cost metrics, statusline cost and context), attributed to agent, role, team, feature and project. Each sample records where its value came from (*observed* or *estimated*). Statusline `rate_limits` are **per-process copies of that agent's last API response**, so for each window the collector uses the **most recent sample across all agents** (by sample time), and treats a missing window as unknown, not 0. A statusline re-render carries no response time: a sample counts as a **new response** only when its readings or its cost changed, and is timed when the core receives it (idle re-renders are dropped). The first response's sample can arrive up to `refreshInterval` (5 s) after its Stop. From OpenTelemetry the collector uses the **`claude_code.api_request` events** (tokens, cache, cost, model per request, attributed by `orch.agent_id`); metrics and traces are accepted and dropped. Each sample stores the Governor's cost class (interactive / critical / autonomous) and allocation, so the week's accounting is replayed after a restart. | M |
| FR-802 | **Weekly plan**, aligned to the weekly reset (`rate_limits.seven_day.resets_at`; the plan week moves when a reading shows another reset): working calendar (days, hours, time zone, holidays), allocations per project/feature/team as **weights**, a priority tier (critical / high / normal / low), and a **reserve** for interactive work. Prefilled from last week. [J8] | M |
| FR-803 | **Ideal curve**: `ideal(t)` = fraction of the week's weighted working hours elapsed at time t × (100 % − reserve − forecast unattributed usage). Off-hours count only if the user enables an off-hours allowance. | M |
| FR-804 | **Pressure states** with hysteresis: **Green / Amber / Red / Catch-up**, computed from the error on the **governed share** `e = (used − reserve used − unattributed used) − ideal` (pp), so your own interactive use and usage outside the app never push agents into Amber/Red, and from the **total** used projected at the reset, so a lock-out is still prevented. Thresholds are in ADR-007 and adjustable in settings. | M |
| FR-805 | **Governor v1 actions**: cap live autonomous agents per state; queue new turns by priority; pause teams at the next turn boundary; propose to defer work until after the reset; a separate **5h guard** that keeps headroom for interactive work. | M |
| FR-806 | **Admission points**: the Governor decides at **turn start** (for turns the app delivers; self-started turns from background work are counted, FR-410) and at **each tool call** (a PreToolUse hook waits up to a few seconds for admission, then denies with "paused by governor" if it is still held). Parallel tool calls run their hooks concurrently, and each one is admitted separately. The user's own typing in a pane is never blocked. **`orch-hook` fails closed for tools**: hook timeouts, crashes and a missing hook binary all let the tool run, so `orch-hook` always answers itself. If the core's socket is missing, refused, closes mid-hold, or misses `orch-hook`'s deadline, PreToolUse and PermissionRequest answer **deny** ("ORCH: orchestrator unavailable"); every other event exits 0 at once and is **spooled** to the agent's folder for replay. It never exits with code 2 on non-tool events (that blocks and erases the user's prompt), and only PreToolUse/PermissionRequest wait on the core; the others wait at most 1 s for the connection **and** the reply together, so a hung core never delays a prompt by more than a second. An admitted tool call gets **no decision** from the hook (not `allow`), so the agent's own permission prompts still apply. | M |
| FR-807 | **Interactive and critical work** is admitted first and draws from the reserve. When the reserve is used up, the app warns instead of blocking and squeezes the other work harder. If most usage is interactive, the Governor has little to act on; the planner says so. | M |
| FR-808 | **Fair scheduling**: queued work is picked by weighted fair queuing over allocation weights, with priority tiers and **aging** (one tier up for every 2 h of waiting) so nothing starves. Unused weight is redistributed. Nothing rolls over to the next week. | M |
| FR-809 | **Transparency**: every queued or paused agent shows why, its estimated start, and **Run now** (or **Start now** if it never ran) with its estimated cost **in the window that binds it**: % of the 5h window when the 5h guard holds it, pp of the week otherwise; the confirmation shows both. Every governor action is logged with its reason and can be undone from the log. A **next-action preview** shows what the Governor would do next. | M |
| FR-810 | **"Ask" actions** come with a default and a timeout (default: apply after 20 min), so they never stall when the user is away. | M |
| FR-811 | **Overrides**: Sprint mode (borrow budget: today's target goes up and the remaining days go down, and the trade-off is shown), hard caps per allocation, **Pause all** / **Resume**. | M |
| FR-812 | **Shadow mode**: the Governor can log its decisions without enforcing them (default on for the first week). Every action that would change admission is logged as *shadow*, including the reserve warning. | M |
| FR-813 | **Unattributed usage** (the user's Claude usage outside the app: claude.ai, mobile, other terminals) = observed weekly % − the app's own attributed share. It is forecast as its own rate, subtracted from the agents' budget, and shown as "outside the app". The rate used for the forecast is shrunk toward a prior by the hours of the week observed so far, so one heavy day early in the week is not extrapolated over the whole week. A restart that replays the week's samples gives the same forecast as the live app. | M |
| FR-814 | **Estimator fallback**: if the observed % is not available (e.g. before the first response of the week), the weekly % is estimated from usage with one calibrated scalar and a safety margin (10–15 pp), marked with a confidence indicator. The weekly reset counts as a reading of 0: the full margin applies only before the week's first reading when the app did not see the reset, and is otherwise capped by what is extrapolated. | S |
| FR-815 | **Per-model weekly limits**: if the plan has a separate weekly limit for some models, it is tracked as an extra window, and those models are avoided automatically when that window is under pressure. | S |
| FR-816 | Off-hours scheduling: queue long work for configured windows, inside the plan's limits; requires AC power and a power assertion (J10). Allowed by the R1 decision. | S |
| FR-817 | Alerts: 50 / 75 / 90 % of the week, entering Red, 5h guard active, limit reached (J9). | M |
| FR-818 | Weekly report: usage per project/feature/role/model, governor actions, plan versus actual. | S |
| FR-819 | **Model downgrade** is manual or opt-in per role, applied only at a task boundary, never to the Lead or reviewers by default. (Switching models breaks the prompt cache and can cost quality.) | C |
| FR-820 | **Pacing simulator**: replay a recorded or synthetic week against the policy, for testing and tuning (acceptance tests in §5). | M |

### 4.9 Overview & monitoring
| ID | Requirement | P |
|---|---|---|
| FR-901 | Overview pane: every agent with live state (§9.5), current tool, files touched, elapsed time, context %, usage, governor status, process memory; grouped by project or by state. [J4] **Replaced by Mission control in P4.2** (FR-1302), which keeps these columns in a row's details. | M |
| FR-902 | Usage charts by project, feature, team, role, model and day. | S |
| FR-903 | Actions from the Overview: open, interrupt, pause, suspend, resume, kill. | M |
| FR-904 | The app's own memory (Tauri + webview processes + each `claude` process), with a warning threshold. | M |

### 4.10 Terminal
| ID | Requirement | P |
|---|---|---|
| FR-1001 | Terminal panes on a real PTY running the user's **login shell** with their env and dotfiles. [J6] | M |
| FR-1002 | **Renders like kitty**: font family/size, line height, colors, cursor style, padding and opacity imported from `kitty.conf` (following `include`s; unmapped options are listed), truecolor, Nerd Font symbols, ligatures only if kitty has them enabled. How (P0, ADR-006): (a) resolve `font_family` **the way kitty does** (exact monospace family, otherwise Menlo) and warn when kitty itself can't resolve the configured font; (b) the core loads the font files for the UI, because WKWebView hides user-installed fonts; (c) kitty's bundled `SymbolsNerdFontMono` is the symbol fallback; (d) cell width rounds up and line height comes from CoreText metrics, like kitty. **Acceptance**: side-by-side screenshots against what kitty *actually* renders (`kitty --debug-font-fallback`), not what `kitty.conf` says. | M |
| FR-1003 | Tabs, splits and zoom; **shells open as tabs of one dock under the panes** (⌘J shows or hides it; ⌘; ] / [ cycle its tabs; hidden shells keep running), splits still make shell panes; the cwd defaults to the focused agent's worktree or the project root; scrollback is capped in memory, with older output spilled to disk. **Resizes are debounced** (a window drag fires dozens) and a size that flips back and forth (A → B → A within a second) is not applied again: Claude Code reprints its whole screen on every resize, and one such feedback loop ran the webview up to 2.8 GB (2026-10-26). | M |
| FR-1004 | **Open in kitty** at the same cwd, using `kitty @ launch --cwd` when `kitty.conf` enables remote control (`allow_remote_control`, `listen_on`), otherwise a new kitty instance. | S |
| FR-1005 | **Renderer policy** (P0 memory, NFR-1b): the **DOM renderer is the default**. The first WebGL pane costs about 110 MB in WebKit's GPU helper, against about 10–15 MB per pane for the DOM renderer. WebGL is opt-in (setting), and even then at most 12 visible panes use it. Hidden panes always drop their WebGL context and keep only their buffer. WebKit allows 16 live WebGL contexts per page, and a lost context falls back to the DOM renderer. P1 confirms that the DOM renderer holds NFR-1 under heavy output; if it doesn't, the focused pane may switch to WebGL. | M |

### 4.11 Editor integration
| ID | Requirement | P |
|---|---|---|
| FR-1101 | **Open in editor** on file paths in panes, diffs, Inbox previews and task artifacts. [J7] | M |
| FR-1102 | The editor command template defaults to `nvim +{line} {file}` (or the `editor` set in `kitty.conf`), run in an embedded pane. If the project already has an nvim pane, the file opens there (`nvim --server <sock> --remote`). An external kitty window is optional. | M |
| FR-1103 | File changes are detected in worktrees. User edits appear in the timeline and are announced to the owning agent at its next turn. | S |
| FR-1104 | No built-in code editor; the only text editing in the app is for short fields (brief, replies). | M |

### 4.12 Settings
| ID | Requirement | P |
|---|---|---|
| FR-1201 | Claude Code: path to `claude`, supported version range (checked at every spawn), login status. Agents run with `DISABLE_AUTOUPDATER=1` (in P0 the CLI replaced its own binary mid-session); Claude Code updates are applied by the user, and the contract tests (R2) run after each one. The helper binaries (`orch-hook`, `orch-status`, `orch-mcp`) are installed at a **stable path in app data** (not inside the `.app` bundle, which moves and updates), and checked to exist and be executable at every spawn: a missing hook command fails open. The app never sets `ANTHROPIC_API_KEY`, and warns if it finds one in the environment (it would bill the API instead of the plan). | M |
| FR-1202 | MCP server registry (reused across roles) with per-project overrides. | M |
| FR-1203 | Integrations: GitHub (`gh` auth), Linear, Notion, Penpot, Figma, each with status and reconnect. | M (GitHub) / S (others) |
| FR-1204 | **Keyboard contract** (§9.7) with configurable bindings and a conflict checker that also reads the macOS system shortcuts (`com.apple.symbolichotkeys`, e.g. input-source switching). | M |
| FR-1205 | Theme: see §9.1 for precedence. | S |
| FR-1206 | The app never writes to the user's Claude Code configuration (`~/.claude`, `~/.claude.json`). All agents share it, because a separate config dir (`CLAUDE_CONFIG_DIR`) loses the login; Claude Code itself writes transcripts, history and trust there, and the app only reads them. Each agent gets an extra per-agent settings layer (§6.3). | M |

### 4.13 Overwatch, attention & Mission control (v0.12, ADR-011)
| ID | Requirement | P |
|---|---|---|
| FR-1301 | **One attention model**: the core computes, for every agent and team run, an attention record (`level`, typed `waiting_on {kind, ref, since}`, `reason`, `activity`) and **pushes it on change** over one channel; the UI renders it and derives no state of its own (the per-pane polls of 1–15 s go). `needs_you` means exactly `waiting_on.kind = user`, so every count of "needs you" is the same number. `since` is when the current wait began, not when the agent was created. A run's record has its phase, its members' records and its first external blocker. | M |
| FR-1302 | **Mission control** is the home view (replaces the Overview, FR-901): lanes *Needs you · Running · Waiting on · Queued · Done today* across every project, with a **per-project filter from the first version** (one, several or all projects; remembered). A team run is one row (feature, phase stepper draft → spec → planning → executing → integrating → PR, its members as chips with their own `waiting_on`); a solo session is a row. *Queued* rows show the reason, the ETA and **Run now** with its cost (FR-809); the lane lists the **whole** queue. *Done today* lists outcomes (task done, merge done, PR opened with its link, run finished). Pane-local keys as in §9.7. [J4] | M |
| FR-1303 | **Decisions keep their structure**: an Inbox item is a Decision with a typed subject (agent, role, project, feature, run, task), typed **context blocks** (tool call with its full input and cwd; rendered markdown; diff summary with per-file line counts and the patch on demand; options with their descriptions, every question of a multi-question `AskUserQuestion`; the agent's last message; PTY tail only when nothing better exists; cost), its actions, and a **lifecycle**: open → decided (by user / policy / overwatch / pane) → sent → effect (resumed, queued with ETA, PR opened `<url>`, failed: why), or expired → parked (FR-705), or stale. Every kind the core produces has a payload (the restart gate included); none is reduced to plain strings. | M |
| FR-1304 | **Cards in the Overwatch chat** replace the Inbox pane: the core posts a decision's card **at once**, without the LLM; the card shows its context blocks and actions, its lifecycle as it moves (a toast never claims an action before it is sent), and is threaded under its run (or its agent for a solo session), so a follow-up question sits under the earlier answer. Settled decisions collapse to one line that expands. Level 4 (§9.6) is a **digest of outcomes**, not of turns delivered. Keyboard as FR-707. [J5] | M |
| FR-1305 | **Chat log**: entries by `core` (cards, outcome lines, the morning line), `overwatch` (text, collapsed decision lines, annotations, rule proposals) and `user` (messages to the Overwatch, which wake it). One thread per working day; older days are searchable. | M |
| FR-1306 | **The Overwatch agent**: one global Claude Code session with a fixed role (default model Sonnet), **event-driven**: suspended between events and resumed by the core with a batch of new events. Blocking events (a decision it may settle, a stuck agent, the user's message) wake it at once; others are batched (default 30 s). One session per working day, started with a briefing the core writes. It has its own slot (FR-203), runs at the critical tier, and its usage is attributed to "overwatch" (drawn from the reserve). Read-only access to registered projects; no Edit, Write or Bash; its tools are the orchestrator's (FR-1307). If it is down or slow, nothing waits on it: the cards are already posted. [J12] | M |
| FR-1307 | **Overwatch tools**: `world()` (the attention snapshot), `inspect(agent)` (task, last messages, transcript tail), `decide_permission(decision, allow \| deny, reason)`, `answer_question(decision, answer, sources)`, `annotate(decision, note)`, `nudge(agent, text)`, `resume_agent(agent)`, `restart_fresh(agent)`, `reprioritize(queue_entry, tier)` (inside the weekly plan's allocations), `propose_rule(pattern, effect, scope)`, `post(text)`. It has **no** tool to start sessions, runs or tasks, decide spec or PR gates, change the weekly plan or Pause all. | M |
| FR-1308 | **Permission policy**: (1) a **hard-deny list** in the core: writes outside the agent's worktree, credential paths, push, merges into protected branches, PR creation, and any call an *ask* rule sends to the user; (2) the user's **policy file**, global and per project (`.orchestrator/policy.toml`, or app data when `.orchestrator/` is not tracked): rules over tool, input pattern and role with effect `allow`, `deny`, `overwatch` or `user`; the store's permission rules (FR-708) are entries of it; (3) **learned rules**: after the user gives the same answer to the same kind of call 3 times, the Overwatch proposes a rule as a card, and one click writes it to the policy file. Order: hard deny → policy → Overwatch (when the effect is `overwatch`, the default for team roles; it has 60 s, then the card waits for the user) → user. | M |
| FR-1309 | **Questions and unsticking by the Overwatch**: it answers an agent's question itself only when the answer is grounded in the feature's docs, `decisions.md` or an earlier answer of the user's, and cites the source in the answer and in its collapsed line; it escalates product, scope, money, security and irreversible calls, and anything it is unsure of, by annotating the card with its recommendation. It receives the FR-411 signals for every team and solo session and may nudge, resume, restart fresh (FR-415) or reprioritize. | M |
| FR-1310 | **Oversight**: every Overwatch decision is a collapsed line in the chat ("allowed front-1: `pnpm add zod` · policy: installs"), expandable to its reason and sources, with **undo** while it still means something (revoke the rule it used, tell the agent the answer changed, re-queue); all of them are audited (FR-709). | M |
| FR-1311 | **Pane interventions are measured**: the user typing into an agent's pane while its attention is `waiting` or `idle` with an open task is recorded as a `pane_intervention` event (agent, what it waited on, how long). The weekly report and the P4 exit criteria count them; the count by cause is the baseline for reducing interventions phase after phase. | M |
| FR-1312 | **Intake source per project**: GitHub (label, milestone or project), Linear (team, project or label), **docs in the repo** (a glob, e.g. `.orchestrator/backlog/*.md`), or none. Items appear in Mission control as *Ready to start* with an estimated cost; starting one (solo session or team run) is always the user's action. With no source, "Plan with the Overwatch" turns a conversation into a feature brief the user then launches. | S |

---

## 5. Non-functional requirements & acceptance tests

| ID | Category | Requirement (testable) |
|---|---|---|
| NFR-1 | Responsiveness | UI input-to-paint < 100 ms at p95 with 4 live agents, 6 terminals and 200 queued items; transcripts and lists virtualized. PTY output uses the **pull transport** (ADR-001): with 6 panes streaming, P0 measured frames at p95 35 ms / max 50 ms, against 2–5 s UI stalls with Tauri Channels. |
| NFR-1b | **Memory (primary constraint)** | Measured as **phys_footprint** (RSS is misleading under memory pressure: 66–315 MB for the same process in P0). App (Tauri + WebContent/GPU/Network helpers + Rust core) ≤ 250 MB with nothing running; **measured in P0 on an empty page: about 75 MB idle**, 88 MB with 1 DOM pane, 104 MB with 6 DOM panes; **measured with the full app shell (2026-10-26): 150–200 MB idle** (the GPU helper holds 40–90 MB while the shell repaints), 242 MB with 1 agent pane, 254 MB with 3 (with WebGL: about 220 MB with 1 pane, 240 MB with 6). Test: 6 panes after heavy output ≤ 400 MB. The app's memory display counts the GPU helper. Measure with the screen unlocked and the window visible (WKWebView throttles otherwise). Each shell outside the app ≈ 5 MB. Each live agent ≈ 250 MB (P0: 165–210 MB idle, 230–255 MB working; 189–206 MB with `--strict-mcp-config`, which saves about 25 MB; 3 agents ≈ 0.6 GB). Runtime measurements (2026-10-21, haiku, `--strict-mcp-config`, about 26 % free RAM): 189–200 MB idle, 207–216 MB working, 204–210 MB after a resume. Default cap 3 live agents, lowered under memory pressure (FR-203). |
| NFR-2 | Resilience | After a crash every agent has exited (the core owns their PTYs); every session can be resumed by id, queues and plans are intact, worktrees reconciled (J10 reconciler). **Not guaranteed**: programs running inside shell panes (restarted in their cwd only), and an interrupted **non-idempotent** command can leave its partial effect twice after a resume (the resume message asks the agent to check first). App-delivered turns are never delivered twice (turn id markers). Test: kill -9 the app with agents idle, mid-tool, held in a gate and mid-merge, restart, and check nothing is lost or delivered twice (passed in the P0 recovery spike). |
| NFR-3 | Security | Sandbox and credential rules of FR-702/703. Orchestrator MCP traffic goes over a Unix socket with 0600 permissions, with no network port; its path stays under macOS's 104-byte limit (checked at start) and a stale socket file is removed at start. Secrets only in the macOS Keychain. Test (ADR-009): an agent prompted to write outside its worktree, read another worktree or a credential file, `curl` an unlisted host, `ssh -T git@github.com`, `gh api user` or `git push` (directly or inside a script) must fail. |
| NFR-4 | Privacy | Local-first; no telemetry. OpenTelemetry from Claude Code is exported only to the app's local receiver. |
| NFR-5 | Observability | Structured logs per agent; a debug pane with raw hook, statusline and OTEL events. |
| NFR-6 | Portability | macOS (Apple Silicon) first; no macOS-only APIs in the core except behind interfaces (power assertion, notifications, Keychain). |
| NFR-7 | Compatibility | A supported Claude Code version range is declared (P0 baseline: 2.1.302); the app checks it at every spawn and refuses or warns on untested versions (hooks, flags and statusline formats can change). Auto-update is disabled for agents (FR-1201). The contract tests include a missing hook binary (must not let tools run) and a hook with the core down. git ≥ 2.48 is required (relative worktree paths). |
| NFR-8 | Data | SQLite (WAL, one writer thread) in app data, with versioned migrations; large outputs stored outside the rows; retention for raw events (default 30 days), forecast snapshots 30 days, usage samples 90 days, decided Inbox items 90 days; the audit log is kept. |
| NFR-9 | Pacing quality | In the simulator (FR-820): with a synthetic week whose demand is 130 % of the allowance, the policy never hits 100 % before the reset during working hours and ends between 85 and 100 %; with 60 % demand it never queues work. |
| NFR-10 | Compliance | Agents only use the official Claude Code client logged in with the user's subscription, with no API key. Automatic and unattended runs (several agents, off-hours, FR-816) are allowed, always inside the plan's 5h and weekly limits, which the Governor enforces (R1 decision, §10). The binary is never modified and the app never handles Claude credentials. |

---

## 6. Architecture

Stack (ADR-001): **Tauri 2** shell (system webview), a light UI framework (**Svelte 5**, picked in P0), a **Rust core**, and **`claude` CLI processes** spawned directly by the core in PTYs. There is no Node runtime.

```mermaid
flowchart LR
  subgraph UI["Webview UI"]
    OV[Overview]
    PN[Panes: agent / shell / nvim / diff]
    CV[Team canvas]
    FB[Feature & board]
    WP[Week planner]
    IN[Inbox]
  end

  UI <-- "Tauri IPC (pull for PTY output)" --> CORE

  subgraph CORE["Rust core"]
    PS[Project & Git service]
    AM[Agent manager]
    TR[Team runtime]
    GOV[Governor]
    UC[Usage collector]
    GI[Gate & Inbox]
    TB[Task backends]
    PTY[PTY service]
    BUS[Local bus: Unix socket]
    OTL[OTLP receiver]
    DB[(SQLite)]
  end

  AM --> PTY --> CL["claude processes (one per live agent)"]
  CL -- "hooks (orch-hook)" --> BUS
  CL -- "statusline (orch-status)" --> BUS
  CL -- "MCP stdio (orch-mcp)" --> BUS
  CL -- OpenTelemetry --> OTL
  BUS --> GOV & GI & TR & UC
  OTL --> UC --> GOV
  GOV --> AM
  TB --> GH[GitHub] & LN[Linear] & LOC[Local]
  CL <--> EXT[External MCP: Penpot, Figma, Notion…]
  PTY --> SH[login shells, nvim]
```

### 6.1 Principles
1. **Claude Code is the agent runtime.** The app never re-implements the agent loop or the chat. It **configures** each `claude` process, **observes** it (hooks, statusline, OTEL, transcripts) and **steers** it (turn delivery, hook decisions, MCP tools, PTY input).
2. **The Governor controls every autonomous turn.** Autonomous work only starts when the app delivers it, and PreToolUse hooks give it a pause point at every tool call. Interactive typing is never blocked.
3. **Agents coordinate only through the orchestrator MCP server**, which enforces the team graph.
4. **Git is the integration medium**: work happens on branches and worktrees; artifacts are files in `.orchestrator/`.
5. **Small helper binaries** (`orch-hook`, `orch-status`, `orch-mcp`, shipped with the app) connect each `claude` process to the core over a Unix socket. The agent's identity comes from the environment the core sets when it spawns the process.

### 6.2 Agent lifecycle
`created (queued, no process: "Not started yet") → (governor admits, or the user presses Start now) → working ⇄ waiting_for_you | waiting_gate | queued | paused | blocked_by_limit → idle → suspended (process stopped, resumable) → archived`, plus `error` and `killed`. `paused` holds the agent at a turn boundary with its process alive until Resume. A Stop with a non-empty `background_tasks` is **not idle**: background work is still running and will start a new turn by itself. The mapping to UI labels is in §9.5.

Team run: `draft → spec → spec_gate → planning → executing → integrating → pr_gate → done`, plus `paused`, `deferred` and `failed`.

### 6.3 How an agent is configured (compiled from its Role)
For each agent the core writes a per-agent directory in app data and launches `claude` in the worktree. Flags verified in P0 (Claude Code 2.1.302):
- `--settings <file>`: the per-agent settings layer (loaded on top of the user's settings; its `statusLine` replaces the user's).
- `--mcp-config <file>` and, when the role says so, `--strict-mcp-config` (ignore the user's other MCP servers; also saves memory).
- `--setting-sources` when the role should not load the project's or the user's settings.
- `--model`, `--effort`, `--append-system-prompt`, `-n/--name`.
- `--session-id <uuid>` on first start, `--resume <id>` afterwards (`--fork-session` to fork).
- `--permission-mode` **always passed explicitly**, one of `manual | acceptEdits | auto | dontAsk | plan` (`bypassPermissions` never). Otherwise the user's own `defaultMode` applies to agents.
- **Never** `--bare` or `--safe-mode`: both disable hooks, including the app's.

The generated settings contain:
- **hooks** → `orch-hook` for SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, PostToolUseFailure, PostToolBatch, Notification, Stop, StopFailure, SubagentStart/SubagentStop, PreCompact and SessionEnd, each with a generous `timeout` (600 s) that `orch-hook` never reaches (FR-806);
- **statusLine** → `orch-status` with `refreshInterval: 5` so idle agents keep reporting; it forwards the JSON to the core and then runs the user's own statusline command, if any, and prints its output;
- **sandbox** and **permissions** from the role's profiles (FR-702, ADR-009), with the orchestrator's own tools (`mcp__orch__commit`, `mcp__orch__request_pr`) allowed: the core applies its rules when it runs them;
- **MCP**: the orchestrator server `orch` (`orch-mcp` over stdio, the agent id and socket in its environment) in `--mcp-config`, always present on top of the role's servers;
- **environment**: built from scratch (not inherited from the app), with PATH taken from the user's login shell (user plugin hooks need tools like `node`), the agent id and socket path, `DISABLE_AUTOUPDATER=1`, OpenTelemetry to the local receiver tagged with `OTEL_RESOURCE_ATTRIBUTES=orch.agent_id=<id>`, the generated git config, and no credentials (FR-703).

**Orchestrator preamble**: every compiled prompt includes a short `--append-system-prompt` section saying that text starting with `ORCH:` in hook context comes from the orchestrator on the user's behalf. Without it, the model treats hook context as untrusted and ignores it (seen in P0). It also tells the agent to commit with `mcp__orch__commit` (not `git commit`) and to ask for PRs with `mcp__orch__request_pr`.

User customizations still load into every agent (CLAUDE.md, skills, plugins and their hooks, claude.ai connectors, user MCP servers, permissions) unless the role turns them off with `--strict-mcp-config` / `--setting-sources`.

### 6.4 Signals and control channels (verified in P0 unless marked)
| Need | Channel |
|---|---|
| 5h / weekly %, reset times, context %, cost, prompt-cache state | statusline JSON via `orch-status`: `rate_limits.five_hour/seven_day.{used_percentage, resets_at}` (integer %, epoch s), `context_window`, `cost`, `prompt_cache` (`ttl`, `expires_at`, `recache_tokens_if_cold`). Missing until the first API response of a session. |
| Tokens and cost over time per session | OpenTelemetry (OTLP http/json, local only): metrics `claude_code.token.usage`, `cost.usage`, `active_time.total`, `session.count`…; events `api_request` (tokens, cache, cost, duration), `tool_result`, `tool_decision`. Records include the user's email and account ids, so the receiver stays on localhost. No rate-limit data here. |
| State (working, waiting, idle, finished) | Hooks: UserPromptSubmit, PreToolUse, Stop, Notification (`permission_prompt`, `idle_prompt` ~60 s after Stop), SessionStart (`startup`/`resume`), SessionEnd. Nothing fires on SIGKILL. UserPromptSubmit **also fires for a message queued during a turn**, with the running turn's `prompt_id`: same `prompt_id` = not a new turn. Stop with a non-empty `background_tasks` is not idle (§6.2). SubagentStop with an empty `agent_type` and no SubagentStart (internal helpers, compaction) carries no state. SessionEnd `reason` is `other` both on crash and on suspend, so the core uses its own intent. |
| Tool admission | PreToolUse decision `allow` / `deny` with a reason. Deny rules in permissions win over a hook's `allow`. When the Governor admits a call the hook answers nothing, so the CLI's own permission flow still applies; it denies with "paused by governor" after holding a few seconds. |
| Permission prompts and short gates | **PermissionRequest** hook answering `allow`/`deny` (no dialog shown). |
| Audit of rule-denied calls | **PostToolBatch** (calls denied by permission rules reach neither PreToolUse nor PermissionDenied). Sandbox violations only appear in tool output. |
| API / rate-limit errors | **StopFailure** hook (verified): `error` = `rate_limit` or `model_not_found`, with `last_assistant_message`; no Stop follows. On a plan-limit 429 the statusline shows `used_percentage: 100` and the CLI arms its own auto-continue at the reset (J9). |
| Inject messages into a busy agent | PostToolUse `additionalContext`, prefixed `ORCH:` (needs the preamble above). |
| Deliver a new turn to an idle agent | PTY bracketed paste + Enter while idle and the user isn't typing, following the FR-206 delivery rules. |
| Keep an agent working on queued tasks | **Not the Stop hook** (P2): a Stop `block` keeps the turn running, so the state tracker and the Governor would see one endless turn. The agent stops; its next task is queued as a normal turn and delivered through the Governor (FR-206), which costs one delivery round trip. |
| History and search | Claude Code's transcript JSONL (`transcript_path` in every hook payload). |

The contract test suite (R2) replays these checks after every Claude Code update.

### 6.5 Security model (ADR-009)
Per-agent OS sandbox for Bash, permission rules for Claude's own tools, no credentials, push and PR only through gated MCP tools, and an audit log of every gate and permission decision.

---

## 7. Data model (first cut)

| Entity | Main fields |
|---|---|
| Project | id, name, path, defaultBranch, remotes, worktreeRoot, taskBackend {type, config}, intakeSource? {type: github / linear / repo_docs / none, config} (FR-1312), trackOrchestratorDir, settings |
| Feature | id, projectId, slug, title, brief, base, status, integrationBranch, integrationWorktreeId, priority, allocationId (default: the project) |
| Role | id, scope, name, icon, prompt, model, effort, tools, mcpServers, permissionProfile, sandboxProfile, turnCeiling, subagentsAllowed, budgetPriority, version. Keyed by (scope, id): a project's role shadows the global one with the same id in that project only |
| Team | id, scope, name, nodes[], edges[], settings, version. Keyed by (scope, id), like Role |
| TeamVersion | teamScope, teamId, version, team, roles (the roles its nodes use): the snapshot a run points at (FR-308), kept after the team is archived |
| AgentNode | id, teamId, roleId, overrides, position, maxConcurrency, isLead |
| Edge | id, teamId, from, to, type |
| TeamRun | id, teamId, teamScope, teamVersion (→ TeamVersion), compiledConfigPath, featureId, status, resumeStatus? (where a paused or deferred run goes back to), prUrl?, startedAt, endedAt |
| Worktree | id, projectId, path, branch, baseBranch, locked, state, createdAt |
| Agent | id, projectId, teamRunId?, agentNodeId?, featureId?, taskId? (its current task; the team seat), worktreeId, claudeSessionId, parentAgentId? (fork), pid?, state, priority, lastActiveAt |
| Turn | id, agentId, kind (task / message / continue / user), admittedAt, startedAt, endedAt, usageDelta, outcome |
| Task | id, projectId, backend, externalId?, featureId, teamRunId?, title, body, status, assigneeNodeId?, agentId?, dependsOn[], branch?, prUrl?, notes[], createdAt, updatedAt, remoteUpdatedAt? (the remote's last change seen, or a pending local change, FR-503) |
| Message | id, teamRunId, from, fromAgent?, to (a node), toAgent? (set when one agent of the node receives it), kind, body, ref, deliveredAt?, readAt? |
| Artifact | id, featureId, kind (doc / design / build), uri, fromNodeId, toNodeId? |
| InboxItem (Decision, v0.12) | id, kind (permission / question / spec_gate / pr_gate / restart_gate / budget_ask / alert / startup / rule_proposal / planner), subject {agentId?, role?, projectId?, featureId?, runId?, taskId?}, context[] (typed blocks: tool_call {tool, input, cwd} / markdown / diff_summary {files[{path, added, deleted}]} / options[{label, description}] / last_message / pty_tail / cost), actions[], threadId (run or agent), priority, status (open / decided / sent / effect / expired / parked / stale), decidedBy? (user / pane / rule:<id> / overwatch / expired), decision?, effect?, annotation? (the Overwatch's recommendation), createdAt, decidedAt? |
| ChatEntry | id, ts, day, author (core / overwatch / user), kind (text / card / decision_line / outcome / annotation / rule_proposal / morning), refs {decisionId?, agentId?, runId?}, body |
| PermissionRule | id, scope (global / project / role / agent), pattern, effect (allow / deny / ask / overwatch / user), source (user / learned / inbox), sourceInboxItemId?, createdAt, revokedAt? (revoked rules are kept for the audit trail). Mirrors the policy file (FR-1308). |
| Attention | (computed, not stored) subject (agent / run), level, waitingOn {kind, ref?, since}, reason, activity? (FR-1301) |
| Hold | id, agentId, toolUseId?, kind (gate / governor), status (held / allowed / denied / interrupted), summary, inboxItemId?, createdAt (a held hook; the reconciler needs them, J10 step 7) |
| AuditEntry | id, ts, actor (`user` / `core` / `rule:<id>` / `agent:<id>`), agentId?, action (permission / commit / pr / rule_added / …), target, decision, detail (FR-709, never purged) |
| UsageSample | id, ts, agentId? (null = unattributed), turnId?, model, tokens {in, out, cacheRead, cacheWrite}, costEstimate, costDelta, window {fiveHourPct?, weeklyPct?, modelWeeklyPct?, resetsAt?}, provenance (observed / estimated), source (statusline / otel), costClass (interactive / critical / autonomous), allocationId |
| WeeklyPlan | id, weekStart, resetAt, calendar, reservePct, offHoursAllowancePct, shadowMode, status |
| BudgetAllocation | id, weeklyPlanId, target, weight, tier, hardCapPct? |
| PacingPolicy | id, thresholds {amberEnter, amberExit, redEnter, redExit, catchupEnter, minDwellMin}, liveCapByState, fiveHourGuardPct, askTimeoutMin, defaultAskAction, unattributedPriorPctPerHour, unattributedPriorHours |
| QueueEntry | id, kind, agentId/taskId, allocationId, tier, enqueuedAt, effectiveTier, reason, estimatedStart (a view of the Governor's queue; after a restart turns that waited in it are asked again) |
| GovernorAction | id (per core start: stored with the start time), ts, state, action, targets, reason, mode (auto / ask / shadow), expectedEffect, outcome, undoneAt? |
| Calibration | id, ts, costPerWeeklyPct, confidence, unattributedRate |
| ForecastSnapshot | id, ts, used, ideal, governed, error, projected, eta, state |
| Terminal | id, projectId, agentId?, cwd, shell, layout |

---

## 8. Provider interface (kept small, G8)

Only one implementation is planned: **Claude Code CLI**. The interface exists so the core doesn't depend on Claude Code details everywhere.

```rust
trait Provider {
    fn capabilities(&self) -> Capabilities;           // resume, fork, statusline limits, hooks, sandbox…
    fn compile(&self, role: &Role, ctx: &CompileCtx) -> AgentConfig; // files + flags + env
    fn spawn(&self, cfg: &AgentConfig, resume: Option<SessionId>) -> Result<AgentProcess>;
    fn deliver(&self, agent: &AgentProcess, input: Input) -> Result<()>; // PTY input, only when idle
    fn interrupt(&self, agent: &AgentProcess) -> Result<()>;
    fn stop(&self, agent: &AgentProcess) -> Result<()>;  // for suspend, kill
}
// Observations arrive as normalized events on the local bus:
// HookEvent, StatusSample, OtelSample, StateChange, LimitReached, Exited.
```

---

## 9. User interface

### 9.0 Penpot file: the UI source of truth
The UI is **designed in Penpot before it is built** (ADR-008), with a gate sized for one person.
- **File**: `Orchestrator — UI`, linked from `docs/design/README.md`.
- **Pages**:
  | Page | Content |
  |---|---|
  | `00 Cover` | Version, changelog, links, and the status of each page: **Not started**, **In review** (designed, waiting for the user's review) or **Ready for dev** (approved; only the user sets it). |
  | `01 Foundations` | Tokens: colors (dark and light, fixed status colors), typography (UI font + mono font; the mono token accepts the font from `kitty.conf`), spacing, radius, borders, elevation, icons, motion. |
  | `02 Components` | The components in §9.4. **States are component variants** (default, hover, focus, disabled, loading, error…). |
  | `03 Shell & layouts` | Title bar, sidebar (full + icon rail), panes, inspector (docked + overlay), status bar; breakpoints 1280 / 1512 / 1728; zoomed pane. |
  | `04 Screens` | The pane types of §9.3, normal state, plus the empty and error states that differ from the component variants. |
  | `05 Keyboard & copy` | The full shortcut map (global, leader and pane-local keys of §9.7) with conflict notes; banner, notification and toast copy. |
  | `06 Flows` | Storyboards of **J2, J5 and J8** only: annotated frame sequences with the key sequence of each step (no clickable prototypes). |
  | `07 Explorations` | Drafts, not binding. |
- **Tokens pipeline**: Penpot tokens are exported as W3C design-token JSON, converted to CSS variables with Style Dictionary (a script in the repo), and a CI check fails on drift. Code uses tokens only.
- **Process**: new screens are designed first. Small changes may be made in code and **backported to Penpot in the same PR**. UI/UX agents can work on the file through the Penpot MCP.
- **Gate**: Foundations, Shell and the P1 screens (Overview, agent pane, Week planner, Inbox) must be in Penpot before P1 UI work starts. **P4 gate** (v0.12): the new shell (board + chat column), Mission control, every decision card kind with its lifecycle states, the collapsed Overwatch lines and the session overlay must be in Penpot (pages `03`, `04`, and a `06` storyboard of J12) before P4.2 UI work starts.

### 9.1 Design principles
1. **Keyboard-first**: every action is in the command palette (⌘K) and has a shortcut (§9.7). The mouse is optional, including on the canvas.
2. **Terminal-native look**: calm and dense, mono where content is code or shell. Theme precedence: **user choice > derived from kitty (if enabled) > system**. Deriving from kitty maps only *surface, foreground and accent* from the kitty palette, runs a contrast check (falls back if it fails), and **status colors stay fixed**.
3. **Status always visible**: pacing, live agents, Inbox count and git state are always on screen.
4. **Everything is a pane**: agent, shell, nvim, diff, board, feature, canvas and planner are pane types in a tiling workspace. Layouts are saved per project. From P4, Mission control is the home pane and the chat a fixed column; agent sessions open on demand.
5. **Progressive disclosure**: raw events and configs are hidden until asked for.
6. **Light on resources**: no blur, video or long animations; hidden panes don't render; lists are virtualized.
7. **Consistent agent identity**: each role has a color and icon, used in the sidebar, panes, timeline, board and canvas.
8. **Accessible**: visible focus rings, color never the only signal, reduced-motion support, UI font scaling, throttled `aria-live` for streaming status.
9. **Nothing requires a pane** (v0.12, G10): every decision is answerable from a card, every wait is visible with its cause; the Claude Code pane is for watching or taking over, never for unblocking.
10. **One source of state**: surfaces render the core's attention records (FR-1301) and never derive state themselves; one push channel, no per-pane polling.
11. **Every number says what it is**: a unit or a label on every figure ("+2 pp vs plan", "3 files changed", "ctx 7 %", "waiting 12 min"); no bare `±31` or `(+2)`.

### 9.2 App shell & navigation
```
┌ Title: [⌘K]  Week 50% · agents 41% · plan 38% (+3) · 5h 22% · resets Mon 09:00  [Inbox 3] ┐
├ Sidebar ─────┬ Workspace (tiling panes) ────────────────────────────────┬ Inspector ──────┤
│ ▾ shop-api   │ ┌ claude · back-1 ────────┐┌ nvim ────────────┐          │ (selection:     │
│   ▾ checkout │ │ Claude Code TUI         ││                  │          │  agent, task,   │
│     ● lead   │ │                         │├ zsh ─────────────┤          │  node, gate)    │
│     ● back-1 │ │                         ││ $ pnpm test      │          │                 │
│   ◌ solo/fix │ └─────────────────────────┘└──────────────────┘          │                 │
├──────────────┴──────────────────────────────────────────────────────────┴─────────────────┤
│ ⎇ orch/checkout/integration ↑2 · 3 live · 2 queued (Amber) · RAM 1.4 GB                   │
└───────────────────────────────────────────────────────────────────────────────────────────┘
```
**From P4.2** (v0.12) the shell becomes:
```
┌ Title: [⌘K]  Week 50% · agents 41% (+3 pp vs plan) · 5h 22% · week resets Mon 09:00   ─┐
├ Sidebar ─────┬ Mission control (home)    ───────────────────┬ Overwatch ───────────────┤
│ ▾ shop-api   │ NEEDS YOU 1          → card in the chat      │ ▸ allowed back-1:        │
│   ▾ checkout │ RUNNING 2                                    │   cargo add serde · pol. │
│     ● lead   │  shop-api › checkout  ●●●○○○ executing       │ ┌ Spec gate · checkout ┐ │
│     ● back-1 │   lead ✓ · back-1 T-4 · front-1 ⧗ back-1     │ │ 3 docs · +142 lines  │ │
│   ◌ solo/fix │  blog › solo/fix  writing src/feed.ts        │ │ ow: scope matches    │ │
│              │ WAITING ON 1                                 │ │ [a approve] [o open] │ │
│              │  front-1 → back-1 · API contract · 12 min    │ └──────────────────────┘ │
│              │ QUEUED 1  infra/deps · cap 2/2 · ≈14:20      │                          │
│              │ DONE TODAY  blog #12 PR opened ↗             │ > ask the Overwatch…     │
├──────────────┴──────────────────────────────────────────────┴──────────────────────────┤
│ ⎇ orch/checkout/integration ↑2 · 3 files changed · 3 live / cap 3 · Amber · RAM 1.4 GB │
└────────────────────────────────────────────────────────────────────────────────────────┘
```
Opening a row's session shows its Claude Code pane over the board, as a tab (Esc returns to Mission control); shells stay in the dock. The Inspector shows the selection's details (agent, run, task or decision) and follows the selection in Mission control.

- **Sidebar** (⌘B): Projects → Features → agents, and solo sessions **under their project**. It is the only navigation tree. ⌘P is a fuzzy jump into it. Below 1512 px it collapses to an icon rail with status dots.
- **Workspace**: tiling panes with splits, tabs and zoom, over the **shell dock** (one tab per shell, resizable, ⌘J).
- **Inspector** (⌘I): docked at ≥ 1512 px, overlay below that.
- **Status bar**: git state of the focused project, live/queued agents with the pressure state in words, a **5h guard** chip whenever the guard holds work ("5h guard · cap 1 until 13:00", in any pressure state), memory.

### 9.3 Pane types
| Pane | Purpose | Key states |
|---|---|---|
| Mission control (P4.2, replaces Overview) | Home: lanes Needs you / Running / Waiting on / Queued / Done today, team runs as rows with a phase stepper, filter per project (FR-1302) | no projects (→ J0), nothing running, everything waiting on you |
| Overwatch chat (P4.3, fixed column) | Conversation with the Overwatch; decision cards; collapsed Overwatch lines; outcome digest (FR-1304, FR-1305) | Overwatch asleep / waking / down (cards still work), no decisions, many open |
| Overview (until P4.2) | All projects and agents in one place (replaces "Projects home" + "Monitor"); group by project or by state | no projects (→ J0), nothing running |
| Agent | Claude Code in a PTY, with a header: role, model, context %, usage, governor status, actions | working, waiting for you, queued (why/ETA/Run now), not started yet (queued, no process; Start now), paused (Resume), suspended (Resume with ⏎ or a click: resuming spends quota, so focus alone does not resume), blocked by limit, error |
| Shell | Login shell | — |
| Editor | nvim | — |
| Diff | Worktree diff | no changes, conflict |
| Project | Branches/worktrees, features, settings | not a git repo, dirty, conflicts |
| Feature | Docs (rendered, open in nvim), tasks, team timeline, design artifacts, PR | draft, spec gate, executing, PR gate, done |
| Board | Tasks over any backend | backend disconnected |
| Team canvas | Builder + Rules panel | invalid (inline errors), unsaved |
| Week planner | Ideal vs actual (week and 5h), allocations, forecast, policy, governor log, next-action preview | no plan yet, shadow mode, Green/Amber/Red/Catch-up, estimated (low confidence) |
| Inbox (until P4.3) | Triage | empty, many pending, stale items |
| Settings | Claude Code, MCP registry, integrations, terminal/editor, pacing policy, keyboard, memory | integration disconnected |

**Global banners** (component inventory): limit reached + reset time (one variant per window: weekly and 5h), login expired, Claude Code version unsupported, Governor in Red, live-agent cap reached, offline, not on AC power with off-hours work queued.

### 9.4 Component inventory
- **Shell**: title bar with pacing indicator (compact/expanded), sidebar item (status dot, badge), icon rail, pane, pane header, pane tab, split handle, zoom indicator, inspector (docked/overlay), status bar, command palette, fuzzy jump, toast, banner.
- **Agents**: agent chip (role color + icon + name), status badge (§9.5), agent row (Overview), action menu, queued explanation ("Queued: low priority, week +9 pp, starts ≈14:20 · Run now ≈ +2 pp").
- **Pacing**: two-window indicator, curve chart (ideal/actual/forecast, week and 5h), allocation row with weight slider and tier, pressure-state pill, governor log entry with undo, next-action preview, confidence marker.
- **Work items**: task card, kanban column, feature card, PR/CI badge, branch/worktree row, git summary.
- **Canvas**: node, edge (4 styles + labels), port, minimap, Rules panel, inline validation, role inspector form, preset card.
- **Inbox**: item per kind with preview area, keyboard hints, stale marker, batch bar, undo toast.
- **Mission control & chat** (v0.12): lane, run row (phase stepper, member chips with their wait), agent row, waiting-on chip, queue row (reason, ETA, Run now), outcome row; **decision card** per kind with its context blocks (tool call, rendered markdown, diff summary, options, last message, PTY tail, cost) and lifecycle states (open, deciding, sent, effect, failed, parked, stale), the Overwatch's annotation, collapsed decision line (expand, undo), rule proposal card, morning line, chat input, Overwatch status (asleep, waking, working, down).
- **Base**: button, icon button, input, select, toggle, segmented control, slider, tabs, table, tooltip, dialog, empty state, skeleton, shortcut hint.

### 9.5 Status vocabulary (single source)
| UI label | Internal state | Meaning |
|---|---|---|
| Working | working | Claude is generating or running a tool |
| Waiting for you | waiting_for_you | Permission prompt or question in its pane |
| At gate | waiting_gate | Waiting for an Inbox decision (process may be suspended) |
| Queued | queued | Held by the Governor (budget, live-agent cap or 5h guard). An agent that never ran has **no `claude` process** yet: it reads "Not started yet" and offers **Start now** with its cost |
| Paused | paused | Stopped at a turn boundary by the user (pause, **Pause all**) or by the Governor (pause team); process alive; resumes only on **Resume** or when the Governor releases it |
| Blocked by limit | blocked_by_limit | A plan window is exhausted until its reset |
| Idle | idle | Turn finished, process alive |
| Suspended | suspended | Process stopped to save memory; resumes on demand |
| Error | error | Crashed or failed; needs attention |
| Done | (task done) | Assigned task finished |

Each label has one color and one icon everywhere (§9.0 Foundations).

**Waiting on** (v0.12, FR-1301): every Waiting, Queued, At gate and Waiting-for-you label is followed by what it waits on, and since when:

| `waiting_on.kind` | Shown as | Example |
|---|---|---|
| user | Needs you · the decision | "Needs you · permission WebFetch example.net · 2 min" |
| agent | → agent · what | "→ back-1 · API contract · 12 min" |
| task | blocked by task | "blocked by T-2 (back-1, in progress)" |
| governor | Queued · reason · ETA | "Queued · live cap 2/2 · ≈14:20" |
| limit | Blocked by limit · window · reset | "5h window full · resets 16:00" |
| startup | Not started · why | "Not started · startup dialog in its pane" |
| external | Waiting · on what | "Waiting · Penpot tab asleep" |

**Separate palettes** (v0.12): agent states (above), task statuses (Backlog / Ready / In progress / Review / Done), run phases (draft → PR) and decision kinds each get their own colors and icons; none borrows another's (a task in Review is not "At gate", a budget card is not "Queued", a failed run is "Failed", not "Stopped").

### 9.6 Attention ladder
| Level | What | Channel |
|---|---|---|
| 1 Blocking | An agent waiting for you or at a gate, sorted by waiting time | macOS notification (batched: "3 agents need you"), badge, sidebar dot |
| 2 Errors | Crash, watchdog, login expired, limit reached | Notification + banner |
| 3 Budget | Governor "ask" (with default/timeout), entering Red | Badge, toast |
| 4 Info | Task done, merge done, report ready | Silent: digest in the Inbox |

The Inbox badge counts levels 1–3; level 4 stays in the digest. Do-Not-Disturb holds levels 3–4 and batches level 1. **Jump to next waiting agent**: leader `w`.

From P4.3 (v0.12) the ladder applies to **cards in the Overwatch chat** instead of the Inbox pane: level 1–2 cards are posted at once and notify (a notification click opens the app on that card); what the policy or the Overwatch settles never notifies and appears as a collapsed line; level 3 cards carry their default and countdown ("applies in 18 min"); level 4 is the outcome digest. The badge counts open cards of levels 1–3 that wait on the **user** (not those the Overwatch is still deciding).

### 9.7 Keyboard contract
- When a terminal pane (agent, shell, nvim) has focus, **every key goes to the PTY** except ⌘-chords and the **leader key**. The default leader is **`⌘;`**: ⌘ never reaches the PTY, so it can't clash with nvim or the shell. (`Ctrl-Space` was rejected in P0: it is macOS's input-source shortcut and nvim-cmp's completion key.) Any leader can be configured; a non-⌘ leader is sent literally when pressed twice.
- **Shift+Enter** in agent panes sends `ESC CR`, so it adds a newline in Claude Code instead of submitting (P0).
- **⌘⏎ submits a dialog**: the chord handler passes it to the dialog even when focus is not in a text field.
- The app ships its own macOS menu, so system items (Close Window ⌘W, Hide ⌘H, Minimize ⌘M) don't take chords this contract uses.
- **Global ⌘ shortcuts**: ⌘K palette, ⌘P jump, ⌘B sidebar, ⌘I inspector, ⌘N new project, ⌘T new session, ⌘1…9 focus pane, ⌘W close pane, ⌘⇧T reopen the last closed pane, ⌘⇧I Inbox.
- **Leader + key**: `s`/`v` split horizontally/vertically, `h j k l` move focus, `z` zoom, `t` new shell, `e` open editor, `w` next waiting agent, `p` pause/resume the focused agent, `o` Overview, `g` Week planner.
- **Notation** (Penpot, docs, UI): one key-hint container per shortcut; `+` joins keys pressed together (`⌘+⇧+I`), a space separates the steps of a sequence (`⌘+; z` = leader, then `z`).
- **Pane-local single keys** when the pane has focus and is not a terminal: Inbox (FR-707, plus `e` open in nvim), Overview (`⏎` open, `i` interrupt, `p` pause, `s` suspend, `r` resume, `⇧K` kill (asks first; `k` moves up), FR-903), Week planner (`S` sprint mode, `P` pause all, `e` edit plan, `u` undo the selected log entry), canvas (FR-302). The full map is on Penpot page `05`.
- **From P4** (v0.12): ⌘⇧I focuses the chat on the oldest open card (cards take the FR-707 keys; `j/k` move between open cards, skipping collapsed lines); ⌘L focuses the chat input; leader `o` opens Mission control; in Mission control, `j/k` move rows, `⏎` opens the session over the board (Esc returns), `i`/`p`/`s`/`r`/`⇧K` act on the **selected** row, `f` filters by project.
- **Known limits** (P0): xterm.js has no kitty keyboard protocol, so `Ctrl-I` = Tab, `Ctrl-M` = Enter and `Ctrl-[` = Esc as in legacy terminals, and `Ctrl-/`, `Ctrl-.`, `Ctrl-;` and every `Ctrl-Shift-<letter>` send nothing. These are recorded on Penpot page `05`. Not yet tested with physical keys: ⌘-chords against the menu, dead keys and IME composition (R9).

### 9.8 Wireframes (low-fi, starting points for Penpot)

**Week planner**
```
┌ Week 42 · resets Mon 20 Oct 09:00 · state: RED (+9 pp) · shadow mode off ─────────┐
│ 100%│                                             ....ideal                         │
│     │                             ....´´´´  ___actual   - - forecast              │
│  50%│                 ....´´´´ ____/¯¯¯¯    - - - ⚠ empty Fri 14:00                │
│   0%└─Mon────Tue────Wed────Thu────Fri────Sat────Sun──   5h: 61% · resets 16:00    │
│ Allocations       weight  used  tier      status                                 │
│  shop-api           50     24%  high      over ▲                                  │
│  blog               30      9%  normal    on plan                                 │
│  infra              10      3%  low       queued (2)                              │
│  reserve            10      5%  —         outside app: 4%                         │
│ Next action: queue "infra/upgrade-deps" (low) · [Sprint mode] [Pause all]          │
│ Log: Wed 15:02 RED → live cap 1 · queued 2 (infra) [undo]                          │
└────────────────────────────────────────────────────────────────────────────────────┘
```

**Team canvas**
```
┌ Team: Full-stack feature ───────────────────────────┬ Role: Backend ─────────────┐
│        ┌───────┐                                    │ model   [opus ▾] effort[high▾]│
│        │ Lead  │                                    │ tools   ☑ edit ☑ bash ☐ web │
│        └─┬─┬─┬─┘  reports-to                        │ MCP     ☑ orch ☐ penpot     │
│   ┌──────┘ │ └───────┐                              │ perms   [gated ▾] sandbox[wt▾]│
│ ┌─▼────┐ ┌─▼─────┐ ┌─▼───┐  hands-off ┌────┐        │ budget  [normal ▾]          │
│ │Back  │ │Front  │◀┤ UI  │◀───────────┤ UX │        │ prompt  [open in nvim]      │
│ └──────┘ └───────┘ └─────┘            └────┘        ├ Rules ─────────────────────┤
│ [n] add  [c] connect  [⏎] inspect  [=] auto-layout   │ Backend → Lead (reports)   │
└─────────────────────────────────────────────────────┴────────────────────────────┘
```

---

## 10. Risks & open questions

| # | Risk / question | Mitigation / next step |
|---|---|---|
| R1 | **Compliance of automated use** — **resolved 2026-10-20 (go)**. Claude Code's Legal and compliance page allows "signing in to the unmodified Claude Code binary with their own Claude subscription", and limits "assume ordinary, individual usage". Anthropic support (2026-10-20) confirmed that limits are shared across all Claude surfaces, that parallel instances use quota proportionally, and that "sessions can run unattended (like overnight) as long as your machine stays on and you stay within your plan limits"; it declined to interpret the terms. **Decision (Guillaume)**: spawning several agents automatically and running unattended is acceptable for this personal tool, as long as it stays inside the subscription's limits. | Keep the conditions: unmodified binary, the user's own login through Anthropic's flow, no credential handling, personal use only, Governor-enforced limits. Re-check if the published terms or the Legal and compliance page change. |
| R2 | **Signal and control channels** (statusline `rate_limits`, hook fields, PTY idle detection, flags) are not a stable API and can change between Claude Code versions. | P0 spike against the installed version; declared supported range (NFR-7); a contract test suite that runs on each Claude Code update. |
| R3 | **Statusline data appears only after the first response** of a session, and `rate_limits` are **per-process copies of that agent's last API response**: idle refreshes re-render the agent's own last value and don't see other agents' usage (P0 recovery spike: after a 429 one agent read 100 % while others read 40 %; with fresh samples all agents agreed exactly). | Use the most recent sample across all agents per window (FR-801); keep values with their timestamp; estimator fallback (FR-814) with a confidence marker. |
| R4 | **PTY-driven input** can collide with the user typing or a TUI change; leading `/`, long pastes and tabs are altered (P0). | Deliver only when idle (Stop/Notification hook) and the pane has had no user keystroke for 2 s; the FR-206 delivery rules; prefer hook channels where possible. |
| R5 | **Memory**: each live agent is a full Claude Code process. | Live-agent cap, suspend/resume, P0 measurement (NFR-1b). |
| R6 | **Resume cost**: a resumed session re-pays its whole context once the prompt cache has expired (1 h TTL observed). | The Governor uses `prompt_cache.expires_at` and `recache_tokens_if_cold` from the statusline; resume before expiry when possible. |
| R7 | **Merge conflicts** between specialists. | Lead decomposes by module ownership; frequent integration; conflicts routed to the owner. |
| R8 | **MCP auth** (Penpot, Figma, Linear, Notion) inside agents. | Central registry, status, Inbox items (FR-604). |
| R9 | **Terminal rendering & input** — mostly retired by the [P0 terminal spike](spikes/p0-terminal.md): fidelity matches kitty after the FR-1002 fixes, and the WebGL context limit is handled (FR-1005). Still open: physical-key checks (⌘-chords vs the menu, dead keys, IME), Retina 2× / 120 Hz rendering, transparent windows (`background_opacity < 1`), why the first WebGL context costs about 110 MB, input latency (keydown → paint), and the OSC 52 clipboard. | Test with a human at the keyboard on the built-in display during P1 setup. |
| R10 | **Mostly answered** by the [P0 recovery spike](spikes/p0-recovery.md): long hook holds while the user types (fine, input is queued), SubagentStart/Stop and PreCompact payloads, StopFailure (with a simulated 429), several agents at once, trust in a worktree (keyed by the main repo). **Still open**: a real plan-limit hit and what the CLI's own auto-continue at the reset does to hooks and the statusline; how to turn off async subagents and background Bash per role; crash points in flows that don't exist yet (gate decision, PR creation); whether the sandbox confines detached `&` children. **New (2026-10-21, 2.1.302)**: (a) sandboxed Bash in a linked worktree cannot write the repository's git dir (`.git/worktrees/<name>/index.lock` → "Operation not permitted") whatever `allowWrite` or `--add-dir` says, with relative or absolute worktree links, although the Claude Code docs say linked worktrees get this grant automatically: agents can't commit; (b) the sandbox now blocks reads outside the working directory by default (the main checkout, `<repo>/.git/worktrees/`), which P0 did not see on the same version, so it is probably switched server-side; `allowRead` on the git dir restores git's reads. | Check during P1/P1b as each flow is built. Report (a) to Anthropic; meanwhile agents commit through the orchestrator MCP tool `commit` and the core runs the commit (FR-702; verified live 2026-10-26). Keep the explicit denies for (b), since the default may change again. |
| R11 | **Wrong answers by the Overwatch** (allows a risky call, answers a question with the wrong decision). | Hard-deny list in the core (FR-1308); answers only with a cited source, escalation otherwise (FR-1309); every decision visible as a line with undo and audited (FR-1310, FR-709); `overwatch` is opt-in per rule, defaulting to team roles only. |
| R12 | **Overwatch wake latency** on a cold prompt cache, and its context growth over a day. | Cards never wait on it (the core posts first); 60 s per permission before the card waits for the user; one session per working day with a core-written briefing. Measure wake time in P4.4 (target < 20 s warm). |
| R13 | **Auto mode's real effect is unknown**: the 2026-10-27 run was still in `acceptEdits`. | P4.0 exit: a measured team run in auto mode, counting prompts by tool and pane interventions, before the policy defaults are set. |
| Q1 | Should the Lead be able to add roles at runtime? | Default no; possible later behind a gate. |
| Q2 | ~~Overwatch chat retention, learned-rule threshold, per-project filter~~ — **resolved 2026-10-28**: one thread per working day with older days searchable (FR-1305); 3 identical answers before a rule is proposed (FR-1308), tuned after P4.4; a per-project filter in Mission control from P4.2 (FR-1302); wake target < 20 s warm with 60 s per permission (R12). | — |

---

## 11. Phasing

| Phase | Content | Exit criteria |
|---|---|---|
| **P0 — Spikes, go/no-go, UI foundations** | (1) R1 answer from Anthropic: **done** (go, see R1). (2) `claude` in a PTY: flags, per-agent settings, hooks, statusline `rate_limits`, OTEL, suspend/resume by id, PTY delivery while idle. (3) Memory: footprint per `claude` process, app baseline. (4) Sandbox scoped to the worktree, credentials removed, `git push` blocked. Items (2)–(4) **done**: see [p0-claude-cli.md](spikes/p0-claude-cli.md); open items in R10. (5) xterm.js in WKWebView with the user's `kitty.conf` + nvim, key conflicts, app memory baseline: **done**, see [p0-terminal.md](spikes/p0-terminal.md); open items in R9. (6) Crash/restart reconciliation: **done**, see [p0-recovery.md](spikes/p0-recovery.md); open items in R10. (7) **Penpot**: Foundations, Shell, P1 screens. | Each spike demoed; R1/R2 answered; NFR-1b numbers set; Penpot gate met. |
| **P1 — Solo power user** | Projects & git (4.1), agent panes with suspend/resume (4.2), terminal (4.10), editor (4.11), Overview (4.9), Inbox for permissions, sandbox and credential rules (FR-702/703), **usage collector + Week planner + read-only forecast** (FR-801–804, 813, 817), pacing simulator (FR-820). | Daily driver for solo work; forecast within ±10 pp of the observed weekly % at reset. |
| **P1b — Governor v1** | FR-805–812: live cap per state, queue, fair scheduling, pause/defer, 5h guard, transparency, shadow mode, overrides. | One real week in shadow mode, then one enforced week meeting G6. |
| **P2 — Teams** | Canvas & builder (4.3), orchestration runtime + MCP server (4.4), task backends Local + GitHub (4.5), gates (4.7). | A feature delivered by a team with 2 gates and no sandbox violation. |
| **P3 — Ecosystem** | Linear, Notion, Penpot/Figma design flow (4.6), presets, weekly report, estimator, per-model limits, off-hours scheduling, optional model downgrade. | Design → code flow on a real feature. |
| **P4.0 — Stop the bleeding** (v0.12) | Read allow on the agent's `messages/` folder and `delivered_at` at paste (FR-206); expired holds park the agent (FR-705); never-started sessions recovered (FR-415); memory-cap hysteresis and no pressure-suspend of a pasted turn (FR-203); who decided each permission (FR-709); Notification events stored with their type; `pane_intervention` events (FR-1311); the restart gate gets a payload. | A real team run **in auto mode**, with prompts by tool and pane interventions counted against the 2026-10-27 baseline (29 prompts, 6 interventions, ~5 h of agent time waiting). |
| **P4.1 — One "now"** | Attention records in the core, pushed on change (FR-1301); Decisions keep their payload and lifecycle (FR-1303); outcome digest; every existing surface renders attention (sidebar, rail, Overview, agent header, Inspector, status bar). | Every "needs you", live and queued count agrees across surfaces; no per-pane polling left. |
| **P4.2 — Mission control** | Penpot pass (§9.0 P4 gate), then Mission control replaces the Overview (FR-1302); sessions open over the board; the Inspector follows the selection. | "What is running, waiting on what, queued why, done today" answered on one screen. |
| **P4.3 — Chat & cards** | Penpot pass, then the chat column (FR-1305), cards for every decision kind with their context and lifecycle (FR-1304); the Inbox pane is removed; notification clicks open the card. | Every Inbox interaction available from a card; no decision needs the pane. |
| **P4.4 — The Overwatch** | The Overwatch role, its tools and wake scheduler (FR-1306–1307); the policy file and learned rules (FR-1308); questions and unsticking (FR-1309); oversight lines (FR-1310). | A team run where the user only uses Mission control and the chat: 0 pane interventions. |
| **P4.5 — Intake** | Intake source per project and the *Ready to start* lane (FR-1312). | A week's work started from intake items only. |
