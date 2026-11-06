# P4.5 design notes: Intake (Ready to start, Intake settings, brief card)

UX pass for P4.5 (Intake), 2026-11-06, by `ux` for `ui` (then `front`). Sources: [spec](../../.orchestrator/features/p-4-5-intake/spec.md), [API contract](../../.orchestrator/features/p-4-5-intake/api-contract.md), [UX brief](../../.orchestrator/features/p-4-5-intake/ux-brief.md), [decisions](../../.orchestrator/features/p-4-5-intake/decisions.md), and the earlier notes: [P4.2](p4-2-design-notes.md) (**P4.2 §n**: lanes §1.1, queue row §1.5, details §1.8, empty states §1.11, Run now §1.12, keys §3), [P4.3](p4-3-design-notes.md) (**P4.3 §n**), [P4.4](p4-4-design-notes.md) (**P4.4 §n**: rule proposal §7 is the model for the brief card, keys §11). Their copy and rules stand unless a section here says "supersedes".

Copy rules as before: sentence case, no exclamation marks, name the agent / project / item, local 24 h times, **every number says what it is**. The Overwatch is "the Overwatch" in running text, "Overwatch" as a label. Source names in copy: **GitHub**, **Linear**, **Repo docs**. Item text (titles, bodies, labels) is untrusted: always rendered as plain text (no HTML, no images, no clickable links inside the body); only the "Open source ↗" control navigates.

Point numbers follow ux-brief §2 (1–11). Widths: Mission control row area is 1512 − chat column (360) − rail; check every board at the 1512 and 1280 shells (chat column 340 at 1280, P4.3 §9).

**Visual rule for ui (principle "Ready is not attention")**: the lane has no accent, no warning colour, no badge; neutral surface, muted text, a 1 px **dashed** left rule instead of the solid attention rule, and a tint that is **not** the Board's "Ready" column colour (check page 04.1 and pick a different token; if none is free, use neutral with no tint). Only a source **error** uses the error token, on the header line, not on rows. The estimate warning is muted warning text, never a banner.

---

## 0. Findings for lead

No change made to spec or contract. "Proposal" is what these notes assume until lead decides; ui draws the proposal unless told otherwise.

| # | Finding | Proposal |
|---|---|---|
| F1 | **`error.fix` has three values, but one needs a command and one needs no fix.** `gh_scope` ("gh auth refresh -s read:project") needs the command shown and copyable; `rate_limited` and `failed` have no sensible settings fix; the contract's `fix` is "settings \| integrations \| gh_scope" with no null. | `fix` may be `null`. Table in §4.2 fixes the mapping per code: gh_missing, gh_auth, linear_key, linear_auth → `integrations`; not_found, bad_glob, branch_missing → `settings`; gh_scope → `gh_scope` (the UI renders the fixed command, not the core's message); rate_limited → `null`; failed → `null` (Retry). |
| F2 | **`rate_limited` carries its retry time only inside `message`.** The UI wants to disable Refresh until then and print "retry at 14:20" in local time. Parsing a message is fragile. | Add `error.retry_at_ms?` (only for `rate_limited`). The UI formats it; `message` is not shown for this code. |
| F3 | **`error.message` is not safe copy.** For `failed` and `not_found` it can carry stderr or API text (untrusted, long). | The UI shows its own copy per code (§4.2) and puts `message` only in the details tooltip, clamped to 200 chars. For `failed` the UI prints "Could not refresh: <message>" clamped to one line. Back: keep `message` ≤ 200 chars, plain text. |
| F4 | **`intake_preview.error` has no shape.** §1.1 gives validation errors `{code: bad_request, field, message}`; §2.1 gives source errors `{code, message, fix}`; the preview's `error` can be either. | Preview `error` = the §2.1 shape for source errors, and validation errors stay command errors (`bad_request` with `field`) so the UI puts the message under the field. State this in the contract. |
| F5 | **`IntakeItem` lacks what the details ask for.** ux-brief §2.3 and spec §3.2.2 list milestone, priority, assignees (GitHub), Linear priority and points; the item has only labels, size and excerpt. | `intake_item` returns, beside `body` / `truncated`: `milestone?: string`, `priority?: string` (Linear: "Urgent", "High", …), `assignees: string[]`, `points?: number`. All optional; the details row is left out when absent. |
| F6 | **`room_pp` has no scope.** "over this week's room (3 pp left)" is true for the week, but when the project has an allocation the room is the project's; the copy must say which. | Add `estimate.room_scope: "allocation" \| "week" \| null`. Copy: "over shop-api's room this week (3 pp left)" / "over this week's room (3 pp left)". Until it exists the UI uses the second form. |
| F7 | **`over_room` only for the suggested mode.** Details show both modes with the room against each. | None needed: UI computes `high_pp > room_pp` for the other mode itself. Said here so back doesn't add a second flag. |
| F8 | **"Updated 12 min ago" per source, but the lane has one header.** `sources[].refreshed_at_ms` is per project; with several projects the header time is ambiguous. | Header shows the **oldest** `refreshed_at_ms` among the shown projects with a source (honest worst case); each project's own time is in the tooltip and in its status line. No contract change. |
| F9 | **`count`, `more`, `hidden` semantics are not defined.** Is `count` all ready items upstream (including those cut by the cap of 50) or the ones in `items`? Does it include hidden ones? | Proposal: `count` = ready, **not hidden**, items in `items` for this project (≤ 50); `more` = ready items beyond the cap, not in `items`; `hidden` = hidden items in `items`. Header total = Σ `count`. "+ n more" = `more`. |
| F10 | **"+ n more in <source>" needs a link target.** `sources` has a label but no URL. | Add `sources[].url?` (GitHub issues list with the filter, Linear team / project view; null for repo docs, which opens the glob's folder in the editor via the project path). |
| F11 | **The `stale` error does not say what started the item.** Spec wants "Already started: <run>"; the error is `{code: "stale", message: "already started"}`. | Add `started: { kind: "run" \| "agent", name: string }` to the `stale` error for items (absent for briefs). For a brief, `stale` with `message: "brief closed"` (dismissed, revised, project removed). `intake_prefill` returns the same errors, so the dialog never opens on a stale item. |
| F12 | **Start from a hidden item.** The core checks `state = ready`; hidden is a different state, so a start from a hidden item is `stale`, with a misleading "already started". | Rows of hidden items offer **Unhide** only (no Start); if it still happens, `stale` message `not ready` (code is enough; UI copy decides). Stated in §1 and §2. |
| F13 | **Origin on outcomes.** `origin` is on `AgentAttention` / `RunAttention`; the Done today lane shows outcomes (merged PR, ended session) which would lose "from #123". | Add `origin` to `OutcomeAttention` too (same shape). §6 draws it there; if lead declines, the mark disappears when the run moves to Done today, which looks like a bug. |
| F14 | **`origin.kind: brief` has no title.** "from the Overwatch's brief" is a dead end; the user wants the brief's title and a way back to the card. | `origin: { kind: "brief", decision: "d_88", title: "Partial refunds" }`. Link target = the decision's line in the chat (`chat:<id>` scheme, P4.4 F9). |
| F15 | **`brief` decision has no title field.** The card heading and the collapsed line "launched partial-refunds" need the title and project name; context blocks are `markdown` / `text` / `cost` only. | Add `title` (≤ 80) and `project` to the decision (they exist in `propose_brief`). The `text` block stays "project · mode · team". |
| F16 | **Global intake settings have no commands.** Refresh interval (`intake.refresh_secs`) and default estimates (team 6 pp, solo 1.5 pp) are named in the spec; the contract has no get / set. | Reuse the settings pipeline (`settings_get` / `settings_set` of Pacing). Keys `intake.refresh_secs` (60–3600, default 600), `intake.default_team_pp`, `intake.default_solo_pp` (0.5–50). Where they sit: §7.4. |
| F17 | **No way to show *why* an item is hidden or when it returns.** `hidden` exists; the reason (user hid it) is implied. For repo docs `status: done` hides the item from the source side: not the same as the user hiding it, and not in `items` (spec says "hides the item"). | Nothing to do if front-matter-hidden files are simply absent from `items` (assumed). The "n hidden · Show" counts only user-hidden items. Confirm. |
| F18 | **Report line has no names for untracked starts.** `intake { started, planned, untracked }` is counts only; `orchd intake --week` prints "1 untracked: agent fix-ci-3". The report pane can only say "1 not from intake". | Add `untracked_items: [{ kind, ref }]` (≤ 5) to report JSON so the pane can name them in a tooltip. Optional; copy works without. |
| F19 | **GitHub Projects v2: nothing offered to pick from.** `intake_options` returns labels and milestones for GitHub; for `project` the user has to type owner, number and the Status value blind. | Add `projects: [{ owner, number, title, statuses[] }]` (the user's / repo owner's open projects) to the GitHub `intake_options`. Until then the Project filter is three free fields with a hint (§7.2). Needs `read:project`, so its error (`gh_scope`) can appear in the Options call: show it inline under the filter. |
| F20 | **Brief card estimate: which mode?** The `cost` block holds the full Estimate (both modes); the card shows the proposed mode's. | None: the UI reads `estimate[mode]`. The brief's `mode` is in the `text` block only as text; add `mode` and `team` as fields on the decision so the UI does not parse "shop-api · team run · default-web". |
| F22 | **Linear estimate sizes have a hole** (spec §3.4: ≤ 1 S, 2–3 M, ≥ 5 L: 4 and 1.5 map to nothing). | Back: ≤ 1 S, ≤ 3 M, ≥ 4 L, fractions by the same thresholds. |
| F23 | **`intake_options` takes a full `source`**, but suggestions are needed before a filter exists (`names: []` is `bad_request`). | `intake_options` accepts a partial source (GitHub: `repo` only; Linear: `team` only); say so in the contract. |
| F24 | **Team availability on an item.** `team` is null for solo-suggested items, so the UI can't tell "project has no team" (§2.2 disables Start team run). | Add `can_team: bool` on `IntakeItem`. |
| F25 | **Glob `*` crossing `/`** (`globset` default): `backlog/*.md` would match `backlog/archive/x.md`. | Spec: require `literal_separator(true)`. |
| F26 | **Event fields differ** between spec §3.7 (`est`) and contract §6 (`est_pp`, `project` on planned). | Contract §6 is the source of truth; fix the spec. |
| F21 | **Prefill and the chat draft for a project the user filtered out.** "Plan with the Overwatch" from the palette needs a project; with a filter showing one project it is that one, otherwise a choice. | UX decision, no contract change: §8.1. |

---

## 1. Lane "Ready to start" (point 1)

### 1.1 Position, header, count
- **Position**: after **Queued**, before **Done today** (P4.2 §1.1 order: Needs you · Running · Waiting on · Queued · **Ready to start** · Done today · Idle).
- **Not attention**: no notification, not in the badge, not in "Needs you", does not alter the rail or the window title. The lane is collapsible like the others (collapsed state remembered with the other lanes).
- **Header** (one line, 28 px, muted): `Ready to start · 6 items` + status text on the right + actions.

| Part | Copy |
|---|---|
| Title + count | "Ready to start · 6 items" · "Ready to start · 1 item". Count = Σ `count` (F9), hidden not counted. |
| Status text, right | "updated 12 min ago" (oldest refresh, F8; relative: "just now", "3 min ago", "2 h ago", "yesterday 18:02") · "refreshing…" while any shown source is `refreshing` (static text under reduced motion, otherwise a 12 px spinner before it) · "2 sources failed" (error style) · "1 source failed" |
| Refresh | Ghost button "Refresh" with `⇧R`; disabled while refreshing ("Refreshing…") and for `rate_limited` until its time (tooltip "Retry at 14:20"); tooltip otherwise "Check GitHub, Linear and repo docs now". Calls `intake_refresh` for the **shown** projects (one call per project, so a filter is respected). |
| Hidden toggle | "3 hidden · Show" / "Hide the 3" — button next to Refresh, only when Σ `hidden` > 0. Lane-wide, remembered per session only. |
| Collapsed | "Ready to start · 6 items ▸" (counts stay, as P4.2 §1.1). A source error stays visible in a collapsed header ("· 1 source failed", error style) — collapsing never hides a failure. |

- **When the lane exists**: whenever at least one project is shown by the filter. With every shown project in a no-source / empty state the lane still renders with its lines (§4). With no projects at all it is absent (P4.2 §1.11 "No projects yet" stands).
- **Per-project filter** (P4.2 §1.9) applies: filtered-out projects contribute nothing, not even errors or lines. The filter button tooltip does not count Ready items.

### 1.2 Grouping with several projects
- **Order**: by project in the registered projects order (the same order the filter popover lists), and inside a project by the source's own order (`items` array order, which the core already sorts: spec §3.2.2). No re-sorting in the UI.
- **No sub-headers**: every row names its project (§2), so group headers would repeat it. Projects are separated by a 6 px extra gap only.
- **A project's non-item lines** (error, empty, refreshing, "+ n more") sit in that project's position, after its items (error first when there are no items). Lines of the same kind from several projects stay separate except the two merges in §4.1.
- **Hidden items** (when "Show" is on) sit after the project's ready items, dimmed, tagged "Hidden" (§4.1).
- **Cap**: ≤ 50 rows per project (the core's cap). After them: "+ 12 more in acme/shop · Open ↗" (muted line, `⏎` opens `sources[].url`, F10). No lane-level "Show all" (the cap is a source concern).

---

## 2. Row (point 2)

One row per `IntakeItem`, two lines, 52 px (row height token of the lane rows).

```
◐ #123  Refunds on partial orders           shop-api    ready · size:M      ≈ 4–7 pp week · team   [Start team run ⏎] [Start solo a]  ⋯
        shop-api · over this week's room (3 pp left)                                                                      
```
Exact layout:

| Slot | Content | Rules |
|---|---|---|
| Glyph (16 px) | GitHub mark, Linear mark, file glyph for repo docs | monochrome (text-muted), not coloured; `aria-label` "GitHub issue", "Linear issue", "Repo doc". New component on page 02: **Source glyph** (3 variants), **Ready row**. |
| Ref | `#123` · `ENG-42` · `backlog/checkout-v2.md` | mono, muted, fixed max 22 chars. Repo docs: the path **without** the glob's static prefix when all items share it (`.orchestrator/backlog/` is dropped, tooltip full path); otherwise the last two segments with a leading `…/`. **Canonical short form everywhere** (dialogs, chips, origin): the same rule's result, so `checkout-v2.md` for the default glob; the examples `backlog/checkout-v2.md` in this file mean a glob whose prefix is not shared. |
| Title | `title` | one line, ellipsis; 100 % of the space left. Tooltip full title. |
| Project | project name | muted, 12 px; right of the title; for a single shown project it is still shown (the rows are copied into other places). |
| Labels | ≤ 2 chips, then "+n" chip | label text in a neutral chip; `size:*` labels are **not** shown as chips (they feed the estimate, which shows "size M" in details); `orch*` never appears. Order as the source gives. |
| Estimate | "≈ 4–7 pp week · team" | for the **suggested mode** only (§2.1). Tooltip: "Team run: ≈ 4–7 pp of the week. Basis in details." |
| Warning (line 2) | "over this week's room (3 pp left)" | muted warning text, only when `over_room`; see §2.1. |
| Actions | `Start <mode>` primary (suggested) · `Start <other>` secondary · Hide in the overflow `⋯` | §2.2 |

Row states (all rows): rest · hover (actions visible; at rest the two buttons are still visible, not hover-only: keyboard-first and touch of the principle) · focused (focus ring, `j/k`) · selected for details · starting (a dialog is open for it: the row shows a muted "Starting…" tag, buttons disabled, until submit or cancel) · hidden (§4.1).

### 2.1 Estimate copy
- Range from `estimate[mode].low_pp` / `high_pp`, shown without decimals: round each end to the nearest integer; if both ends round equal, one number ("≈ 4 pp"); if high < 1, "< 1 pp"; if low rounds to 0 and high is 1 or more, "< 1–N pp" (0.4–1.4 → "≈ 1 pp"; 0.4–2.6 → "< 1–3 pp"). Examples: 4.1–6.8 → "≈ 4–7 pp week"; 0.9–1.6 → "≈ 1–2 pp week"; 0.3–0.6 → "< 1 pp week"; 5.6–6.4 → "≈ 6 pp week". The unit always has "week" and the mode ("· team", "· solo"): "≈ 4–7 pp week · team".
- **Basis flag in the row**: `basis: default` adds a muted "rough" prefix: "rough ≈ 4–8 pp week · team". Tooltip: "No finished runs yet: using the default (6 pp)". `project` / `global` show no flag (the basis is in details).
- **Estimate missing** (older cache, core error): "no estimate" muted, row still startable.
- **Over room** (`estimate.over_room`): line 2, muted warning text with the warning glyph: "over this week's room (3 pp left)" (with F6: "over shop-api's room this week (3 pp left)"). `room_pp` < 0.5: "over this week's room (none left)". `room_pp` null: nothing. It never disables Start and never changes the button style; the dialog repeats it as one line (§5).
- The warning compares the **suggested** mode (core `over_room`). If the other mode would also be over, details say so.

### 2.2 Start actions
- **Two buttons, suggested first** (`mode` from the item: spec §3.3 suggested mode rule). Labels: "Start team run" / "Start solo". Both are **neutral outlined** buttons (no accent fill: 50 rows must not make this the loudest lane); the suggested one has a slightly stronger border and shows `⏎`. The accent fill is reserved for the dialog's submit. Key hints inside: team `t`, solo `a`, and `⏎` shown on the suggested one ("Start team run ⏎").
- If the item has `mode: team` but the project has **no team** (`team` null and project has none): the team button is replaced by a disabled "Start team run" with tooltip "shop-api has no team yet", and solo becomes the suggested one. (Edge: front matter asks for a mode that can't be satisfied.)
- **Accessible names**: "Start team run for #123 Refunds on partial orders" (full item, not "Start").
- **Never one click to spend**: both buttons open the prefilled dialog (§5); nothing starts on press. The toast/confirm for a start is the dialog's submit.
- **Hide**: `⋯` menu item "Hide" (`x`) with the row's other secondary actions "Open source" (`o`) and "Details" (`space`). Hidden rows show **Unhide** in place of both Start buttons (F12).
- After Hide: toast (only after the core confirms, P4.1 §4): "Hidden #123. It comes back if it changes at the source." with **Undo u** (5 s). Unhide: "#123 is back in Ready to start."

### 2.3 Density at 1280
Content width under ~1000 px (1280 shell with the chat column):
1. Project name and labels **drop to line 2** (`shop-api · ready, +1`), after the warning when there is one; the title gets the full line 1 width.
2. Buttons shorten to **"Team run ⏎"** / **"Solo"**, keys unchanged; the accessible name stays long.
3. Estimate stays on line 1 right of the title, text "≈ 4–7 pp · team" (the unit "week" moves to the tooltip, and the lane header's tooltip says "pp of the week"). Rule exception to "every number says what it is": at this width the lane header carries "Estimates are in pp of the week" as a muted line under the title row (one line, always shown at ≤ 1000 px).
4. Row height stays 52 px; never 3 lines. A long title ellipsises first; labels and project next.

---

## 3. Row details — `space` (point 3)

Opens under the row (inline, like P4.2 §1.8), pushes rows down; `space` again closes; one open at a time; `j/k` move away and close it.

```
#123 · GitHub
Excerpt / body (rendered as text, ≤ 12 lines, "Show more")
─────────────────────────────────────────────────────────────
Team run   ≈ 4.1–6.8 pp of the week     median range from 7 runs in shop-api · size M
Solo       ≈ 0.9–1.6 pp of the week     from 12 sessions across projects · size M
Room       3.0 pp left this week (shop-api's allocation)       [over this week's room]
Labels     ready · size:M · billing
Milestone  v2.1      Priority  High      Assignees  gu, mia
Updated    2 days ago (Wed 4 Nov 09:12)
[Open source ↗ o]   [Start team run ⏎] [Start solo a]  [Hide x]
```

| Part | Rules / copy |
|---|---|
| Body | First paint: `excerpt` (≤ 200 chars) + "Loading…" muted while `intake_item(item)` is pending; then `body` replaces it. Markdown rendered as **plain text with simple formatting** (headings, lists, code); no images, no HTML, no links (links print their URL as text). Clamp 12 lines + "Show more" (expands in place). `truncated`: muted line "Shortened to 4 000 characters. Open the source for the rest." Repo docs: the front matter block is not shown (its keys appear in the fields below). Failure: "Could not load the full text: <message>" (muted error) and the excerpt stays. |
| Estimates | Both modes, ranges with **one decimal** (details are exact; the row rounds). Basis column, in this copy (basis, samples, project): **project** "from 7 runs in shop-api · size M" (solo: "from 7 sessions in shop-api"); **global** "from 12 sessions across projects · size M"; **default** "rough: no history yet · size M (default 6 pp)". One sample: "from 1 run in shop-api, ± 40 % · size M". The spec's range is the middle half (p25–p75) of history times the size factor; the copy says "from n runs", not "median" (it is a range, not a median — **differs from the ux-brief example "median of 7 runs"**). Size: "size M" from `size`; `null` → "size M (not set)". |
| Room | Line "Room: 3.0 pp left this week for shop-api" (F6) or "… left this week" for unplanned; hidden when `room_pp` null. When the **other** mode is over room while the suggested one is not (or vice versa), the line under the mode row says "over this week's room" in muted warning text. |
| Labels | all labels as chips (including `size:*`, but not `orch*`, which never come). |
| Milestone / priority / assignees | only the ones present (F5). GitHub: milestone, assignees; Linear: priority, points, labels; repo docs: front matter `size`, `mode`, `team` shown as "Front matter: size L · mode solo · team default-web". |
| Updated | relative + absolute (local time). Repo docs: "File changed 2 days ago (commit a1b2c3d)" is not in the contract: use "Changed 2 days ago" from `updated_at_ms`. |
| Source link | "Open source ↗" (`o`): GitHub / Linear open in the browser; repo docs open the file in the editor (`url` null: opens `<project>/<ref>` in nvim as every file open, P4.1). Tooltip: the URL or path. |
| Actions | same Start buttons and Hide as the row (details do not replace the dialog). |

---

## 4. Lane states (point 4)

### 4.1 Non-item lines (each is a 36 px muted row inside the lane, `j/k` stops on it when it has an action)

| State | Copy | Action (key) |
|---|---|---|
| **One project, no source** | "shop-api has no intake source · **Plan with the Overwatch** · **Set up intake**" | Plan `⏎` · Set up `e` (opens Project settings › Intake) |
| **Several projects, no source** (merged into one line) | "shop-api, blog and infra have no intake source · **Plan with the Overwatch** · **Set up intake**" — list shows 3 names, then "+ 2 more"; with exactly the shown projects only | Plan `⏎` (asks which project: §8.1) · Set up `e` (opens the first, then the settings project switcher) |
| **Source, zero items** (ok) | "Nothing ready in acme/shop · label ready · **Refresh**" (label = `sources[].label`) | Refresh `⇧R` for that project |
| **Several sources, zero items** | up to 2 projects: one line each as above; 3 or more: "Nothing ready in 5 projects · **Refresh**" | Refresh `⇧R` |
| **Source error, items kept** | header goes to "1 source failed" (error style); the project's line: "shop-api · GitHub: not logged in · **Fix**" above its still-shown items, which keep their rows and the header time "updated 3 h ago" | Fix: `⏎` on the line (`f` stays the filter) |
| **Source error, no items** | the same line only | |
| **Rate limited** | "shop-api · GitHub rate limit reached · retry at 14:20" — no Fix; Refresh disabled until then | none |
| **Refreshing, items cached** | items stay; header "refreshing…" | — |
| **First launch** (cache empty, refresh running) | header "Ready to start · refreshing…" (no count); per project with a source: "Looking for items in acme/shop…" (muted, spinner / static); no "Nothing ready" line while `refreshing` | — |
| **Many items** | 50 rows then "+ 12 more in acme/shop · Open ↗" | `⏎` opens |
| **Hidden shown** | after "Show": hidden rows appear below their project's ready rows, dimmed (60 % opacity text), tag "Hidden", action **Unhide x**, no Start buttons; header toggle reads "Hide the 3" | Unhide `x` |
| **Everything hidden** | lane with only the line "All 3 items are hidden · **Show**" | Show `⏎` |
| **Filter hides every project** | lane absent (P4.2 §1.9 line stands) | — |

- A project with a source and an error never gets "Nothing ready" (the error wins).
- Counts of lines never enter the header count (only items).

### 4.2 Source error per code (final copy)

Line format: `<project> · <source>: <what happened> · <Fix>`. Error style (error colour for text and glyph, no fill) on the header text "n sources failed" and on this line. `message` is not shown except in the tooltip (F3).

| Code | Line (what happened) | Fix control (`fix`) | Target |
|---|---|---|---|
| `gh_missing` | "GitHub CLI not found" | **Fix** (`integrations`) | Settings › Integrations (GitHub row) |
| `gh_auth` | "GitHub: not logged in" | **Fix** (`integrations`) | Settings › Integrations |
| `gh_scope` | "GitHub needs the read:project scope" | **Copy command** (`gh_scope`): copies `gh auth refresh -s read:project`, toast "Copied. Run it in a terminal, then Refresh." | clipboard |
| `not_found` | "acme/shop not found, or you have no access" | **Fix** (`settings`) | Project settings › Intake |
| `linear_key` | "Linear: no API key" | **Fix** (`integrations`) | Settings › Integrations (Linear row) |
| `linear_auth` | "Linear rejected the API key" | **Fix** (`integrations`) | Settings › Integrations |
| `rate_limited` | "Rate limit reached · retry at 14:20" (F2) | none; Refresh disabled until then | — |
| `bad_glob` | "The glob doesn't parse: `.orchestrator/backlog/[`" | **Fix** (`settings`) | Project settings › Intake |
| `branch_missing` | "Branch `main` not found in this repo" | **Fix** (`settings`) | Project settings › Intake |
| `failed` | "Could not refresh: <message, one line>" | **Retry** (`null`): same as Refresh for that project | — |

Same strings appear in Settings › Intake's Preview (§7.3), without the project prefix and with the inline link instead of **Fix**.

---

## 5. Prefilled dialogs (point 5)

The existing `launch` (04.4) and `new-session` dialogs, opened by the row buttons / `t` / `a` / `⏎` or by a brief card. Everything is the dialog's today; the additions:

### 5.1 Origin chip
- Top of the dialog body, under the title, a one-line chip: **"From #123 · GitHub ↗"** (ref · source; ↗ opens the source). Linear: "From ENG-42 · Linear ↗". Repo docs: "From backlog/checkout-v2.md · Repo docs ↗" (↗ opens the editor). Brief: "From the Overwatch's brief · Partial refunds ↗" (↗ jumps to the card / line in the chat).
- Chip is not editable; there is no "remove the link" control (an unlinked start is just the normal ⌘T / launch path).

### 5.2 Fields
| Dialog | Prefilled | Editable |
|---|---|---|
| **Launch** (team run) | project, team (`launch.team_id`), slug (derived, unique), title, **brief** (markdown: title, source link, body; GitHub keeps `#123` so the lead can read the issue), base unchanged, *Run off-hours* unchanged (off) | all; brief is a multi-line text area, 12 lines tall, scrollable, focus on **Start** button for items (the user reads it first) and on the **brief** for "Edit" from a brief card |
| **New session** (solo) | project, name (derived), message = the same text + "When done, say so; the item is <url>." | all |

- Slug / name derived by the core (`intake_prefill`); the user may change them; the existing uniqueness check stays.
- **Over room** repeats as one muted warning line above the buttons: "≈ 4–7 pp of the week for a team run · over this week's room (3 pp left)". Also shown without the warning: "≈ 4–7 pp of the week for a team run". It is advice and never disables the primary button.
- **Untrusted text note**, muted, under the brief field (items only, not briefs): "This text comes from GitHub. It goes to the agent as part of the brief you send." Small and calm, no warning glyph.
- Submit label is the dialog's own ("Launch" / "Start"); the dialog title is unchanged. `⌘⏎` submits, `Esc` cancels (dialogs as P4.2 §1.12). Cancel leaves the item and the card exactly as they were (row's "Starting…" tag clears).
- A brief card's **Launch** opens the dialog with the proposed mode; the dialog's mode cannot be switched inside (it is two dialogs); to change mode the user uses Edit then the normal dialogs. (Not a contract matter: Edit opens the same dialog.)

### 5.3 `stale` error (F11, F12)
- Raised by `intake_prefill` when opening, or by `team_launch` / `agent_start` on submit (the item or brief went stale while the dialog was open).
- **On submit** the dialog stays open and shows an inline banner (error style, top of the body): **"Already started: checkout (team run)"** / **"Already started: fix-feed (solo session)"** (F11 gives the name). Primary button becomes **Close**; the form is disabled. Brief: "This brief was closed: it was dismissed, revised or its project was removed." Primary **Close**.
- **On open** (`intake_prefill` fails) the dialog does not open (F11); a toast (after the core answers): "#123 was already started: checkout." (brief: "This brief was closed.") and the row leaves the lane on the next `now`.
- Any other error on submit: the dialog's usual inline error; the item stays ready.

---

## 6. Origin mark (point 6)

| Place | Copy / form |
|---|---|
| **Run row** (Running, Needs you, Queued, Waiting on) | after the run name, a muted chip: `from #123` (link-style, mono ref); `from ENG-42`; `from checkout-v2.md` (the §2 short form, tooltip full path); brief: `from brief` (tooltip "From the Overwatch's brief: Partial refunds"). Click opens the source (GitHub / Linear: browser; docs: editor; brief: the chat line). Not part of the row's focus stops (no extra Tab stop); `o` on a run row keeps its P4.2 meaning (PR), and the chip is reachable through details. |
| **Solo row** | same chip after the session name. |
| **Queue row** | same chip (a queued member of an intake run keeps it). |
| **Outcome row** (Done today, F13) | same chip after the title: "merged · from #123". |
| **Row details** (`space`, P4.2 §1.8) | a line above the table: "Started from #123 Refunds on partial orders · GitHub ↗" (title as text). |
| **Inspector** (run, agent) | a field in the header block: label "Started from", value "#123 Refunds on partial orders ↗". Brief: "Started from the Overwatch's brief: Partial refunds ↗". |
| **Item deleted at the source / returned** | the chip stays (it records the start). `intake.returned` (run deleted or session killed) returns the item to the lane with no toast. |

Chip style: text-muted, 11 px mono for the ref, no border, no fill; hover underlines. It must not compete with the status badge.

---

## 7. Project settings › Intake (point 7)

Page 04.7. New section **Intake**, after Task backend in the section list (Project settings sections: General · Files · Task backend · **Intake** · Allocation · Notion · MCP). Section heading "Intake", subtitle "Where the work you can start comes from. Starting is always your action."

### 7.1 Layout
1. **Source** — segmented control: **None · GitHub · Linear · Repo docs** (None selected when `intake_source` null).
2. Fields for the selected source (7.2).
3. **Preview** block (7.3).
4. Footer: **Save** (primary, `⌘⏎`) · **Revert** (ghost, shown when changed). Unsaved-changes marker: a dot after the heading and the line "Unsaved changes" (muted). Save runs `intake_source_set` (validate, store, refresh at once) and shows its preview result in the block.
5. Hints block (muted, at the foot): "Issues labelled `orch` or `orch:*` are never listed: they are tasks of the task backend. · Pull requests are not listed. · Intake only reads: starting an item changes nothing at the source."

Choosing **None** shows: "No source. The Ready to start lane will offer Plan with the Overwatch for this project." with Save (clears source).

### 7.2 Fields per source

**GitHub**
| Field | Control | Copy / rules |
|---|---|---|
| Repository | text, prefilled with `known_repo`, placeholder "owner/name" | Hint "Defaults to this project's repository." Error (`bad_request`, field `repo`): "Use owner/name, e.g. acme/shop." Empty and a known repo → uses the default; no known repo → "Needs a repository." |
| Filter | segmented: **Label · Milestone · Project** | |
| Label | tags input; suggestions from `intake_options.labels`; chips removable | Hint "Items that have all of these labels." Empty: "Add at least one label." (field `names`). Options failing (rate / auth): inline under the field, same copy as §4.2 without **Fix** |
| Milestone | select from `milestones` ("v2.1 · 8 open") | Hint "Open issues in this milestone." Empty list: "No open milestones in acme/shop." |
| Project | three fields: Owner (text), Number (number ≥ 1), Status (text, optional) — or the F19 select when the contract adds it | Hints: "The project's number is in its URL." / "Status: the value of the project's Status field to keep, e.g. Todo. Leave empty for all." Error (field `number`): "Use a number of 1 or more." If `gh_scope` is returned: "GitHub needs the read:project scope. Run `gh auth refresh -s read:project`." with **Copy command**. |
| Login | line under the section when `gh_missing` / `gh_auth` came back: "GitHub isn't set up. Settings › Integrations ↗" |

**Linear**
| Field | Control | Copy / rules |
|---|---|---|
| Team | select from `linear_teams` | Required. Error (field `team`): "Pick a team." |
| Project | select, first item "Any project" | from `linear_states` projects |
| Label | select, first item "Any label" | from `intake_options.labels` (the team's) |
| Include backlog | checkbox | Label "Include backlog". Hint "By default only issues that are ready to start (Todo, Unstarted) are listed." |
| Key | when `linear_key` / `linear_auth`: "Linear isn't set up. Settings › Integrations ↗" (the fields stay disabled, Save disabled) | |

**Repo docs**
| Field | Control | Copy / rules |
|---|---|---|
| Glob | text, default `.orchestrator/backlog/*.md` shown as a value (not a placeholder: the default is the setting) | Live match line under the field (`intake_options.matches`, debounced 400 ms): "7 files match" · "No file matches" · "1 file matches". Errors (field `glob`): "The glob doesn't parse." · "Use a path inside the repo: no leading `/`, no `..`." |
| Branch | text, placeholder "main (base branch)" | Empty = base branch (resolved name shown in the placeholder: `default_branch`). Error `branch_missing`: "Branch `x` isn't in this repo." Hint "Read from the local branch, no checkout. A file committed on it shows within a minute." |
| Front matter hint | muted text under both | "Optional front matter: `title`, `size` (S, M, L), `mode` (team, solo), `team` (a team id), `status` (done or started hides the file). Without a title, the first `#` heading is used." |

### 7.3 Preview
- Button **Preview** (secondary), `⌘⏎` is Save so Preview has no shortcut; disabled while any field is invalid or a preview is running ("Previewing…", spinner / static text). Not required before Save, but Save of a changed source ends with the same result shown.
- Result block under the button:
  - Ok: "**4 items** in acme/shop · label ready" then up to 5 lines `#123  Refunds on partial orders` (mono ref + title, one line each) and, when `more` > 0, "+ 12 more". Zero: "Nothing matches this source right now. It may still be right: Ready to start will show items when they appear."
  - Error: error-style line with the §4.2 copy for the code, and the link ("Settings › Integrations ↗" or **Copy command**). Retry: **Preview** again.
  - Stale: after editing a field the result greys with the line "Changed since this preview" until re-run.
- **Saved** state: after Save, the heading line "Saved · updated just now" (muted, 5 s) and the block shows the real result. A failed Save keeps the form and shows the error under the field (`bad_request.field`) or in the block.

### 7.4 Global settings (Settings › Pacing)
A small block **"Intake"** at the end of Pacing (not a new section; the values are pacing-like, a handful of fields), page 04.7:

| Field | Control | Copy |
|---|---|---|
| Refresh every | select 5 min · 10 min · 30 min · 1 h (stored in seconds, 60–3600, F16) | Hint "How often Ready to start checks sources. Refresh in the lane checks at once." |
| Default estimate, team run | number, pp, step 0.5 | Hint "Used until there is history: 6 pp of the week." |
| Default estimate, solo session | number, pp | Hint "1.5 pp of the week." |

Invalid value: "Use a number between 0.5 and 50." Footer pattern as the other Pacing fields.

---

## 8. Plan with the Overwatch (point 8)

### 8.1 Entry points
| Entry | Behaviour |
|---|---|
| **Lane line** (§4.1) "Plan with the Overwatch" | Single project: uses it. Several no-source projects in the line: opens a small popover listing them (`j/k`, `⏎`) with a title "Plan a feature for…"; `Esc` closes. |
| **Palette** (⌘K) "Plan with the Overwatch…" | Always listed. One project in the filter → that project; otherwise the same popover (all projects, the filter ones first). |
| **Brief card › Revise** | Not a button on the card (the card has Launch / Edit / Dismiss, contract §5). "Revise" = the user answers in the chat and the Overwatch sends a new brief: the chat input already holds the thread's context. Spec §3.5 mentions "Revise" as an entry: treated as a hint in the card footer, §8.3. |
| **Project with a source** | Palette only (the lane does not offer it next to real items: starting from intake stays the main way). |

All entries: close any overlay, focus the Overwatch input (⌘L target), set the draft to **"Plan a feature for shop-api: "** with the caret at the end (plain text). The draft is replaced only if the input is empty or holds a previous planning draft; otherwise the project line is appended on a new line (never destroys what is being typed). Every send while the tag shows is sent with `context: { plan: project_id }` (a muted tag in the input, "Planning · shop-api" with an ✕ that clears the context but keeps the text). The tag stays after a send, so the user's follow-up answers keep the planning context (the Overwatch needs it to revise); it clears when the brief card is launched or dismissed, or on ✕.

### 8.2 Conversation
- The user's message shows a muted tag under it: "planning · shop-api".
- The Overwatch's replies are normal entries (questions, summaries). Sources it cites (project docs) are links as P4.4 F9.
- No status banner is added; the Overwatch's own status control (`working`, `asleep`) is enough. `Overwatch is working on it · 12 s` (P4.4 §2.2) applies.
- A new planning message with the same `reply_to` thread may produce a **revised** card (§8.3).

### 8.3 Brief card
Level 4: never notifies, not in the badge, not in "needs you" counts, never folded into "N more open", ⌘⇧I skips it unless nothing else is open (as the rule proposal, P4.4 §7.1); `j/k` stop on it.

```
BRIEF                                                    15:20
Partial refunds · shop-api
┌ brief ───────────────────────────────────────────────┐
│ ## Goal                                                │
│ Refund part of an order line…                          │   (markdown rendered, clamped at 12 lines)
│                                                  Show more │
└──────────────────────────────────────────────────────┘
shop-api · team run · default-web
≈ 4–7 pp week · from 7 runs in shop-api · size M
over this week's room (3 pp left)                   (only when over_room)
[ Launch ⏎ ] [ Edit e ] [ Dismiss r ]
Waiting for you · just now          Want changes? Answer here and it will send a new brief.
```
- **Title**: `<brief title> · <project>`. Mode line is `text` block: "project · team run · <team>" / "project · solo session".
- **Estimate**: from the `cost` block for the proposed mode, same copy and rounding as the row (§2.1, size M always; "rough" when basis default).
- **Footer line**: muted, replaces "Revise": "Want changes? Answer in the chat and the Overwatch sends a new brief." (only while open).
- **Brief body** is rendered as in row details (plain markdown, no links/images), clamp 12 lines, `Show more` expands in place.
- **Never says** that the Overwatch started anything; the marker line is "Waiting for you".

### 8.4 Lifecycle and states

| State | Marker / body | Actions |
|---|---|---|
| Open | `Waiting for you · 3 min` | Launch ⏎ · Edit e · Dismiss r |
| Dialog open | `Opening dialog…` then marker returns to Open; the card stays open while the dialog is open (card is closed only by the successful start) | actions disabled while the dialog is open |
| Launch / Edit failed to open | `Could not open the dialog: <message>` (error), `stale` → the Stale row | actions again |
| Launched | `Launched partial-refunds · 15:24`, muted; collapses at once to `▸ launched partial-refunds (brief) · by you` | none |
| Dismissed | `Dismissed · 15:21`, muted, collapses to `▸ dismissed brief: Partial refunds` | none (no undo: dismissing a draft is cheap, ask the Overwatch again) |
| Revised | `Revised · a new brief replaces this one`, muted, collapses to `▸ revised brief: Partial refunds · by the Overwatch`; the open card is the new one | none |
| Stale (project removed) | `Stale · shop-api was removed`, muted | **Dismiss r** only |
| Overwatch off / down | the card **stays** (cards work without the Overwatch, P4.4 §8); the footer line "Want changes?…" is replaced by "The Overwatch is off. Turn it on in Settings to ask for changes." / "The Overwatch is down. Messages are kept until it is back." | actions unchanged |

Collapsed lines follow P4.4 §5 anatomy (verb · detail · actor · expand): verbs `launched`, `dismissed`, `revised` (contract §5), actor `user` or `overwatch` for revised. The expanded body shows the full brief and, for launched, "Started as the run partial-refunds ↗" (or the session name).

### 8.5 Overwatch off / down at the entry points
- The entry still focuses the input and the draft is set. The input's hint (P4.4 §2) already reads "Off · turn it on in Settings" / "Down · messages are kept until it is back". The lane line gains a muted tail while that is true: "Plan with the Overwatch (off: your message waits)" / "(down: your message waits)". Palette item shows the same hint on the right.
- No card can arrive while off; once on, the message is delivered (P4.4 F10) and the card follows.

---

## 9. Report and planner line (point 9)

Page 04.6, the weekly report pane: one line under the week's totals row (P4.4 §10 table), before the per-feature breakdown:

| Case | Copy |
|---|---|
| All from intake | "Started from intake: 10 of 10" |
| Some not | "Started from intake: 9 of 10 · 1 not from intake" (muted tail; tooltip names them when F18 exists: "agent fix-ci-3") |
| With briefs | "Started from intake: 9 of 10 · 7 items, 2 briefs" — numerator = `started + planned`; denominator = `started + planned + untracked`; the tail "7 items, 2 briefs" only when `planned` > 0; with untracked too: "Started from intake: 9 of 11 · 7 items, 2 briefs · 2 not from intake"; total 0 → "Nothing started this week" |
| None started | "Nothing started this week" |
| Not available (older report) | line absent |

- Plain text, no colour, no success/failure styling: it is a measure, not a score. The planner (Week planner) does not get a line (it is not about the plan).
- Also the same string appears in `orchd intake --week` output (back's concern).

---

## 10. Keys (point 10, page 05)

### 10.1 Ready to start rows (Mission control, pane-local, P4.2 §3.1 conventions: unmodified single keys, no beep on others)

| Key | Action | On |
|---|---|---|
| `j` `k` / ↓ ↑ | Move (lane rows and lines are stops; header is skipped) | any |
| `⏎` | Start the **suggested mode** (opens the prefilled dialog, never starts); on the "+ n more" line opens the source; on a no-source line starts Plan with the Overwatch; on an error line runs Fix (Copy command for `gh_scope`); on "Nothing ready" line: Refresh; on "All n items are hidden · Show" toggles | row · line |
| `t` | Start **team** run (dialog) | Ready row |
| `a` | Start solo (**a**gent session) (dialog) | Ready row |
| `space` | Details | Ready row |
| `o` | Open source (browser / editor) | Ready row |
| `x` · `⌫` | Hide (Unhide on a hidden row) | Ready row |
| `e` | Set up intake (opens Project settings › Intake) | no-source line · error line |
| `⇧R` | Refresh the lane (shown projects) | pane |
| `⇧H` | Show / hide the hidden items (header toggle) | pane |
| `u` | Undo the last Hide while its toast shows (5 s) | pane |
| `Esc` | Close details / popover / dialog | — |

- **`s` is not bound on Ready rows** (it stays Suspend on agent rows; solo is `a`). `p`, `i`, `r`, `⇧K`, `n`, `d`, `h`, `l` do nothing on Ready rows (no beep).
- A Ready row never needs an agent to exist: keys that target an agent ignore it.
- Hint row (toolbar right, only what the row at hand allows, P4.2 §1.9): "⏎ start team run · t team · a solo · space details · o source · x hide".
- Palette entries (⌘K), target in the label: "Start team run: #123 Refunds on partial orders", "Start solo: #123 Refunds on partial orders", "Hide #123", "Open #123 at GitHub", "Refresh Ready to start", "Plan with the Overwatch…" (§8.1), "Set up intake for shop-api". With a Ready row focused the row entries come first.

### 10.2 Brief card (in the chat column, P4.4 §11.2 conventions)
| Key | Action |
|---|---|
| `⏎` | Launch (opens the dialog) |
| `e` | Edit (same dialog, focus in the brief) |
| `r` | Dismiss |
| `j` `k` | Move between entries (stops on the card) |
| `Show more` | `⏎` on the focused clamp toggle, as P4.4 |

Keys are the contract's (§5 `key`). The card's own key labels are shown on the buttons.

### 10.3 Conflict check (as P4.2 §3.2)

| Against | Result |
|---|---|
| **Mission control map** (P4.2 §3.1: `j k h l ⏎ space i p s r ⇧K d n f o ⌫ Esc`) | New: `t`, `a`, `x`, `e`, `⇧R`, `⇧H`, `u` — all **unbound** there. Reused with the same meaning: `⏎` (act on the row), `space` (details), `o` (open), `⌫` (P4.2: cancels a queue entry; on Ready rows it hides: both "take it off this list"). **`s` stays Suspend on agent rows and does nothing on Ready rows** (the brief's `s`… proposal is dropped; solo is `a`). `f` stays the filter: **Fix is on `⏎` of the error line, not `f`**. |
| **`r`** | Resume in Mission control; **Dismiss on the brief card** (chat). Same split as the existing `r` = Resume here / Reject in the Inbox (P4.2 F2): different focus target. No new collision. |
| **`e`** | Mission control: new, "set up intake" (lines only). Chat: Edit on the brief card. P4.4's chat `e` (open in nvim) applies to other kinds; the card shows its own key on its button, as the rule proposal does for `⏎`. |
| **`a` and `t`** | `a` is Approve in the chat / Inbox; here it only fires on a focused Ready row in Mission control. `t` is "Back to today" on a past day in the chat (P4.4 §11.2, P4.3 §6.2) and unbound in Mission control. Different panes, never both focused. |
| **`u`** | Mission control: undo last Hide in its 5 s window. Chat: P4.4 precedence (focused line → card → last decision). No overlap because the focus target decides; the Hide toast's `u` applies only when the pane has focus. |
| **`x`** | Chat: reserved for cards in P4.3 (dismiss). Mission control: unbound before; now Hide. |
| **`⇧R`** | Unbound. `r` (Resume) is without shift; the handler tests `shift+r` before `r`, as `⇧K` before `k` (P4.2 §3.2). |
| **Global ⌘ chords and leader keys** | None added; no ⌘ binding. |
| **Dialogs** | The launch / new-session dialogs take ⌘⏎ / Esc before the pane sees a key (P4.2 §3.2). While one is open from a Ready row, the row's keys do nothing. |
| **Text fields** (Intake settings, input) | Single keys type. `⌘⏎` Save in settings. |

Result: **no binding collision, one reuse split (`r`) already accepted in P4.2.** Page 05 gets the two tables plus the note "`s` = Suspend (agent rows); on Ready rows solo is `a`".

---

## 11. States matrix (point 11) and what ui draws

Page numbers per ux-brief §3. Lane boards at 1512 with the P4.2 fixture plus intake; 1280 variants as noted.

| # | Case | Page | Lane / card |
|---|---|---|---|
| 1 | **Lane, full** (fixture: shop-api GitHub 3 items, blog Linear 2 items, one over room, one with `rough` basis, one hidden, repo-docs project 1 item) | 04.2 | header "Ready to start · 6 items · updated 12 min ago", Refresh, "1 hidden · Show"; rows of each source (glyph, ref, title, project, ≤ 2 labels, estimate, two buttons) |
| 2 | **Row anatomy** closeup | 04.2 | a GitHub, a Linear and a repo-docs row, annotations for each slot; focus ring |
| 3 | **Row details** | 04.2 | §3 board; loaded and "Loading…" and "Shortened" variants |
| 4 | **Over-room row** | 04.2 | line 2 warning, both buttons unchanged |
| 5 | **No source** | 04.2 | one project line; several projects line ("shop-api, blog and infra have no intake source") |
| 6 | **Zero items** | 04.2 | "Nothing ready in acme/shop · label ready · Refresh" and the merged "Nothing ready in 5 projects" |
| 7 | **Source errors** | 04.2 | one board per code of §4.2 (10 lines), plus error header; one with kept items and "updated 3 h ago" |
| 8 | **Refreshing** | 04.2 | with cache; and **first launch** (cache empty) |
| 9 | **Many items** | 04.2 | 50 rows collapsed to a sample + "+ 12 more in acme/shop · Open ↗" |
| 10 | **Hidden shown** | 04.2 | dimmed rows with Unhide; "All 3 items are hidden" |
| 11 | **Collapsed lane**, including with an error | 04.2 | header only |
| 12 | **1280 density** | 04.2 | §2.3 rows; "Estimates are in pp of the week" line |
| 13 | **Origin marks** | 04.2 | run row, solo row, queue row, outcome row with `from #123`, `from ENG-42`, `from brief`; details line; Inspector field (04.2 inspector variants) |
| 14 | **Launch dialog prefilled** (item, over room; stale banner) | 04.4 | "From #123 · GitHub ↗", brief editable, estimate line, untrusted-text note |
| 15 | **New-session dialog prefilled** (item, brief) | 04.4 | chip variants: item, brief |
| 16 | **Brief card** (open, long, over room, off/down footer) | 04.3 | §8.3 |
| 17 | **Brief lifecycle** | 04.3 | every row of §8.4, plus the three collapsed lines |
| 18 | **Planning thread** | 04.3 | input draft "Plan a feature for shop-api: " with the "Planning · shop-api" tag; user entry with tag; Overwatch question; brief card; revised sequence (old collapsed, new open); the "Overwatch is off" variant of the lane line |
| 19 | **Plan popover** (several projects) | 04.2 | §8.1 |
| 20 | **Settings › Intake**: None; GitHub (Label, Milestone, Project); Linear; Repo docs | 04.7 | §7.2, each with Save / Preview / Revert |
| 21 | **Settings › Intake errors**: field errors (`repo`, `names`, `number`, `team`, `glob`), preview ok / zero / error per code (at least `gh_scope` with Copy command, `gh_auth` with Integrations link, `rate_limited`), unsaved changes, saved | 04.7 | §7.2–7.3 |
| 22 | **Global settings block** (Pacing › Intake) with invalid value | 04.7 | §7.4 |
| 23 | **Report line**: every case of §9 | 04.6 | §9 |
| 24 | **Keys** | 05 | §10.1–10.3 tables |
| 25 | **Toasts** | 04.2 | "Hidden #123…" with Undo, "Copied. Run it in a terminal…" |

Sections for ui, in order of dependency: **§1 lane header/grouping → §2 row → §3 details → §4 states/errors → §5 dialogs → §6 origin → §7 settings → §8 plan and card → §9 report → §10 keys → §11 matrix**. Components on page 02 if new: **Source glyph** (GitHub / Linear / Repo docs), **Ready row** (with states), **Lane status line**, **Brief card**, **Origin chip**. Tokens: none new expected (neutral surface, muted text, dashed rule; verify the lane tint against the Board's "Ready" column colour, §0 visual rule).

---

## 12. Copy that changes in the code (for front/back)

- `ProjectSettingsPane.svelte` `SECTIONS`: add Intake after Task backend; `openProjectSettings(project, 'intake')` is **Fix**'s target (settings fix). `fix: "integrations"` opens Settings › Integrations at the GitHub or Linear row according to the code.
- `mission.js`: lane order `needs_you, running, waiting, queued, ready, done` (P4.2 §1.1 + this lane); Ready is excluded from every attention count (header, badge, rail, window title).
- Dialogs: the chip, the estimate line and the untrusted-text note are additions to the existing `launch` / `new-session` components, not new dialogs.
- Strings use the exact copy of §1–§10; where the contract's `message` differs from §4.2, the UI copy wins (F3).
