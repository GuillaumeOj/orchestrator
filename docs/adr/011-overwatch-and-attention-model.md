# ADR-011 — The Overwatch, one attention model, and decisions as cards

- Status: Proposed · Date: 2026-10-28 · Amends ADR-009 (who answers permission prompts)

## Context
The app was built so the user drives many agents from one place. A day of real use (team run `p3-ecosystem`, 2026-10-27, forensics in [the brief](../brief/2026-10-28-overwatch.md)) shows the user still opens agent panes to keep work moving: 6 typed interventions in the Lead's pane, 29 permission prompts across 5 agents, and about 5 hours of agent time spent waiting on something nobody could see from outside.

Three things cause it:
1. **Defects** in the runtime: the app's own message file triggers a Read prompt on every long task assignment; a prompt that outlives the Inbox hold becomes pane-only; an agent suspended for memory pressure before its first turn can never be resumed; the memory cap flips with every pressure reading. These are fixed directly (SPEC P4.0) and are not this ADR's subject.
2. **No shared "now"**: about eight UI components each derive agent state themselves and disagree (four different "needs you" counts, three refresh rates, one agent shown "Admitted" in one place and "queued 3rd" in another). Nothing models *what* an agent waits on.
3. **The Inbox is three strings**: the core flattens every item to title, detail and preview and drops the payload of most kinds. Decisions vanish without an outcome; the digest lists the prompts sent to agents, not what they did. The user acts on every permission prompt and every question personally, so the Inbox is where all the friction lands.

The user's stated goal: **one interface, a mission-control view of everything, and never having to dive into a sub-session**. Agent panes become an escape hatch.

## Decision

### 1. One attention model, computed in the core
The core computes, for every agent and every team run, one **attention record** and pushes it to the UI on change (no per-pane polling). Every surface renders it; the UI derives no state of its own.

```
Attention {
  level:      needs_you | running | waiting | queued | idle | done | error
  waiting_on: { kind: user | agent | task | governor | limit | startup | external,
                ref?,           // decision id, agent id, task id, queue entry…
                since }         // when this wait started (not when the agent was created)
  reason:     short text        // "permission: WebFetch example.net", "T-4 blocked by T-2"
  activity?:  current tool or "writing", for running
}
Run attention = phase, members' records, blockers (first waiting_on that is not internal)
```
`needs_you` exists only when `waiting_on.kind = user`, so "needs you" has one definition.

### 2. Decisions are typed objects with a lifecycle
The Inbox item becomes a **Decision**: a typed subject (agent, role, project, feature, run, task), typed **context blocks** (tool call with its input and cwd; rendered markdown; diff summary with line counts; options; PTY tail as a last resort; cost), actions, and a lifecycle:

```
open → decided (by: user | policy | overwatch | pane) → sent → effect (resumed | PR opened <url> | failed: …)
     ↘ expired → parked (agent's turn ended, slot freed; a later decision resumes it)
     ↘ stale   (its context changed)
```
Decisions are threaded by run (or by agent for solo sessions), so a follow-up question sits under the earlier answer. The digest (level 4) lists **outcomes**: task done, merge done, PR opened, run phase changed.

### 3. Mission control replaces the Overview as home, and the Inbox pane goes away
- **Mission control** (centre): lanes *Needs you · Running · Waiting on · Queued · Done today*, team runs as rows with a phase stepper and their members, solo sessions alone. Every row says what it waits on and since when.
- **Overwatch chat** (right column, always visible): the conversation with the Overwatch, and where decisions appear as **cards**. The Inbox pane is removed once the cards carry everything it did.
- **Agent panes** (the real Claude Code TUI) open on demand from any row or card, as a tab over the workspace. Nothing requires opening one.

### 4. The Overwatch: one global, event-driven agent
A Claude Code session with a fixed role, not attached to any project's worktree:
- **Event-driven**: suspended between events. The core wakes it (resume, normally within the prompt-cache hour) with a batch of new events: a decision it may settle, a stuck agent, the user's message. Blocking events wake it at once; others are batched (default 30 s). It has its own slot, like a Lead (FR-203), and runs at the critical tier. Default model: Sonnet.
- **Session lifetime**: one session per working day. Each morning a new session starts with a briefing the core writes (the attention snapshot, open decisions, the week's plan); this keeps its context small and its resumes cheap.
- **Tools** (orchestrator MCP, Overwatch role only): `world()` (the attention snapshot), `inspect(agent)` (task, last messages, transcript tail), `decide_permission(decision, allow | deny, reason)`, `answer_question(decision, answer, sources)`, `annotate(decision, note)` (its recommendation on a card it leaves to the user), `nudge(agent, text)`, `resume_agent`, `restart_fresh(agent)`, `reprioritize(queue_entry, tier)` within the weekly plan, `propose_rule(pattern, effect, scope)`, `post(text)`. Read-only access to registered projects (to read specs, `decisions.md`, briefs); no Edit, Write or Bash.
- **May settle on its own**: permission prompts within the policy (§5); agents' questions when the answer is grounded in the feature's docs, `decisions.md` or an earlier answer of the user's (it cites the source); unsticking (nudge, resume, restart fresh) and reordering the queue inside the plan.
- **Never**: start work (sessions, runs, tasks from intake), approve spec or PR gates, change the weekly plan or Pause all, allow anything on the hard-deny list. It **escalates** product, scope, money, security and irreversible calls, and anything it is unsure of, by annotating the card with its recommendation.
- **The core posts first**: a decision's card appears in the chat at once, without the LLM. When the Overwatch settles it, the card collapses into one line ("allowed front-1: `pnpm add zod` · policy: installs"), expandable, with **undo** where it still means something (revoke a rule, tell the agent the answer changed).

### 5. Permission policy: written, then learned
1. The **hard-deny list** (core, not editable by the Overwatch): writes outside the agent's worktree, credential paths, push, merge into protected branches, PR creation (already core-only tools), and any call an *ask* rule sends to the user.
2. The **policy file**, edited by the user: global and per project (`.orchestrator/policy.toml`, or app data when `.orchestrator/` is not tracked), rules over tool + input pattern + role, effect allow / deny / overwatch / user. The store's permission rules (FR-708) become entries of this policy.
3. **Learned rules**: when the user gives the same answer to the same kind of call 3 times, the Overwatch proposes a rule as a card; one click adds it to the policy file.
4. Unmatched calls go to the Overwatch when it has the `overwatch` effect (the default for team roles), else to the user.

This amends ADR-009: permission prompts were the user's only. The sandbox, the credential rules and the core-only push/merge/PR tools stay exactly as ADR-009 decides; only the answer to a prompt inside those bounds can now come from a policy rule or the Overwatch, and every such answer is audited with who decided (FR-709).

### 6. Expired holds park the agent
A permission hold that reaches its deadline no longer falls back to the pane: the hook denies with "ORCH: awaiting the user's approval", the turn ends, the agent may be suspended and its slot is freed. The card stays open with the call and its input. On Allow, the core adds a one-shot allow for that exact call and resumes the agent with "approved: retry the call"; on Deny, it delivers the reason. (This is the long-wait path FR-705 already describes, made the default on expiry.)

## Alternatives considered
- **No LLM, deterministic rules only.** Covers permissions and simple unsticking, but not answering questions from the docs or explaining a stuck run; the user wanted one interlocutor.
- **Always-live Overwatch.** Faster reactions, but a permanent ~250 MB on an 8 GB machine and a context that grows all day. Event-driven with a warm prompt cache is close enough in latency.
- **The Overwatch triages before anything reaches the user.** Fewer cards, but every decision then waits for a resume, and a failure of the Overwatch hides decisions. The core posting first keeps the app usable with the Overwatch down.
- **The Lead as the interlocutor.** Already tried (FR-411): it only covers one team and still left permission prompts and stuck starts to the user.
- **Keeping the Inbox pane next to the chat.** Two places for the same decision; the user chose one.

## Consequences
- The UI loses the Inbox and Overview panes and gains Mission control, the chat column and card components (Penpot pass first, ADR-008).
- The core gains: the attention model and a push channel; Decision objects with context blocks and lifecycle (migration of `inbox_items`); the chat log; the policy file and its loader; the Overwatch role, its tools and its wake scheduler.
- One more Claude session in the budget: its usage is attributed to "overwatch" in the weekly plan (critical tier, drawn from the reserve).
- Risks: wrong auto-answers (mitigated by the hard-deny list, citations, undo, and audit); wake latency on a cold cache (a permission hold waits up to the Overwatch's deadline, default 60 s, then the card stays for the user); the Overwatch itself getting stuck (it has no permission prompts of its own: read-only tools and orchestrator tools only; a failed wake leaves cards for the user).
- To measure in P4.0 before tuning the policy: how many prompts remain with auto mode actually on.
