# Allow fine-tune sandbox permissions · design exports

Source: the Drive `Orchestrator.figcraft` (never a repo copy).
- Page **04.7 Settings**, below everything else (y ≈ 23000): title "Allow fine-tune sandbox permissions", Dark row, Light row 3700 below. Frame names start with `SB ·`.
- Page **04.5 Inbox**, y ≈ 1100: the `sandbox` card board, Dark then Light to its right.

Copy is the final copy of [../ux-flows.md](../ux-flows.md). Each PNG exists as `-dark.png` and `-light.png`. The Light frames are the Dark frames with the Theme collection set to **Light**. Every colour is bound to a Theme variable.

## Frames

| File | What it shows | ux-flows |
|---|---|---|
| `settings-sandbox` | Settings → Sandbox, loaded: explainer + links, All projects (built-in rows tagged "built in", "Every agent gets these."), orch (role chip, role hosts "built in · back", Local ports row, new row highlighted, "Removed … · Undo" row, applies line), shop-api empty | §3.1, §3.3, §3.4, §3.5 |
| `settings-sandbox-limits-open` | Same, with "What can't be allowed" expanded | §1 |
| `settings-sandbox-add-form-error` | Inline Add form, folder `~/.ssh` refused by `sandbox_check` (red field + reason, Add disabled) | §3.2 |
| `settings-sandbox-add-form-github` | Add form, host `api.github.com` OK, role back, why filled, GitHub login line | §3.2 |
| `settings-sandbox-add-form-states` | Add form: normalised ("Will be saved as …"), OK with a green check, Local ports (no value field), Checking… and refused after the check (race), empty value local message | §3.2 |
| `settings-sandbox-states` | Parse error (last good entries, Remove/Add disabled, "Fix the file first: line 14."), warnings (3 then "Show 1 more", "Edit the file to fix it."), no file yet, applies line variants, All projects with nothing added, no projects, remove failed, duplicate highlighted, loading / load error + Retry / kinds missing | §3.3, §3.4, §3.5 |
| `settings-permissions-rules` | Settings → Permissions: explainer + Sandbox link, Policy files, **Rules** per file (sentence, effect chip, role chip, note, raw pattern on a second line, id, Remove; removed row with Undo; empty file), **Test a call** with a matched result, then When no rule matches, Hold and the new help line | §4, §4.1, §4.3 |
| `settings-permissions-add-rule` | Add rule form open in the orch block: run a shell command / `cargo test`, preview "This rule matches" `Bash(cargo test:*)` + sentence, Allow, orch, back, tests | §4.2, §4.4 |
| `settings-permissions-rules-states` | Do anything + Allow (amber line, "I understand" unticked, Add rule disabled), preview error, Checking… then refused by the core, rules parse error, neutralised rule warning, loading / load error | §4.1, §4.2 |
| `settings-permissions-test-states` | Matched a rule (rule id is a link), default applies, decided by the core, sandbox hint for a shell command with a URL, error, empty input (Test disabled) | §4.3 |
| `sandbox-card-states` | The `sandbox` card: network (GitHub line), write with the caret menu open, local ports (parked), socket (solo, so no role item), invalid, pending ("Allowing…"), error on Allow, stale; decided lines: allowed ×3 with Undo, already allowed (no Undo), after Undo, removed elsewhere, rejected, replied (clamped), Undo failed | §2 |

## Decisions for front

- **Kind glyph and colour.** The card uses the Nerd Font shield `` (U+F132), in place of the 🛡 placeholder. Its colour is the new Theme variable **`color/decision/sandbox`**: an alias of `palette/storm/blue1` (#2AC3DE) in Dark and of `palette/day/blue1-aa` (#157180) in Light. The same colour is the card border and the left bar of the Permissions vs Sandbox explainer (a `surface/raised` block). The Settings nav shows a small "new" tag next to Sandbox for discovery. It is optional and can be dropped.
- **Card actions.** "Allow for this project [a]" is a split button: primary plus a caret segment (`⇧a`). The caret opens a 250 px menu right-aligned under the caret. Each item has a label and a muted help line, and the first item is focused. Reject [r] and Reply [y] are on the next row (ghost buttons; secondary when the value is invalid, since they are then the only actions). The muted footer lines come after them.
- **Decided line.** The card collapses to a 2-line block (`surface/pane`, `border/subtle`): kind · agent · value, then the marker in `lifecycle/sent` with "✓", or `text/muted` with "•" for Rejected and Replied. Undo is an accent link. The restart marker uses `status/done`.
- **Sandbox rows.** Each row is 30 px: value · chips (`{role} only` accent chip, `built in` outline chip in `text/disabled`) · note (muted, ellipsis) · ghost **Remove** on the right. Built-in rows use `text/muted` for the value and have no Remove. A highlighted row gets an `accent/default` 12 % fill and an accent border for 2 s.
- **Rule rows.** Each row is 46 px: sentence · effect chip (Allow `status/done`, The Overwatch decides `text/muted`, Ask me `decision/ask`, Deny `status/error`; 14 % fill and a solid border) · role chip · note. The raw pattern is on the second line (11 px muted), and the rule id (`text/disabled`) sits before Remove.
- **Inline forms.** Forms use `surface/raised` with an `accent/default` border, labels in a 160–200 px column, and help lines (11–12 px muted) under the fields. Add / Add rule are primary buttons and disabled until valid; Cancel is a ghost button. The Sandbox form repeats the explainer in 11 px under the buttons.
- **Errors and warnings.** A block with a parse error gets a `status/error` border. Warnings use `decision/ask` with ▲.

## Tokens used

`surface/chrome`, `surface/pane`, `surface/raised`, `surface/selected`, `border/subtle`, `border/strong`, `text/primary`, `text/secondary`, `text/muted`, `text/disabled`, `accent/default`, `status/done`, `status/error`, `decision/ask`, `decision/permission`, `lifecycle/parked`, `lifecycle/sent`, **`decision/sandbox` (new)**.
