# Improve Plan Usage · Feature spec

> Feature `improve-plan-usage` · Integration branch `orch/improve-plan-usage/integration` · core (Rust) + UI, no design pass

## 1. Your question, answered

**Are we already running coding agents on a cheaper model?** No.

- The starter roles (`crates/orch-team/src/presets.rs:115`) set a model only on the **Lead** (`opus`, effort `high`). Backend, Frontend, UI, UX and Reviewer have **no model**, so `--model` is not passed (`crates/agent-runtime/src/compile.rs:264`) and Claude Code uses the account's default, which is Opus on your plan. That is why every agent runs Opus.
- What exists today is only a **safety net**, not a policy: the opt-in per-role *downgrade* (FR-819) switches a role to another model at a task boundary while the week (or the Opus window) is under pressure. It is off by default, and no starter role turns it on.
- The model is already configurable per role and per node (free-text `model` field in the Team canvas), so you can change it by hand today, but nothing guides you there.

**Is lowering the model for coding agents a good idea?** Yes, with two guards.

- Our coding tasks are narrow and well specified: the Lead (on Opus) writes the spec and the API contract, splits the work, and reviews every report before merging. Each specialist then implements a scoped task, with tests, and runs `/code-review` + `/simplify` on its own branch. That is the kind of work Sonnet does well, at a much lower plan cost per token than Opus. Where the plan has a separate weekly Opus limit (the app already tracks `seven_day_opus`), moving the specialists off Opus also stops them from eating the Lead's Opus budget.
- **Guard 1, keep the judgement on Opus.** The Lead (planning, decisions, merges) and the Reviewer (the quality gate) stay on Opus. Weaker judgement there costs more in rework than it saves.
- **Guard 2, an escape hatch.** Some tasks are hard (a tricky concurrency fix, a cross-crate refactor). The Lead must be able to run *one task* on Opus without changing the role. Otherwise a struggling Sonnet agent loops through review rounds and burns the savings.
- **Designers are a separate case (your follow-up on Penpot).** Sonnet can drive Penpot: the Penpot MCP is mostly `execute_code` (JavaScript against the Penpot API) plus exports, which is code and tool use, where Sonnet is strong. What differs is where the value lies:
  - **UX** writes flows and the UX brief (text, some wireframes). That is close to spec writing at a smaller scale: **Sonnet**.
  - **UI** turns the brief into finished screens in Penpot. Its output is visual judgement (hierarchy, spacing, consistency with the design system) that frontend then copies pixel for pixel, and its sessions are long and tool-heavy: a niche API, retries, screenshots and exports read back as images. A weaker pass costs a second design round *and* a frontend redo, which is where the expensive tokens are. UI also runs far less often than the coding agents (only on features with screens to design), so moving it off Opus saves little. **UI stays on Opus.**
  - The per-task model (§2.2) works both ways: the Lead can run a mechanical UI task (re-export frames, rename layers, apply a token change) on `sonnet`.
- Haiku is not a good default for coding agents here: multi-file Rust + Svelte work with a self-review loop is beyond what it does reliably. It stays available by hand.
- How we will know it worked: the Report pane already splits usage **by model** (`ui/src/panes/ReportPane.svelte`, tab "model"), and the Governor learns each model family's cost per percent of the week (`crates/governor/src/estimator.rs`). After a feature or two on the new defaults, compare the Opus and Sonnet shares there.

## 2. Scope

### 2.1 New starter defaults (core)

| Starter role | Model today | Model after | Effort |
|---|---|---|---|
| Lead | opus | opus (unchanged) | high (unchanged) |
| Reviewer | default (Opus) | **opus** (made explicit) | default |
| Backend | default (Opus) | **sonnet** | default |
| Frontend | default (Opus) | **sonnet** | default |
| UI | default (Opus) | **opus** (made explicit: Penpot design work, see §1) | default |
| UX | default (Opus) | **sonnet** | default |

- Bump `STARTER_VERSION` to 4, so **unmodified** installed copies of the starter roles upgrade in place (FR-306). A role you changed yourself is never touched.
- A node override (`overrides.model`) still wins over its role, as today.
- Aliases (`opus`, `sonnet`, `haiku`, `fable`) are passed as is to `--model`; Claude Code resolves them to the latest model of the family. No pinned model ids.

### 2.2 Per-task model (core + lead protocol)

- `create_task` and `update_task` take an optional `model` (an alias or a model id; `""` clears it). It is stored in the task's workflow state (`TaskFlow.model`, never synced to a task backend).
- Effective model of a task's turns, highest first: **task model → downgrade in force (FR-819) → node override → role model → Claude Code default**. An explicit task model is the Lead's deliberate choice, so it wins over a downgrade; the Governor still holds turns of a model whose own window is Red (FR-815), as today.
- It applies at the task boundary through the existing switch (`Runtime::task_boundary`, `crates/agent-runtime/src/pacing.rs:56`): an idle agent on another model is suspended and resumed with `--model`. Never mid-turn. The next task without a model goes back to the role's model the same way.
- The switch is recorded as today (`model_switch` event, with a `why` of `task`), so it shows on the timeline.
- Lead protocol (`crates/orch-team/src/compile.rs`, lead rules): one added line: specialists run a lighter model; pass `model: "opus"` on `create_task` (or `update_task` before it starts) only for a task that is hard or that already failed a review round on the default model, and `model: "sonnet"` for a mechanical task on an Opus node (e.g. re-exporting UI frames).

### 2.3 UI

- **Role inspector (Team canvas):** the free-text `model` field becomes a select: `default (Claude Code's)`, `opus`, `sonnet`, `haiku`, `fable`, and `custom…` which reveals the text input for a model id. An unknown saved value shows as custom with its text. Same control for the node override.
- **Task cards / task detail:** when a task has a model, show it as a small mono chip (`opus`) next to the plan-first marker. Read-only (the Lead sets it).
- The agent header already shows the running model (`Inspector.svelte:27`); no change.

## 3. Out of scope

- Automatic model choice from task size or text (the Lead decides; revisit once we have per-model data in the report).
- Changing effort levels of the specialists.
- Pinned model ids or a model catalogue fetched from the API.
- Changing the downgrade (FR-819) rules or its "never on Lead/Reviewer" warning.

## 4. Acceptance

1. A fresh install creates Backend/Frontend/UX with `model = "sonnet"`, UI and Reviewer with `"opus"`, Lead unchanged; an unmodified v3 copy is upgraded to v4; a user-modified copy is left as is (tests in `orch-team`).
2. A compiled specialist node passes `--model sonnet`; a node override still wins (existing compile tests extended).
3. `create_task` with `model: "opus"` stores it; `update_task` changes or clears it; the tool schemas describe it; an invalid value (empty after trim is "clear", anything with spaces or longer than 64 chars) is refused.
4. An idle Sonnet agent picking a task with `model: "opus"` is suspended and resumed with `--model opus`; its next task without a model goes back to `sonnet`; a task model wins over a downgrade in force (runtime tests next to the FR-819 ones in `pacing.rs`).
5. The role inspector select saves `null` for default, the alias for a known model, the typed id for custom; a stored unknown id renders as custom. The task card shows the model chip when set (UI tests).
6. `cargo test`, `cargo clippy`, `bun run test`, `bun run check`, `bun run lint` pass.

## 5. Tasks

| Task | Owner | Notes |
|---|---|---|
| T-1 Starter defaults v4 | back | `orch-team/src/presets.rs` (+ fingerprints of v3 for upgrade if needed), tests |
| T-2 Per-task model | back | `orch-store` `TaskFlow.model`, `orch-mcp` schemas, `agent-runtime` `create_task`/`update_task` + `task_boundary`, lead protocol line in `orch-team/src/compile.rs`, tests |
| T-3 Model select + task model chip | front | `TeamCanvasPane.svelte` role/node inspector, task card; depends on T-2 for the `flow.model` field; review by `ui` |

T-1 and T-2 can be one branch for `back` (same owner, sequential), T-3 after T-2. No UX/UI design pass: both controls reuse existing patterns (the effort select next to it, the plan-first marker); `ui` reviews the frontend change.
