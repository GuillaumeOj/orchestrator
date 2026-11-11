# Fix Worktrees Deletion, Round 2 · T-0 audit

> All file:line below are on `origin/main` @ 95d930f (read with `git show origin/main:<path>`), not on the integration branch (still at c13b52b). Nothing was run: the sandbox worktree has no `ui/node_modules`, so verdicts are from reading the code.

## 1. Every `confirm(` call site in `ui/src`

`confirm(text, detail, onYes, yes, no)` is defined at `ui/src/lib/app.svelte.js:1560`; it only sets `app.overlay = { kind: 'confirm', … }`. `Dialogs.svelte` (lines 483–484) is the one place that runs `onYes`, so all 20 call sites share the one bug. No callback reads `app.overlay` or `app.window` itself: they close over their own arguments and component state, so **none needs a change of its own** once the dialog reads `onYes` before `close()`.

| # | file:line | confirms | callback runs (the invoke) |
|---|---|---|---|
| 1 | `components/IntegrationsSettings.svelte:82` | Remove the &lt;integration&gt; token from the Keychain | `integration_secret_remove { id }`, then toast |
| 2 | `components/McpSettings.svelte:167` (`discardFirst`) | Discard unsaved changes before Override / Inherit reloads the form | the passed `run`: for Override (`setOverride`, l.173) `mcp_server_save` (project scope); for Inherit (l.206) `mcp_server_delete` |
| 3 | `components/McpSettings.svelte:207` (`setInherit`) | Use the global server in &lt;project&gt; (its own copy is deleted) | `run`: `mcp_server_delete { id, scope: project }`, then `load()` |
| 4 | `components/McpSettings.svelte:213` (`remove`) | Delete a server from the MCP registry | `mcp_server_delete { id, scope }` |
| 5 | `components/McpSettings.svelte:274` (`removeSecret`) | Remove a Keychain value | `mcp_secret_delete { account }` |
| 6 | `components/PlannerWindow.svelte:98` (`setEnforce`) | Enforce the Governor / back to shadow mode | `governor_enforce { enforce }` |
| 7 | `components/PlannerWindow.svelte:159` (`sprint`) | Sprint mode (borrows 5 pp) | `governor('sprint', 5)` → `governor_act` |
| 8 | `components/chat/ChatColumn.svelte:743` (`act`, id `delete`) | Delete the agent of a decision card | `send(d, 'delete')` → `decide` → `inbox_act` after the 5 s undo window (`flushDecisions`, `app.svelte.js:2329`) |
| 9 | `lib/app.svelte.js:811` (`unlessDirty`) | Discard changes of a dirty settings window (close, ⌘W, replace, switch section) | no invoke: `windowGuard.onDiscard?.()`, then the pending `then()` (`closeWindow`'s `close`, `setWindow`'s `apply`) |
| 10 | `lib/app.svelte.js:1386` (`confirmKill`) | Kill an agent (Menu, palette) | `agentAction(id, 'kill')` → `agent_action { id, action: 'kill' }` |
| 11 | `lib/app.svelte.js:1395` (`confirmRestartFresh`) | Restart an agent fresh | `restartFresh(id)` → `restart_fresh { agent }` |
| 12 | `lib/app.svelte.js:1472` (`runNow`) | Run a queued turn now / start a not-started agent | `queueAct('run_now', …)` → `governor_act { action: 'run_now', arg: entry }` |
| 13 | `lib/app.svelte.js:1533` (`cancelQueued`) | Remove an agent's queue entry | `queueAct('cancel', …)` → `governor_act { action: 'cancel', arg: entry }` |
| 14 | `lib/app.svelte.js:2282` (`decide`, action `always`) | Always allow a rule (writes the project's policy) | pushes onto `app.pending` and `flushDecisions()` → `inbox_act` (no undo window) |
| 15 | `panes/DiffPane.svelte:67` (`submitPr`) | Push the branch and open a PR | `worktree_create_pr { project, path, title, body, draft }` |
| 16 | `panes/FeaturePane.svelte:174` (`stop`) | Stop the team on a feature | `act('stop')` → `run_act { run, action: 'stop' }` |
| 17 | `panes/MissionPane.svelte:155` (`kill`) | Kill an agent from Mission control | `act(id, 'kill')` → `agentAction` → `agent_action` kill |
| 18 | `panes/ProjectPane.svelte:103` (`removeWorktree`, merged worktree) | **Remove worktree** (the reported bug) | `run(false)` → `worktree_remove { project, path, deleteBranch: true, discard: false }`, row filtered out, `Removed …` toast |
| 19 | `panes/ProjectPane.svelte:104` (`removeWorktree`, unmerged) | **Discard unmerged work** | `run(true)` → `worktree_remove { …, discard: true }` |
| 20 | `panes/ProjectPane.svelte:119` (`doRemove`) | Remove the project | `project_remove { id, choices }`, then `closePane` |

`git grep -n 'confirm(' origin/main -- ui/src` gives these 20 call sites plus the definition. Rows 2 and 9 are reached from several paths (Override / Inherit; close, ⌘W, replace, switch section).

The broken line, for the record: `components/Dialogs.svelte:483` (the ⏎ key) and `:484` (the form submit), both `close(), o.onYes()`.

## 2. Handlers that clear a `$state` and then read a value derived from it

`o = $derived(app.overlay)` (`Dialogs.svelte:28`); `SettingsWindow.svelte:38` and `ProjectSettingsWindow.svelte:43` have `spec = $derived(app.window)`; `PlanChooser.svelte:7` has `ids = $derived(app.overlay?.ids ?? [])`. A derived read after its source was cleared recomputes to the cleared value.

### Broken

| file:line | handler | why |
|---|---|---|
| `components/Dialogs.svelte:483` | confirm branch, `onkeydown` → `onKey(e, () => (close(), o.onYes()))` | `close()` sets `app.overlay = null`; `o` is then `null`; `o.onYes` throws `TypeError` and the action never runs. |
| `components/Dialogs.svelte:484` | confirm branch, `onsubmit` → `(e.preventDefault(), close(), o.onYes())` | same. This is the reported bug. |
| `components/Dialogs.svelte:292` | origin-chip link, `onclick={() => (close(), focusCard(chip.decision))}` | **Probable, not reproduced.** `chip` is `{@const chip = originChip(o.origin)}` (l.291), which Svelte compiles to a derived over `o`. After `close()` the derived is dirty and reading `chip.decision` recomputes `originChip(null.origin)`, a `TypeError`. Same shape as the confirm bug, so `front` should cover it with a test through the real dialog (Launch / New session opened from a brief card, click the `↗` "Show the brief card" button, expect `focusCard` called). Fix: read `const decision = chip.decision` before `close()`. |

### Safe

| file:line | handler | why it is safe |
|---|---|---|
| `components/Dialogs.svelte:80` | `$effect` for `launch` | `const ov = o;` once, then only `ov` (the safe pattern). |
| `components/Dialogs.svelte:134–146` | `$effect` for the delete dialogs | `const ov = o;` and `if (app.overlay !== ov) return` in the `.then`. |
| `components/Dialogs.svelte:225–235` | `close()` | reads `o` **before** clearing (`prefilledSession`, l.227, with the comment). |
| `components/Dialogs.svelte:112–127` | `launchTeamOnFeature` | `o.intake` is read in the invoke arguments, before `close()` (l.120). After it: `launchTeam`, `launchSlug` derive from `teamsView` / `launch`, not from the overlay. `failed()` uses `o?.intake`, null-safe after a cancel. |
| `components/Dialogs.svelte:245–260` | `startSession` | `o.intake` read before the await and before `close()` (l.252). After it: `project` derives from `form.project_id`. |
| `components/Dialogs.svelte:262–279` | `addProject` | no overlay-derived read; `close()` then `p.id`, `p.name` (the invoke result). |
| `components/Dialogs.svelte:153–166` | `runDelete` | `const ov = app.overlay` (the state, not the derived) before the await; `if (app.overlay === ov) close()`; the error branch uses the captured `what`. |
| `components/Dialogs.svelte:169` | `confirmDeleteAgent` | `o.agentId` is read inside `fn()`, which runs synchronously before any `close()`. |
| `components/Dialogs.svelte:171–174` | `delFeature`, `confirmDeleteFeature` | `delFeature` is `o?.kind === … ? … : null` (null-safe: it reads `null` after a close); `confirmDeleteFeature` reads `delFeature?.slug ?? o.featureId` and `o.featureId` before `runDelete` closes. |
| `components/Dialogs.svelte:176–215` | the `new-session` `$effect`s | guarded with `o?.kind`; `o.focus` inside a `queueMicrotask` (l.203) reads while the overlay is open (the dialog was just opened by the prefill); a cancel inside the same microtask is not realistic. |
| `components/Dialogs.svelte:282–285` | `onKey` | takes `submit` as an argument; the unsafe part is the callers' lambda (l.483), not `onKey`. |
| `components/Dialogs.svelte` snippets and templates | `{#if o…}` blocks | the template re-renders from `o`; a read in a render is fine, only a read in an event handler after `close()` is at risk (see l.292). |
| `components/SettingsWindow.svelte:38–105` | `spec` | only read in the template and `section`; never read in a handler after a close. `onclose={onWindowClose}` → `closeWindow` reads `app.windowReturn` into `el` **before** `app.window = null` (`app.svelte.js:856–858`). |
| `components/ProjectSettingsWindow.svelte:43–48, 267, 364` | `spec`, `pid` | `pid = untrack(() => spec?.projectId)` captures the id once (l.45); the rest is template reads. |
| `components/FloatingWindow.svelte:111–120, 158, 174` | `onWindowKey`, backdrop, × | only calls `onclose(reason)`; reads nothing derived after it. |
| `lib/app.svelte.js:796–806` | `backdropClose`, `onWindowClose` | read `windowDirty()` (a module variable) before `closeWindow()`. |
| `lib/app.svelte.js:828–839, 852–863` | `setWindow`, `closeWindow` | read `app.windowReturn` / `app.focused` before or independent of the clear; `restoreFocus(el, app.focused)` passes values. |
| `lib/app.svelte.js:809–823` | `unlessDirty` callback | reads the module variable `windowGuard`, not a derived; it is the callback that today never runs (item 9 above). |
| `lib/app.svelte.js:1765` | `prunePanes` | `app.window.projectId` read in the same condition, before `closeWindow(true)`. |
| `components/PlanChooser.svelte:12–18, 29` | `close`, `choose(ids[i])` | `ids[i]` is evaluated as an argument first; `choose` → `planWithOverwatch(id)` (`intake.svelte.js:170–179`) sets `app.overlay = null` and then reads only `app.chatDraft` and the `projects()` result, none derived from the overlay. |
| `components/Palette.svelte:21–28` | `choose(item)` | `close(); item?.run()`: `item` is a plain object passed by the caller, not read back through `app.overlay`. `items` / `scoped` derive from `query` / `mode` props, not the overlay. |
| `components/Menu.svelte:84–92, 121–126` | `close`, `run(item)` | `close(true, false)` then `item.run?.()`: `item` is a parameter; `open` / `sub` are local state; no overlay read. This matters because the Menu is what opens most confirms. |
| `App.svelte:75–81` | chord handler | `app.overlay = null` then `run(r.command)`: `r.command` is read from the resolved key result, not from the overlay. |
| `components/PlannerWindow.svelte:189, 219` | `app.overlay = { kind: 'plan' }` | sets, never clears then reads. |
| `lib/intake.svelte.js:51` | `intakeUi.starting` reset | reads `app.overlay` live inside an effect (not a derived copy). |
| `panes/*`, `components/chat/*`, `McpSettings`, `IntegrationsSettings` callbacks (table 1) | the `onYes` lambdas | they read their own `$state` and arguments, not `app.overlay` / `app.window`. Component state they read (`p`, `d`, `removal`, `spec` of a **pane**, `t.cards`) is not what `close()` clears. |

### Not found

No other handler in `ui/src` clears `app.overlay`, `app.window` or another `$state` and then reads a value derived from it. `git grep -nE 'app\.(overlay|window)\s*=' origin/main -- ui/src` lists every writer; the only ones that clear and keep going are `Palette.svelte:22`, `PlanChooser.svelte:12`, `App.svelte:78`, `intake.svelte.js:173`, `app.svelte.js:857` (all safe above) and `Dialogs.svelte`'s `close()`.

## 3. Tests that call `app.overlay.onYes()` directly (they bypass the dialog)

16 call sites in `ui/test` on `origin/main` (`git grep -n 'onYes' origin/main -- ui/test`, nothing else uses `onYes`):

| file:line | test |
|---|---|
| `chat.test.js:162`, `:167` | keyless actions: Always allow decides, Answer in the pane decides nothing, Delete asks first (it starts at l.152) |
| `decisions.test.js:388` | Always allow names the rule; a failure stays until dismissed and opens the decision (l.375) |
| `ecosystem.test.js:360` | MCP servers: a Keychain env value needs the opt-in; a used server is not deleted (l.309) |
| `ecosystem.test.js:864` | MCP env Keychain values: status from the core, stored with `mcp_secret_set`, removed with `mcp_secret_delete` (l.833) |
| `mission-pane.test.js:243` | `n` on a queue row confirms with the cost in both windows (FR-809) (l.230) |
| `projects.test.js:209` | shows the git state, trust, worktrees and keeps unmerged work behind its own confirmation (l.184) |
| `projects.test.js:239` | a stale `project_detail` resolving after a removal does not bring the row back (l.222) |
| `projects.test.js:254` | the row disappears right after a successful removal, before the reload answers (l.247) |
| `projects.test.js:265` | a refused removal keeps the row and shows the core error (l.259) |
| `projects.test.js:307` | J11: archive by default, unmerged worktrees kept unless ticked (l.296) |
| `projects.test.js:437` | offers to enforce once the shadow week is over, and to go back (l.430) |
| `settings-sections.test.js:254` | Inherit asks first when the project copy has its own env (l.249) |
| `windows.test.js:304` | closing, ⌘W or replacing a dirty window asks; Keep editing keeps it; Discard goes on (l.287) |
| `windows.test.js:313` | switching section asks; a clean window never asks; a server change in place never asks (l.308) |
| `windows.test.js:341` | Project settings: a backdrop click on unsaved Notion flashes the footer; Esc and × ask (l.323) |

Tests that render `Dialogs` (`delete.test.js:59`, `ecosystem.test.js:666, 906, 916`, `intake-lane.test.js:751, 781, 808`) cover the delete, add-project / launch / new-session dialogs, not the generic confirm branch; none of them submits a confirm.

## Summary for T-1

- Fix `Dialogs.svelte:483–484` (spec U1) and `:292` (the chip link, U2). Nothing else needs a code change.
- Add a test through the real dialog for the chip link; the spec's U3 list covers the confirm branch.
