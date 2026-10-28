# Brief: from many panes to one Overwatch

> 2026-10-28 · Brainstorm with Claude on the state of the app after the first real team run. Decisions are in [ADR-011](../adr/011-overwatch-and-attention-model.md); requirements and phasing in [SPEC v0.12](../SPEC.md) (§4.13, §9, §11 P4).

## Why we're changing course

The app was meant to make many Claude Code sessions on many projects manageable from one place, paced against the laptop's memory and the Max plan's limits:
- monitor every running session;
- queue work to save resources and plan limits;
- specialized agents for each kind of task;
- plan from GitHub or Linear issues;
- a dedicated team that writes product specs;
- and, in the end, **one interface instead of talking to each sub-session**.

Most of that is built. Day-to-day use still hurts in four places:
1. The Inbox is raw text.
2. Stuck agents have to be unlocked from inside their own pane.
3. There is no clear view of what is running and what isn't.
4. The UI doesn't explain what is really going on.

## What we found

Three read-only investigations: the stored data and transcripts of the team run `p3-ecosystem` (2026-10-27), the Inbox code path, and the UI's information architecture.

### Most pane dives came from defects, not from missing UI

You typed into the Lead's pane 6 times during the run.

| Cause | Agent time lost |
|---|---|
| **29 permission prompts** across 5 agents. 8 of them came from the app itself: turns over 900 characters are written to `agents/<id>/messages/*.md`, and agents have no Read allow on that folder. **Every long task assignment prompted.** | ~146 min |
| Prompts that outlived the 538 s Inbox hold. The Inbox item then lost the tool and its input and could only say "waiting for you in its pane". | ~121 min (part of the above) |
| Two agents (lead-1, front-1) suspended for memory pressure 1 s after their task was pasted, before any transcript existed. Every Resume exited with status 1; the only way out was to delete and relaunch. | ~150 min |
| Memory pressure flipped warn↔normal about 50 times with no hysteresis. Each flip released one agent, and the next suspended another. | hours of queue lag |
| The Lead waited for the slot it held itself, and ended a turn on a plain-text question. | ~50 min, **fixed in v0.11** |
| A Penpot browser tab went to sleep. The `ask_user` question reached the Inbox correctly. | 44 min, worked as designed |

The run used `acceptEdits`, so v0.11's auto mode was never live. We don't yet know how many prompts the classifier absorbs.

### No shared "now"
- About eight components each work out agent state on their own, and they disagree. "Needs you" is counted four ways (sidebar, icon rail, Overview, title badge), and one waiting agent shows "Admitted" in the Inspector and "queued 3rd" in its pane.
- Refresh rates of 1 s, 4–5 s and 15 s let panes contradict each other.
- **"Waiting on what" isn't modelled.** Waits on another agent, task dependencies and Governor holds live in three places, or nowhere.
- No view shows finished work. Team state is spread across the Feature pane, sidebar, Board and Inbox.
- Many numbers have no labels: `(+2)`, `±31`, `0.0 pp`, a bare `7%`.

### The Inbox throws its structure away
- The core flattens every item to three strings (`crates/orch-app/src/inbox.rs`) and keeps the payload only for gates and questions.
- Permission items lose the tool input and cwd. PR gates show file names although the diff and line counts are already computed. Questions are unrendered markdown, and multi-question `AskUserQuestion` options are dropped.
- Decisions vanish. The toast says "Reply sent" before anything is sent, and the outcome (agent resumed, PR URL, failure) never reaches you.
- The "finished a turn" digest shows the prompt sent *to* the agent, not what it did, and can't be dismissed.
- Nothing is grouped by run or threaded, so a follow-up question looks unrelated to the earlier one.

## Decisions

| Topic | Decision |
|---|---|
| Home screen | **Mission control**: Needs you · Running · Waiting on · Queued · Done today, with team runs as rows that show their phase |
| Main layout | **Board + chat side by side**: Mission control in the centre, the Overwatch chat always visible on the right |
| Who you talk to | **A global Overwatch agent** that sees every project and run |
| Agent panes | **Escape hatch**: open on demand, never required |
| The Inbox | **Folded into the Overwatch chat** as cards; the Inbox pane goes away |
| How cards arrive | **The core posts at once** (no LLM needed); the Overwatch settles what it may and annotates the rest |
| Overwatch authority | **Permission prompts** (within policy), **agents' questions** (grounded in the docs, cited), **unstick and reprioritize**. Not: starting work, spec/PR gates, the weekly plan |
| Overwatch runtime | **Event-driven**: suspended between events, its own slot, Sonnet, one session per working day |
| Its decisions | **Collapsed lines in the chat**, expandable, with undo where it still means something |
| Permission policy | **Written + learned**: a policy file you edit, plus rules proposed from your repeated answers. A hard-deny list it can never override |
| Hold expiry | **Park the agent**: end its turn, free the slot, resume on your answer. Never pane-only |
| Intake | **Per project**: GitHub, Linear, docs in the repo, or none yet. Starting work stays yours |
| Order | **Fix the defects first**, measure a real team run in auto mode, then redesign |

## What a day looks like afterwards

At 09:00 you open the app on Mission control. The Overwatch has posted its morning line: "2 runs in progress, 1 decision for you, week at 41 % against a 38 % plan."

The single card on the right is checkout-v2's spec gate. It shows a rendered summary, 3 docs, +142 lines, and the Overwatch's take: "scope matches the brief; one open question on refunds". You read it and approve. The card collapses to "approved · lead resumed".

During the morning, the Overwatch's collapsed lines pile up:
- "allowed back-1: `cargo add serde_json` · policy: installs"
- "answered front-1 (API pagination) · from api-contract.md §3"
- "restarted front-2: its first session never started"

Each line can be expanded. One card asks you: "ui-1 wants to fetch fonts.googleapis.com, outside the allow-list. I'd allow it for this project." You click *Always for this project*, and the policy file gains a rule.

You never opened a pane. When you want to watch an agent work, you click its row, its Claude Code session opens over the board, and Esc brings you back.

## Phases

| | Content | Exit |
|---|---|---|
| **P4.0 Stop the bleeding** | Read allow on the messages folder; park expired holds; restart sessions that never started as fresh sessions; hysteresis on the memory cap; never pressure-suspend an agent whose turn is pasted; audit who answered each prompt; record "pane interventions" | A real team run in auto mode, with prompts and interventions counted against this run's baseline (29 prompts, 6 interventions) |
| **P4.1 One "now"** | Attention records computed in the core and pushed to the UI; typed `waiting_on`; Decisions keep their full payload; the digest shows outcomes | Every surface shows the same counts |
| **P4.2 Mission control** | Penpot pass, then the board replaces the Overview | "What's going on" answered in one screen |
| **P4.3 Chat + cards** | Penpot pass, then the chat column, decision cards with context, the lifecycle, threading. The Inbox pane is removed | Every Inbox interaction available as a card |
| **P4.4 The Overwatch** | Role, tools, wake scheduler, policy file and learned rules, answers with sources | A team run where the user only uses the chat and the board |
| **P4.5 Intake** | Per-project source and a "Ready to start" lane with cost estimates | — |

## Settled after review (2026-10-28)
- **Wake latency.** Target under 20 s with a warm cache. The Overwatch has 60 s on a permission before the card waits for you.
- **Learned-rule threshold.** 3 identical answers, then a proposal. Tune it after P4.4.
- **Chat retention.** One thread per working day; older days are searchable.
- **Many projects.** Mission control has a per-project filter from day one (P4.2).
- **Counting pane interventions.** The user typing into an agent that is waiting, or idle with an open task, is recorded as a `pane_intervention` event (P4.0). The count is the baseline for reducing interventions later.
