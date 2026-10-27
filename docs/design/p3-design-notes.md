# P3 design notes: flows, states, copy and keys

UX pass for P3 (Ecosystem), 2026-10-27, by `ux` for `ui`. Sources: [spec](../../.orchestrator/features/p3-ecosystem/spec.md), [UX brief](../../.orchestrator/features/p3-ecosystem/ux-brief.md), [API contract](../../.orchestrator/features/p3-ecosystem/api-contract.md), Penpot pages 04, 05 and 06 as they are today, and the shipped key handlers in `ui/src`.

Everything here is a proposal until Guillaume marks the frames **Ready for dev**. No SPEC edits were made. The open decisions are in §0.

**Format.** The storyboards are frame-by-frame text notes; I did not draw on Penpot. `ui` draws the 04 frames first and puts the notes boards on page 06 over them, as was done for J3. The layout is suggested in §8.

---

## 0. Decisions to confirm and gaps found

| # | Finding | Proposal | Owner |
|---|---|---|---|
| D1 | The **Design sprint** starter is lead + ux + ui only, with no Frontend (`presets.rs`). It can't show "Frontend implements from the frames". | J12 runs on **Full-stack** (shop-api › onboarding). Design sprint stays design-only: ui hands off to lead. | lead |
| D2 | In Full-stack, `can_handoff(ui, front)` is **false**. The only link is `front → ui: reviews`, and a handoff needs a lead endpoint or a `hands_off_to` edge. | The P3 starter update (`starter_version`) adds `ui → front: hands off to` to Full-stack. | lead / back |
| D3 | The contract's Inbox `mcp_needs_auth` item has only `open` and `reject`. The brief asks for **Reconnect**. | Add a `reconnect` action (§5.15): opens a pane with `claude /mcp` for http/sse, the claude.ai connector settings for a connector, and Settings for a stdio server with a Keychain env ref. | lead → back |
| D4 | The queue reason kind `off_hours` already means "outside working hours" in P1 (`ui/src/lib/status.js`). The P3 contract reuses it with `starts_at` for "Run off-hours". | Use a distinct kind (`scheduled_off_hours`), or the rule "`starts_at` present = you scheduled it". The copy differs (§6.3). | lead → back |
| D5 | The brief's copy says "Off-hours · starts **19:00**", but in the scenario's plan working hours end at **18:00** (J8 · 1 "09–18", J8 · 3 "stops at 18:00"). | Use 18:00 everywhere. The time always comes from `starts_at`. | ui |
| D6 | The P1 page 05 already has an off-hours banner ("Off-hours work needs AC power."), and the brief proposes different copy. | Keep the P1 title and merge the brief's point into the message (§6.1). There is one banner, not two. | ui |
| D7 | Copy rule (05): "usage in pp, never tokens or dollars". FR-818 / the contract give a **$ estimate** per row. | The report shows pp first. $ is only a muted secondary column headed "≈ API $", with the tooltip "What these tokens would cost at API prices. Your plan is not billed this way." This is the only exception to the rule. | Guillaume |
| D8 | Switching a project from Local/GitHub to **Linear** when it already has tasks: the spec doesn't say what happens to them. | A confirm step (J13 · 4): "Create 7 open tasks as Linear issues in ENG" (default) or "Leave them out (they stay in the app, not synced)". Done tasks are never pushed. | lead / Guillaume |
| D9 | A link-only design handoff (a Penpot URL, no export) can't be read by Frontend (agents don't fetch Penpot, ADR-009 / spec §4). | The Design tile says so and offers **Ask ui to export** (sends a message from you to the ui agent). The ui/ux starter prompts say "always export". | lead (prompts), ui (tile) |
| D10 | New global banner "MCP needs authentication" is not in SPEC §9.3's banner list. | Add it in the follow-up SPEC edit; one banner per server, collapsed to "+n" like the others. | lead |
| D11 | Penpot 05 lists the Overview kill key as `k`, while SPEC §9.7 and the code use `⇧K`. This predates P3. | Fix 05 to `⇧+K` while ui is on that page. | ui |

---

## 1. Scenario

Same as P1/P2: **Wednesday 21 Oct 11:00**, Governor in Red (+9 pp), live cap 1, shop-api › checkout-v2 team running. Additions for P3:
- **shop-api › onboarding**, a Full-stack run (D1) whose feature is "First-run onboarding: three welcome screens before the cart". Agents: lead, ux-1, ui-1, front-2 and an idle back-2. checkout-v2 already has front-1 and back-1, so **the onboarding agents are `front-2` and `back-2` everywhere**.
- Registry server `penpot` (connector "claude.ai Penpot Perso"), used by ux and ui.
- Linear workspace "Acme", team **ENG · Engineering**, for J13.
- Plan working hours 09:00–18:00; off-hours window 18:00–08:00; off-hours allowance 10 %, live cap off-hours 1.
- Model window seen: `seven_day_opus` → **opus week 92 % · Red**.

---

## 2. J12 · Design → code (P3 exit criterion)

Eight frames at 1512. Each notes board has: step, title, key sequence, what happens, exit → next. The times run on Wed 21 Oct.

**J12 · 1 · Launch Full-stack on "onboarding" (10:05)**
- Frame: 04 Launch on feature · 1512 (P2), with the P3 additions: an **Off-hours** checkbox (unchecked) and a **Design tools** line.
- Keys: `⌘+K` "launch" · `tab` · `⌘+⏎`.
- Design tools line: "ux, ui use **Penpot** (claude.ai connector · Not seen yet)". Status glyph + label, never color only.
- If `penpot` is `needs_auth`, the line becomes a warning: "Penpot MCP needs authentication: ux and ui will fail its tools. **Open Settings** ⌘+, · Launch anyway ⌘+⏎". Launch stays possible: the user may fix it while the lead writes the spec.
- Exit: `⌘+⏎` → the lead starts (queued in Red; the dialog already says so, P2) → J12 · 2.

**J12 · 2 · Spec gate with design tasks (10:40)**
- Frame: 04 Inbox · 1512 (gate selected), same item as J3 · 3.
- Keys: `⌘+⇧+I` · `j/k` · `a`.
- The gate preview lists the tasks the lead will create, with their dependencies:
  1. "UX brief and flows" → ux
  2. "Welcome frames in Penpot, exported" → ui · *after 1*
  3. "Build the welcome screens from the frames" → front · *after 2*
- The 3rd task carries "reads `.orchestrator/features/onboarding/design/*.png`". This makes the design → code link visible before approval.
- Exit: `a` → J12 · 3.

**J12 · 3 · UX brief handed to UI (11:20)**
- Frame: 04 Feature · 1512 on onboarding, with the **Design** section in its *no designs yet* state (§5.6).
- Keys: `⌘+P` onboarding · `d`.
- Timeline: "11:20 ux-1 → ui-1: handoff doc · ux-brief.md".
- Design section: "No designs yet. ui-1 is on 'Welcome frames in Penpot'. Exported frames appear here when ui-1 hands them off."
- Exit: ui-1 starts (`depends_on` met) → J12 · 4.

**J12 · 4 · UI draws in Penpot through the MCP (13:10)**
- Frame: 04 Agent pane · 1512 for ui-1, with the Feature pane split beside it.
- Keys: `⌘+; w` / `⌘+1`.
- The agent pane shows `mcp__penpot__execute_code` and `export_shape` calls.
- The pane header's MCP chip reads "penpot ● Connected". It flips from "Not seen yet" on the first successful call.
- ui-1 then writes the exports to `.orchestrator/features/onboarding/design/welcome-1.png`, `-2.png` and `-3.png`, commits them, and calls `handoff(kind=design)` to front for each one, plus once with the Penpot board link.
- Design section: three tiles in the *not merged yet* state:
  - Preview from ui-1's branch, with the chip "On ui-1's branch · not merged". The previews load from `source: 'branch'`.
  - Each tile shows "ui-1 → front-2 · 13:12".
  - One Penpot link tile, "Penpot · Onboarding / Welcome · board".
- Exit: ui-1 reports done → the lead reviews and merges → J12 · 5.

**J12 · 5 · Lead merges the design branch (13:40)**
- Frame: Feature pane.
- Keys: none (watching); `tab` to the timeline.
- Timeline: "13:40 lead merged ui-1 into orch/onboarding/integration".
- Tiles drop the "not merged" chip; the previews now come from the integration branch.
- The front task card moves Ready → In progress when the Governor admits front-2. In Red it queues: "Queued: live-agent cap 1 (Red) · starts ≈ 14:20 · Run now ≈ +2 pp".
- No toast: the merge is a level 4 digest item, "lead merged ui-1 into onboarding".
- Exit → J12 · 6.

**J12 · 6 · Design section with previews (13:45)**
- Frame: **04 Feature · Design · 1512** (new 04 frame; §5.6 *previews*).
- Keys: `d` · `→ →` · `Space`.
- The grid shows three thumbnails, ≤ 480 px wide, lazy-loaded. Each has its title ("welcome-2"), from → to chips (ui-1 → front-2), the note ("Step 2: pick your store; primary CTA bottom-right"), the time and "Open ⏎". The Penpot tile is last.
- Exit: `Space` → J12 · 7.

**J12 · 7 · Large preview (13:46)**
- Frame: **04 Large preview overlay** (§5.7).
- Keys: `Space`/`esc` close · `←/→` previous/next · `0` fit · `1` 100 % · `⏎` open.
- Fit by default. Caption: "welcome-2.png · 1512 × 982 · 214 KB · integration branch".
- Exit: `esc` → J12 · 8.

**J12 · 8 · Frontend implements from the frames (14:20)**
- Frame: 04 Agent pane for front-2.
- Keys: `⌘+; w` when it asks; otherwise none.
- front-2 starts from the integration branch, so the files are in its worktree. Its first tool calls are `Read .orchestrator/features/onboarding/design/welcome-1.png` … `-3.png`; the handoff message listed the paths.
- After it reports done, the lead asks for the PR → the PR gate (J2 · 4 frame, same item). **"No link was copied by the user"** goes on the notes board as the exit criterion.
- Exit: PR opened.

**Failure branches.** These are noted on the J12 · 4 and J12 · 6 boards and not drawn as frames:
- **Penpot needs auth mid-run.** A failed `mcp__penpot__*` call with an auth error marks `penpot` as `needs_auth` and creates one Inbox alert and one banner (§6.1); the agent's tool call fails visibly in its pane. `a` Reconnect (D3) → the connector settings open in the browser → `t` Test in Settings → connected, and the Inbox item closes itself.
- **Link-only handoff** (D9): the tile reads "Penpot link only. front-2 can't open Penpot links: ask ui-1 to export the frame." with the action **Ask ui-1 to export** `x`.
- **Export too large** (> 5 MB): the tile has no preview; it shows "Too large to preview (7.4 MB · max 5 MB). Open ⏎ still works."
- **Handed off before commit:** "Not found on ui-1's branch or on the integration branch. ui-1 may not have committed it yet."

---

## 3. J13 · Connect an integration (Linear)

Five frames, Wed 21 Oct 15:00.

**J13 · 1 · Settings › Integrations (15:00)**
- Frame: **04 Settings · Integrations · 1512** (§5.1).
- Keys: `⌘+,` · `j` (Integrations) · `⏎` · `j j` (Linear) · `⏎`.
- Rows:
  - GitHub ● Connected "via gh · logged in as GuillaumeOj · checked 14:58"
  - Linear ○ Not configured
  - Notion ○ Not configured
  - Penpot ● Connected "claude.ai connector · seen 13:10"
  - Figma ○ Not configured
- `⏎` on Linear focuses its secret field. Placeholder: "Paste a Linear personal API key". Help line: "linear.app › Settings › Security & access › Personal API keys. Stored in the macOS Keychain; agents never see it."
- Exit: paste, `⌘+⏎` → J13 · 2.

**J13 · 2 · Stored and tested (15:01)**
- Same frame, Linear row.
- Keys: `⌘+⏎` · `t`.
- Sequence: the field clears at once and reads "Stored in Keychain · Replace · Remove" → the badge shows "◌ Checking…" → "● Connected · Acme · as Guillaume · checked 15:01".
- Toast: "Linear token stored in Keychain".
- Error variant (drawn as a state, not a frame): "✕ Error · Linear rejected the token (401). Paste a new key with Replace." The token stays stored until Replace or Remove: we never silently delete a secret.
- Exit: `⌘+K` "project settings" (or Project pane › Settings) → J13 · 3.

**J13 · 3 · Project settings › Task backend: Linear (15:02)**
- Frame: **04 Project settings · Linear · 1512** (§5.13).
- Keys: `⌘+K` "shop-api settings" · `j` to Task backend · `3` (Linear).
- Segmented control: Local · GitHub Issues · **Linear**.
- Team picker: "Loading teams…" (skeleton rows) → "ENG · Engineering", "OPS · Operations".
- Optional project picker: "Any project" (default) / "Checkout".
- Exit: `⏎` selects ENG → J13 · 4.

**J13 · 4 · Column mapping (15:03)**
- Same frame, mapping table filled.
- Keys: `j/k` rows · `⏎` change · `⌘+⏎` save.
- Table "Linear state → Board column": Backlog → Backlog, Todo → Ready, In Progress → In progress, In Review → Review, Done → Done, Canceled → Done. Each row shows "default" in muted text until changed.
- Footer: "Issues labelled `orch` in ENG sync both ways. When both sides changed, Linear wins and you get an Inbox note."
- `⌘+⏎` with 7 existing local tasks opens the D8 confirm: "Move shop-api's tasks to Linear? · Create 7 open tasks as issues in ENG ⏎ · Leave them out (they stay in the app, not synced) l · Cancel esc". 12 done tasks are not pushed.
- Exit: `⏎` → J13 · 5.

**J13 · 5 · Sync now and the board (15:04)**
- Frame: 04 Board · 1512 with the backend chip "Linear · ENG".
- Keys: `r` (sync).
- The Board header reads "Linear · ENG · synced 15:04". Each card shows its Linear id (ENG-142).
- The sync report sits in Project settings › Task backend ("Last sync 15:04: 7 pushed · 0 pulled · 0 conflicts") and as a toast: "Synced shop-api with Linear · 7 pushed".
- Remote-wins state on the same board: the card ENG-140 has a marker "↓ changed in Linear", plus the Inbox info item (§5.15).
- Exit: done.

---

## 4. J8 · 4 update: weekly report from the Inbox digest

The existing frame "J8 · 4 · Weekly report (Mon 26 Oct 09:00)" stays the end state. Two changes:

1. **Entry.** The notes board's key sequence becomes `⌘+⇧+I` · `j` to the 4 · Info digest · `o` open. The 4 · Info row reads "󰄬 Weekly report for week 43 is ready · 09:00", which is already in J8 · 1's digest; the copy is unchanged. `o` opens the Week planner in **report view** for that week. No new frame is needed: draw the selected digest row as an inset on the J8 · 4 notes board.
2. **The report body** follows §5.9:
   - Header: "Week 43 · Mon 19 Oct 09:00 → Mon 26 Oct 09:00 · 93 % at reset"
   - KPI tiles (kept)
   - **Plan vs actual** table per allocation, replacing "By project"
   - Breakdown tabs `1` Project · `2` Feature · `3` Role · `4` Model
   - State time bar (Green 61 h · Amber 23 h · Red 31 h · Catch-up 2 h of working + off-hours time)
   - Governor actions by kind
   - The "What to change for week 44" block (kept)
   - Past reports list on `R`

   The notes text becomes: "Step 4 · Weekly report (Mon 09:00) | ⌘+⇧+I · o | R reports · 1–4 breakdown · ⏎ plan week 44 · e markdown | At the reset a 'Weekly report ready' digest item opens the report: used at reset, plan vs actual per allocation, use per project, feature, role and model, time in each state, governor actions, and suggestions for the next plan (not applied). | ⏎ → back to step 1 for week 44".

---

## 5. Screens and components: states and copy

Status labels for integrations and MCP servers. Each is shown as glyph + label; the glyphs are Material Design names, and ui picks the codepoints in the FiraCode NF set and adds them to the README glyph table.

| Status | Label | Glyph (MD name) | Color token |
|---|---|---|---|
| connected | Connected | check-circle | `color.status.done` |
| needs_auth | Needs auth | key-alert | `color.status.waiting` |
| error | Error | close-circle (same as agent Error) | `color.status.error` |
| not_configured | Not configured | circle-outline | `color.text.muted` |
| checking | Checking… | progress-clock (static; no spinner, calm) | `color.text.secondary` |
| unknown | Not seen yet (connector) / Unknown | help-circle-outline | `color.text.muted` |

"Checked" times use 24 h + relative ("checked 14:58 · 2 min ago"); after 24 h: "checked Tue 14:58".

### 5.1 Settings pane (04)
- **Frame:** left nav, 200 px, with the sections Claude Code · Integrations · MCP servers · Terminal & editor · Pacing policy · Keyboard · Memory. The right side shows the content of the selected section. The pane header reads "Settings" with `⌘+,`. Sections not designed in P3 show their title and "Not designed yet. The current values are editable from ⌘+K."
- **States:**
  - *default* (Integrations selected)
  - *loading*: one skeleton row per integration, "Checking integrations…" in the header line. Each row resolves on its own; the pane is never blank.
  - *integration disconnected*: the nav item shows a status glyph + count ("Integrations ⚠ 1"), and the affected row is first.
- **Opening it:** `⌘+,`, ⌘K "Settings", "Open Settings" in banners and Inbox items. These open straight to the right section, with the row or server selected.

### 5.2 Integration row (02)
- **Layout:** icon · name · status badge · detail · checked-at · actions (Test `t` · Connect/Reconnect `⏎` · Remove `x`).
- **Copy per integration:**

| Integration | Detail when connected | Not configured | Connect / Reconnect does |
|---|---|---|---|
| GitHub | "via gh · logged in as {GuillaumeOj}" | "gh is not logged in" | opens a shell pane running `gh auth login` |
| Linear | "{Acme} · as {Guillaume}" | "Add a personal API key to use Linear as a task backend" | focuses the secret field |
| Notion | "{Acme workspace} · integration {Orchestrator}" | "Add an internal integration token to publish feature docs" | focuses the secret field |
| Penpot | "claude.ai connector · seen {13:10}" or "{server name} · {kind}" | "No Penpot server in the MCP registry" | → MCP servers, `penpot` selected |
| Figma | same pattern as Penpot | "No Figma server in the MCP registry" | → MCP servers |

- **States:**
  - the five statuses (+ unknown for connectors)
  - *focus*: row focus ring, actions show their key hints
  - *busy*: the Test button reads "Testing…" and is disabled, and the badge reads "Checking…"
- **Error detail copy:**
  - "401 from api.linear.app: the key was revoked or is wrong"
  - "Could not reach api.notion.com (offline?)"
  - "gh: not logged in to github.com"
- **Remove** asks first: "Remove the Linear key from the Keychain? shop-api uses Linear as its task backend; sync stops until you add a key. Remove ⏎ · Cancel esc". If nothing uses it: "Remove the Linear key from the Keychain?"

### 5.3 Secret field (02)
- **States:**
  - *empty*: placeholder "Paste a {Linear personal API key}", hint `⌘+⏎ save`
  - *typing*: the value is masked (•••• with the last 4 shown only while typing). Buttons: Save `⌘+⏎` · Cancel `esc`.
  - *stored*: "🔒 Stored in Keychain · Replace · Remove". No value and no length are ever shown back.
  - *error*: the field keeps focus, text "Linear rejected this key (401). Nothing was stored." For a rejected key we don't store it. For a later failure (key revoked) the stored state stays with the row's Error badge.
- On save, the field clears at once (the secret leaves the UI) before the test result comes back.
- No copy-to-clipboard, no reveal toggle, by design.

### 5.4 MCP registry (04)
- **Table columns:** Name · Kind (stdio / http / sse / claude.ai connector) · Design tool (Penpot / Figma / —) · Used by ("ux, ui in 2 teams", tooltip lists team › node) · Status · Scope (Global / "shop-api override").
- **Drawer (edit)**, by kind:
  - *stdio*: command, args, env (key = value; a value `keychain:{account}` shows a 🔒 chip "from Keychain: {account}")
  - *http/sse*: URL, with the note "OAuth tokens are kept by Claude Code, not by the app"
  - *connector*: connector name (must match the name in claude.ai)
  - Every kind also has: design tool select, "Override for this project" toggle (only when opened from a project), Test `t`, Save `⌘+⏎`, Delete `x`.
- **States:**
  - *empty*: "No MCP servers yet. Add one with n, or start a design preset: it adds Penpot." The table is hidden.
  - *list*
  - *edit*: the drawer is open, and the table row stays highlighted
  - *connector hint*: an inline info box in the drawer, "A claude.ai connector comes from your claude.ai login, not from this app. It can't be passed in --mcp-config, so roles that use it run without --strict-mcp-config: they also see your other claude.ai connectors. Its status reads 'Not seen yet' until an agent uses it."
  - *needs auth*: the status cell reads "Needs auth" with Reconnect in the row. The drawer shows how to fix it per kind:
    - http/sse: "Run claude /mcp and authenticate {name}." Button: Open a pane `⏎`.
    - connector: "Reconnect {connector} in claude.ai › Settings › Connectors." Button: Open in browser `⏎`.
    - stdio: "{name} exited or refused its credentials: {detail}."
- **Secret-in-env opt-in.** It shows when an env value starts with `keychain:`, as an unchecked checkbox with a warning block in the FR-110 tone:
  > **Pass this secret to the agent.** Agents using {penpot-local} get {FIGMA_TOKEN} in their MCP config. The agent can read this token and send it to the model, and a prompt injection could make it leak. Prefer an OAuth (http) server, whose tokens Claude Code keeps itself.
  > ☐ I understand, pass it to agents
  - Without the opt-in, Save works, but Test and launch fail with "{name} uses a Keychain secret in its env, but 'pass it to agents' is off."
- **Delete refused** (used by roles): "{penpot} is used by ux, ui (Full-stack, Design sprint). Remove it from those roles first." Each role name links to its inspector.

### 5.5 Role inspector: MCP picker and downgrade (02/04)
- **MCP servers picker:** a checkbox list over the registry. Each row has its name, status glyph + label, and kind. Connector rows get "connector · needs your claude.ai login".
- **States:**
  - *none selected*: "No MCP servers. The role only has Claude Code's built-in tools."
  - *selected with status dots* (glyph + label)
  - *connector warning with strict MCP*: inline warning under the list: "{penpot} is a claude.ai connector, so this role can't use strict MCP: it will also see your other claude.ai connectors. Strict MCP is turned off for this role." This is shown, not asked: the compile does it. The same text appears in `teams_view` issues.
  - *ref missing*: "{figma} is not in the MCP registry. Add it in Settings › MCP servers or remove it here."
- **Downgrade** line: toggle "Use a cheaper model under pressure". When on: "to [sonnet ▾] when [Red ▾]". The *when* options are Amber or worse · Red · "{model} window in Red". Cost line, always visible when on: "Switches at the next task, never mid-task. Loses the prompt cache; the conversation is kept."
- **States:**
  - *downgrade off*
  - *downgrade on*
  - *warning on Lead/Reviewer*: "This role is the team's lead. A downgraded lead plans and reviews with a weaker model; the starters never do this." The toggle stays usable.
  - *same model*: Save is disabled, "Pick a model other than the role's ({sonnet})."

### 5.6 Feature pane: Design section (04)
- **Placement:** this replaces the "design" lines in P2's "PR and artifacts" card. Non-design artifacts stay there.
- **Grouping:** tiles grouped by "from → to" (header "ui-1 → front-2 · 4"), newest group first.
- **Tile:** preview (or tool tile) · title · note (2 lines, ellipsis) · time "13:12" · state chip · "Open ⏎".
- **States:**
  - *no designs yet*: "No designs yet. {ui-1} is on '{Welcome frames in Penpot}'. Exported frames show here when they're handed off." If no design role exists: "No designs in this feature."
  - *links only*: Penpot/Figma tool tiles with file/frame name. Each also shows the D9 hint, "Link only: front-2 can't open it. Ask ui-1 to export · x".
  - *previews*: thumbnails
  - *preview loading*: a skeleton at the tile's aspect, no spinner
  - *too large*: "Too large to preview ({7.4} MB · max 5 MB)"
  - *missing on branch*: "Not found on {ui-1}'s branch or the integration branch."
  - *not merged yet*: chip "On {ui-1}'s branch · not merged". The preview still shows, from the branch.
  - *not previewable*: a file tile with its extension ("welcome.fig: no preview").

### 5.7 Large preview overlay (04)
- **Layout:** a full-pane overlay on `surface.overlay`. The image is fit by default. Caption: "{welcome-2.png} · {1512 × 982} · {214 KB} · {integration branch | ui-1's branch}". Corner hints: `0` fit · `1` 100 % · `←/→` · `⏎` open · `esc` close.
- **States:**
  - *image*
  - *svg*: rendered as `<img>`; the caption adds "SVG shown as an image"
  - *error*: "Could not load this preview: {not found on branch}. Open ⏎ to try Quick Look."

### 5.8 Presets (02)
- **Preset card menu** (`⋯` or `⏎` then menu):
  - Duplicate to global `D` then `1`
  - Duplicate to {shop-api} `D` then `2`
  - Save as preset / Remove from presets (custom teams)
  - Open in canvas `⏎`
- **States:**
  - *default*
  - *menu open*
  - *update available* (modified starter copy): a chip "Update available" + the line "The {Design sprint} starter changed in this version. Your copy was edited, so it wasn't updated. Duplicate the new starter to compare." Action: "Get the new starter" → duplicates it as `{design-sprint}-v{2}`.
- **Starter card:** "Starter · read-only". Editing it says "Starters are templates. Your changes will be saved as a copy in {shop-api} ⏎ / global g."

### 5.9 Week planner: Weekly report (04)
- **Layout:** from the J8 · 4 frame, plus:
  - **plan vs actual** table: allocation · weight · planned pp · used pp · delta (+/− with ▲▼ glyph) · queued p50 / max
  - **breakdown tabs** 1–4: Row = label · pp · share bar · ≈ API $ (muted, D7)
  - **state time bar**: segments with labels, not color only
  - **governor actions**: kind · count · undone
  - alerts and limit hits as one list
  - **model windows**: peak % per window
  - **past reports** list (`R`): "Week 43 · 93 % · Mon 26 Oct", "Week 42 · 94 %"…
- **States:**
  - *current week*: header "Week 44 · so far · Mon 26 Oct 09:00 → now". Totals say "so far", and the suggestions block is hidden.
  - *past week*
  - *no data*: "No usage was recorded this week. The app was not running, or no agents ran."
  - *estimated (low confidence)*: the confidence marker (P1) + "Estimated: fewer than {20} usage samples this week; pp per row are approximate."
- **Unattributed row:** "Outside the app".

### 5.10 Model windows (02/04)
- **Planner:** one extra row under the 5 h window, per model window seen: "opus week · 92 % · Red · resets Mon 26 Oct 09:00". It uses the pressure pill with its label.
- **Pacing indicator (expanded):** the same chip.
- **Title bar compact:** nothing extra unless the state is Red or exhausted: then "· opus 92 %".
- **States:**
  - *none*: hidden entirely
  - *green*
  - *red*
  - *exhausted*: "opus week · 100 % · exhausted · resets Mon 09:00"
- **Queued explanation:** "Queued: opus week in Red · starts after the reset, Mon 09:00 (in 4d 22h) · Run now ≈ +2 pp". When exhausted, there is no Run now: "Queued: opus week exhausted · starts Mon 09:00 · or set a downgrade for this role".

### 5.11 Off-hours (02/04/05)
- **Actions:**
  - "Run off-hours `O`" on a queued entry (Overview row, planner queue row, the not-started agent strip) and in the task card menu (Board, Feature)
  - Launch dialog checkbox "Run off-hours (starts 18:00)"
  - The toggle back is "Run in working hours `O`"
- **Queued explanation:** "Off-hours · starts 18:00 (in 7h) · Run now ≈ +2 pp". *Running off-hours*: the agent status line adds "· off-hours".
- **Planner curve:** off-hours windows as hatched bands, labelled "off-hours" once per day. Queued off-hours work is a count marker at the next window's start: "3 queued".
- **Power chip** (status bar): "☾ Keeping awake · 3 off-hours" on AC with the assertion held. On battery: "☾ On battery · 3 off-hours waiting". Hidden when nothing is pending.
- **States:**
  - *queued off-hours*
  - *running off-hours*
  - *on battery*: banner §6.1 + chip
  - *allowance 0*: the action is disabled, tooltip "Off-hours allowance is 0 % this week. Set it in the plan (e)."

### 5.12 Agent header: downgraded model (02)
- **Header model field:** "sonnet · downgraded (Red)". The tooltip: "Role model is opus. Switched at 14:02 because the week is in Red; back to opus at the next task once it clears."
- **Governor log:** "14:02 ↓ Downgraded front-2 opus → sonnet at its task boundary (Red) [Undo u]". The undo toast reads "front-2 goes back to opus at its next task". The restore entry: "Thu 09:10 ↑ Restored front-2 to opus (Green)".

### 5.13 Project settings: Task backend Linear (04)
- **States:**
  - *not connected*: the Linear option is enabled. Selecting it shows "Linear is not connected. Add a key in Settings › Integrations ⌘+," and hides the pickers.
  - *loading teams*: skeleton
  - *mapped*: J13 · 4
  - *sync errors*: the last sync line turns into an error box. Examples: "Last sync 15:30 failed: Linear rate limit, retrying at 15:35 · 2 changes waiting to push", or "ENG-150: state 'QA' isn't mapped. Map it ⏎".
- **Sync now** `r`. The report line: "Last sync {15:04}: {7} pushed · {2} pulled · {1} conflict (Linear won)".

### 5.14 Notion publish (04, C)
- **Feature pane header action:** "Publish to Notion `N`", or "Refresh in Notion `N`" once published. Line: "Published {Wed 15:20} · Open in Notion ↗ · {2} comments".
- **States:**
  - *not configured*: the button is disabled, tooltip "Set a Notion parent page in shop-api's settings, and connect Notion (⌘+,)"
  - *publishing*: "Publishing 4 docs…"
  - *published*
  - *error*: "Publish failed: Notion says the integration has no access to the parent page. Share the page with the integration in Notion."
- **Toast:** "Published 4 docs to Notion · Open o".

### 5.15 Inbox items (02)

| Item | Level | Title | Preview (who / what / why) | Actions |
|---|---|---|---|---|
| MCP needs auth | 2 | "Penpot MCP needs authentication" | Who: penpot (claude.ai connector). What: ux-1 and ui-1 in onboarding use it; their Penpot tools fail. Why: an auth error at 13:55 / probe got 401. | `a` Reconnect (D3) · `o` Open Settings · `r` Dismiss until it fails again |
| Weekly report ready | 4 (digest) | "Weekly report for week 43 is ready" | — | `o` Open |
| Linear sync: remote won | 4 (digest) | "ENG-140 changed in Linear: kept Linear's version" | "Title and state changed in Linear and in the app since the last sync; the app now shows Linear's." | `o` Open the task |
| Not on AC | 2 | "Off-hours work needs AC power" | "3 off-hours tasks won't run while the Mac sleeps tonight." | `o` Details (planner) · `r` Dismiss for tonight |
| Notion comment (C) | 3 (question) | "Comment on onboarding · spec.md" | "{Author}: {text}" | `o` Open in Notion (no ⏎ reply: replies happen in Notion) |

The MCP item closes itself when the server is connected again. The timeline keeps "Penpot reconnected 14:05".

---

## 6. Copy for page 05

Rules as in P1: sentence case, no exclamation marks, name the agent and project, pp for usage, 24 h times with a relative hint.

### 6.1 Banners

| Banner | Title | Message | Actions |
|---|---|---|---|
| MCP needs auth (new) | "Penpot MCP needs authentication." | "Agents using it will fail its tools: ux-1, ui-1 (onboarding)." | Reconnect ⏎ · Settings o |
| Not on AC (replaces P1 text, D6) | "Off-hours work needs AC power." | "{3} off-hours tasks won't run while the Mac sleeps. Plug in to keep it awake, or they wait for the next off-hours window." | Details o |

### 6.2 Toasts (past tense + object)
- "Linear token stored in Keychain"
- "Linear key removed from Keychain"
- "Duplicated Design sprint to shop-api · Open o"
- "Duplicated role ui to global"
- "Saved Full-stack as a preset"
- "Published 4 docs to Notion · Open o"
- "Synced shop-api with Linear · 7 pushed"
- "deps-1 runs off-hours · starts 18:00 · Undo u"
- "front-2 goes back to opus at its next task" (downgrade undo)
- "Penpot connected" (after Test)

### 6.3 Queued reasons
| Reason | Copy |
|---|---|
| Scheduled off-hours (D4) | "Off-hours · starts 18:00 (in 7h)" |
| Outside working hours (P1, unchanged) | "outside working hours" |
| Model window Red | "opus week in Red · starts after reset Mon 09:00" |
| Model window exhausted | "opus week exhausted · starts Mon 09:00" |

### 6.4 Warnings
- **Secret in agent env:** §5.4.
- **Downgrade on Lead/Reviewer:** §5.5.
- **Connector with strict MCP:** §5.5.

### 6.5 macOS notifications
- **Level 2 · MCP.** Title: "Penpot MCP needs authentication". Body: "ux-1 and ui-1 will fail its tools. Reconnect from the Inbox."
- **Level 2 · AC.** Title: "Off-hours work needs AC power". Body: "3 tasks are queued for tonight. Plug in to keep the Mac awake."

---

## 7. Keyboard additions for page 05

Checked against Penpot 05 (P1 + P2 rows), SPEC §9.7 and the shipped handlers (`ui/src/lib/keyboard.js` and the panes).

| Scope | Key | Action | Conflict check |
|---|---|---|---|
| Global | `⌘+,` | Open Settings | Free (not in CHORDS, NATIVE or menu.rs). Add to the app menu as "Settings…", the macOS standard. |
| Settings | `j/k`, `⏎`, `esc` | Move sections/rows, enter, back | New pane |
| Settings › Integrations | `t` test · `⏎` connect · `x` remove | — | New pane |
| Settings › MCP servers | `n` new · `⏎` edit · `t` test · `x` delete · `⌘+⏎` save drawer | — | New pane; `⌘+⏎` = submit (05) |
| Secret field | `⌘+⏎` save · `esc` cancel | — | Same as dialogs |
| Feature | `d` | Focus the Design section | Free (e b c p m tab used) |
| Feature | `tab` | Cycle docs → tasks → timeline → **design** | Extends the P2 tab cycle |
| Feature › Design | `h j k l` / arrows · `⏎` open · `Space` preview · `x` ask to export (link tiles) | — | Free in Feature |
| Preview overlay | `Space`/`esc` close · `←/→` · `0` fit · `1` 100 % · `⏎` open | — | Modal |
| Feature | `N` | Publish / refresh in Notion (C) | Free (`n` unused) |
| Canvas | `D` | Duplicate the selected role, or the team if nothing is selected, then `1` global / `2` project | Free (⇧R library, ⇧hjkl move node) |
| Library / preset card | `D` | Duplicate preset | Same |
| Week planner | `R` | Reports list | Free. Close to `r` "keep running" on an ask: different keys, and the list is listed for clarity like the r Overview/Inbox entry |
| Planner report view | `1–4` tabs · `[`/`]` prev/next report · `e` markdown · `⏎` plan next week · `esc` back | — | e/⏎ as in the J8 · 4 frame |
| Board / Overview / planner queue / not-started strip | `O` | Run off-hours / run in working hours | Free. Not in a live agent pane (PTY): use ⌘K "Run off-hours" |

Palette entries (⌘K) for each: "Settings", "Settings: Integrations", "Settings: MCP servers", "Test {integration}", "Publish to Notion", "Duplicate team/role…", "Run off-hours", "Run in working hours", "Weekly report", "Past weekly reports".

---

## 8. Penpot placement (suggestion for ui)

- **04 Screens, row y = 6000:**
  - Settings · Integrations
  - Settings · MCP servers (list + drawer)
  - Feature · Design (previews)
  - Large preview overlay
  - Week planner · Weekly report
  - Project settings · Linear
  - Notion publish (C)
  - States boards per screen beside each
- **02 Components:** new sets
  - Integration row
  - Secret field
  - MCP status badge
  - Design tile
  - Model window chip
  - Power chip
  - Inbox item variants (§5.15)
  - Role inspector MCP picker + downgrade
  - Preset card menu
- **06 Flows:**
  - J12 notes at y = 5200, frames at y = 5440, x step 1612 (8 frames)
  - J13 notes at y = 6700, frames at y = 6940 (5 frames)
  - J8 · 4: notes text replaced in place
