# Fix Team Getting Stuck — Architecture decision: one session per agent, or a Lead that fans out subagents?

> Companion of [spec.md](spec.md). This file answers the brief's question. The spec holds the work.

## 1. The question

Teams get stuck, most often because a message sent from one agent never reaches the other. The brief asks two things:
- Is one Claude Code session per agent a sound way to run a team, and what does the industry say?
- Would it be better to run **one session, the Lead's**, and let the Lead fan out Claude Code subagents (the Agent tool) itself?

## 2. What actually breaks today (code trace, HEAD 54cdda4)

Messages don't travel *between sessions*. Every message goes to the core first, which stores it in SQLite (`messages`) and then **types it into the target's terminal** (PTY paste + Enter) or attaches it to a hook reply (`additionalContext`). The sessions never talk to each other directly. So the losses come from that **last step, the hand-over to the CLI**, and from the core's state tracking around it. They don't come from the idea of separate sessions.

| # | Failure | Where | Effect |
|---|---|---|---|
| F1 | **A paste the CLI never acknowledges wedges the agent.** Messages are marked delivered at paste. `ready_at` is zeroed and only a SessionStart or Stop resets it. Nothing times out a `Delivering` turn, and a delivering agent is never suspendable. | `runtime.rs:2258-2273`, `delivery.rs:138`, `resources.rs:177` | Every later turn for that agent waits forever. `nudge` queues behind the stuck turn; `resume_agent` refuses ("is running"). Possible triggers: a TUI menu or modal is open, an autocomplete menu eats the Enter, or the UserPromptSubmit hook missed its 1 s reply deadline and was only spooled. |
| F2 | **Live agents in a non-Idle state get nothing.** `StopFailure` (API error) sets `Error` while the process keeps running. `rate_limit` sets `BlockedByLimit`, and nothing moves it back. A Stop with background tasks sets `Working`. | `state.rs:203-226`, `runtime.rs:2214` | Messages are queued into a turn that is never pasted, and they are then excluded from tool-call injection as "already carried" (`team.rs:984`). |
| F3 | **A crashed agent (`Error`/`Killed`, process gone) is never restarted.** The pump spawns only `Suspended` or never-started agents. | `runtime.rs:2167`, `team.rs:2334` | Message turns, nudges and the Lead's own `resume_agent` "Continue" turn sit queued forever. |
| F4 | **A late Stop reply leaves the tracker in `Working`.** The core returns a Stop *block* (FR-403) before it applies the state change. If `orch-hook` misses its 1 s deadline, the CLI ends the turn but the core thinks the turn goes on. A lost hook event is spooled and replayed only at app boot. | `runtime.rs:1257`, `orch-hook/src/lib.rs:59`, `reconcile.rs:117` | The agent shows `Working` forever. The only alert reaches the user after 15 min, and the Lead is never told. |
| F5 | **Head-of-line blocking.** The pump looks only at an agent's first queued turn. A turn waiting on the Governor (off-hours, the 5 h guard, the slot cap, a model switch) holds back everything behind it, including the user's answers, the spec approval and PR results. | `runtime.rs:2071-2082` | The user answers, and nothing happens. |
| F6 | **Slot-cap deadlock.** Agents wedged by F1–F5 hold live slots, so a suspended member that has a message never gets one. | `runtime.rs:2182`, `resources.rs:177-206` | The whole team stalls behind one wedged agent. |
| F7 | **Read without being seen.** Messages are cut at 1,500 chars on injection and 8,000 in a turn, ending in "(more: call inbox)", but every id is marked read at the end of the turn. | `team.rs:961-1041`, `runtime.rs:1370` | If the model doesn't call `inbox`, the rest is lost. A single long message can arrive as "(more…)" alone. |
| F8 | **Injection gaps.** Messages are marked delivered before the hook reply is written. A subagent's PostToolUse also injects, so the text can land in a *subagent's* context. The list of injected messages lives only in memory, so a core restart leaves them delivered and never read, and they are never sent again. | `team.rs:1016`, `runtime.rs:612` | Silent loss. |
| F9 | **One thread for every run.** `team_tick` (delivery, dispatch, supervise) has no panic guard and makes backend network calls (GitHub/Linear) inline. | `runtime.rs:1983-1995`, `team.rs:526-713` | One panic stops delivery for every team until the app restarts, and a slow call delays it. |
| F10 | **Blind spots in stuck detection.** The Lead itself is never checked. Waits on another agent are never counted as stuck, so a member waiting on the Lead while the Lead waits on the member goes unseen. The alerts travel over the same failing channel. | `supervise.rs:42-137` | Nobody notices. |

The test double `tests/fake_claude.sh` always acknowledges a paste, never loses an Enter, and never shows a modal, a StopFailure, background tasks or a slow hook. None of F1–F9 is tested.

**This matches "it gets stuck until I restart the app"**: the boot reconciler turns every wedged agent back to `Suspended` and re-queues unacknowledged turns.

## 3. What the industry says

- **Anthropic, "How we built our multi-agent research system" (June 2025).** An orchestrator–worker design: a lead agent fans out subagents in parallel, each with its own context window, and they return condensed findings. It works best for breadth-first, parallelizable work. Domains "that require all agents to share the same context or involve many dependencies between agents" are named as a poor fit today, and **most coding tasks** are given as the example. The post also reports that multi-agent runs use about 15× the tokens of a chat, and that a synchronous lead becomes the bottleneck.
- **Anthropic, "Building effective agents" (Dec 2024).** Start with the simplest pattern that works. Orchestrator–workers suits tasks whose subtasks can't be predicted in advance. Every added agent adds cost and failure surface.
- **Cognition, "Don't Build Multi-Agents" (June 2025).** Parallel agents that don't share full context make conflicting implicit decisions. The advice is a single-threaded agent with context compression, or workers kept to narrow, read-only questions. This argues against *peer-to-peer* teams more than against a star.
- **Berkeley, "Why Do Multi-Agent LLM Systems Fail?" (MAST, 2025).** Across 7 frameworks, failures fall into three groups of roughly equal weight: specification and system design, **inter-agent misalignment** (ignored input, withheld information, conversation resets), and **verification and termination**. Most failures are coordination and protocol failures, not model failures. Structured protocols and explicit verification help most.
- **Claude Code itself.** It ships both shapes:
  - **Subagents** (the Agent tool): one context window each, results reported only to the caller, no nesting (a subagent can't start subagents), optionally in their own git worktree, async by default.
  - **Agent teams** (experimental since early 2026): separate Claude Code sessions with a shared task list and mailbox, which can message each other.

  Its own docs say to use subagents for focused workers that only report back, and teams when workers need to talk to each other. The docs also list known limits for teams that match ours: task status can lag (a teammate doesn't mark its task done and blocks the dependent ones), and teammates can't be resumed. *To check on the pinned version before relying on any detail.*
- **Other frameworks.**
  - **LangGraph:** its 2025 benchmark of supervisor vs swarm found that the supervisor loses information by "translating" between agents, and its fix was to forward worker outputs directly.
  - **OpenAI Agents SDK:** offers "agents as tools" (fan-out) and "handoffs" (transfer of control).
  - **MetaGPT:** a shared publish/subscribe pool and structured documents instead of free messages.
  - **Google A2A:** tasks with an explicit lifecycle and push notifications.

  The common lesson: **coordination needs a durable protocol with explicit states and acknowledgements.** Messaging middleware and the actor model learned the same decades ago: a durable mailbox, at-least-once delivery, an acknowledgement *by the consumer* (not the transport), and a supervisor that restarts failed workers.

Summary: one long-lived session per role is a recognized pattern (Claude Code agent teams, MetaGPT, CrewAI roles). It costs more and is harder to coordinate than fan-out. Fan-out is the industry default when workers only need to report back. **Neither shape fixes a delivery layer that has no acknowledgement and no retry**, and ours has neither.

## 4. The options

- **A — Keep one session per agent, make delivery reliable.** A durable mailbox with an acknowledged hand-over, retry and a watchdog. Recovery for every wedged state, a supervisor that also watches the Lead, and a test double that misbehaves.
- **B — One Lead session that fans out subagents** (the Agent tool with `isolation: worktree`, subagent definitions generated from the roles).
- **C — Claude Code's native agent teams** instead of our own relay.
- **D — Hybrid.** A now. B as a later, per-team "fan-out" mode for small features. C watched until it leaves experimental.

### Pros and cons

| Criterion | A — one session per agent, fixed | B — Lead + subagents in one session | C — native agent teams |
|---|---|---|---|
| **Message loss** | Fixed at the cause: acknowledgement, retry, recovery (spec §3). The relay stays ours, so it stays our risk. | **Gone by construction** for Lead↔worker: a subagent's result is the return value of a tool call. | Claude Code's problem rather than ours, but its docs already list "task status can lag" and no resume. |
| **Peer messages** (front↔back, ux→ui handoff, front asks ui for a review) | Kept. | **Lost**: subagents only report to the caller. Every exchange goes through the Lead (the "telephone game" LangGraph measured), and the Lead's context grows with it. | Kept. |
| **Self-review (ADR-010)** | Kept: the owner runs `/code-review` (which starts its own subagents) and `/simplify`. | **Breaks**: a subagent can't start subagents, so `/code-review` can't run inside a worker. The Lead would review every diff itself, which is exactly the context blow-up ADR-010 removed. | Kept, if teammates can run slash commands (to check). |
| **Plan first, spec gate, ask_user, permission parking** | Kept. All of these are built on per-agent turns and hooks. | Mostly rebuilt. Background subagents can't ask the user, and permissions they lack are denied (to check). All prompts surface in the Lead's single pane. | Rebuilt against an experimental API. |
| **User talks to a specialist (FR-409)** | Kept: every agent has its own pane. | **Lost**: subagents have no pane. The user sees only the Lead. | Partly (split panes in tmux/iTerm). |
| **Per-role config** (model, tools, MCP servers, sandbox, permission profile) | Kept, one compiled config per agent (ADR-002 §6.3). | Model, tools and prompt can be set per subagent. **Sandbox, permission profile and hooks are the Lead's process for everyone.** MCP servers are shared too, so every design server's tools are loaded in the Lead. | Set when a teammate is spawned, with little control. |
| **Memory and live slots** | 190–255 MB per agent, a 3-slot cap, plus a slot-cap deadlock to manage (F6). | **One process**: no slot cap, no suspend/resume per member. A big simplification. | A process per teammate, as in A. |
| **Usage pacing (Governor)** | Per agent, per turn, as today. | Still possible (subagent tool calls pass PreToolUse with an `agent_id`), but admission and suspend become all-or-nothing for the whole team. | Out of our hands. |
| **Lead context** | Small: one report per task. | Grows with every subagent summary, review and back-and-forth. A long feature compacts the Lead, which is the one context that must not forget. | As A. |
| **Failure blast radius** | One agent. The core restarts it with its state in SQLite. | **The Lead is a single point of failure**: a crash, a plan limit or a compaction stops or degrades every in-flight worker, and async subagents don't survive a Lead restart (to check). | As A, but recovery is Claude Code's. |
| **Long tasks and continuity** | Each member keeps its session for its task (T-40 rotates per task). A follow-up ("fix the review findings") lands in a warm context. | A subagent ends when it returns. A follow-up needs a continuation by agent id within the same Lead session, or starts cold. | As A. |
| **Visibility and audit** | Every agent has a transcript, events, a timeline and a run report. | Subagent transcripts exist (SubagentStart/Stop give their path), but the per-agent UI (panes, status, mission control) needs a new model. | Little visibility for the app. |
| **Fit with the industry** | The role-team pattern. The guidance is to keep the topology a star and the protocol explicit. | Anthropic's default orchestrator–worker. Its own caveat: coding tasks with many dependencies are a poor fit. | Anthropic's own take on our design. |
| **Cost to build** | **Medium**, in code we own and test (spec §3). | **Very high**: a rewrite of the team runtime, the review pipeline, the attention model and the UI. | High, and tied to an experimental feature that our hooks contract (R2) doesn't cover. |
| **Token cost** | Baseline. | Somewhat lower: one system prompt and CLAUDE.md load, no per-member warm-up. Workers still start cold per task. | ≈ A. |

## 5. Recommendation (subject to the spec gate)

**D: do A in this feature, record B as a possible later mode, and watch C.**

1. **The losses are bugs in the hand-over, not in the architecture.** All ten failures above can be fixed where they sit. B would remove F1–F8 for Lead↔worker traffic only by also removing peer messages, self-review, per-role sandboxes, per-member panes and per-member recovery. Those are what the last four features built.
2. **The industry reading favours a star and a protocol, not a single process.** The stated reasons multi-agent systems fail are coordination and verification (MAST). Anthropic's own caution is about coding tasks with many dependencies. The cure is explicit states, acknowledgements and a supervisor, which is what A adds.
3. **B is worth having later, as a team mode, not as a replacement.** For a small feature (one backend task and one frontend task, no design, no peer talk), a Lead that fans out subagents with worktree isolation is cheaper and can't lose a message. As a follow-up feature it would be a team setting `mode: sessions | fanout`, chosen at launch. It is not in this feature's scope.
4. **C is the same idea as ours, built by Anthropic, and still experimental**, with the same lag limits listed. Re-evaluate it when it leaves experimental and the R2 contract tests can cover it.

**Decided (user, 2026-11-02): "We keep A."** One session per agent stays. This feature makes delivery reliable (spec §3). B and C are not in scope; B stays recorded as a possible later team mode.
