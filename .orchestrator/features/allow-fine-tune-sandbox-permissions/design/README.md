# Allow fine-tune sandbox permissions · design exports

Source: the Drive `Orchestrator.figcraft` (never a repo copy).
- Page **04.7 Settings**, below everything else (y ≈ 23000): title "Allow fine-tune sandbox permissions", Dark row, Light row 3700 below. Frame names start with `SB ·`.
- Page **04.3 Chat column**, y ≈ 19000 (below the Improve Intake row): the `sandbox` card board, Dark then Light to its right.

Copy is the final copy of [../ux-flows.md](../ux-flows.md). Each PNG exists as `-dark.png` and `-light.png`. The Light frames are the Dark frames with the Theme collection set to **Light**. Every colour is bound to a Theme variable.

## Frames

| File | What it shows | ux-flows |
|---|---|---|
| `settings-sandbox` | Settings → Sandbox, loaded: explainer + links, All projects (built-in rows tagged "built in", "Every agent gets these."), orch (role chip, role hosts "built in · back", a removed folder replaced in place by "Removed … · Undo", Local ports row, new socket row highlighted, applies line), shop-api with no file yet ("no policy file yet", Create and open) | §3.1, §3.3, §3.4, §3.5 |
| `settings-sandbox-limits-open` | Same, with "What can't be allowed" expanded | §1 |
| `settings-sandbox-add-form-error` | Inline Add form, folder `~/.ssh` refused by `sandbox_check` (red field + reason, Add disabled) | §3.2 |
| `settings-sandbox-add-form-github` | Add form, host `github.com` OK (not yet in orch), why filled, GitHub login line | §3.2 |
| `settings-sandbox-add-form-states` | Add form: normalised ("Will be saved as …"), OK with a green check, Local ports (no value field), Checking… (alone), OK then refused by the core on Add (race), empty value local message | §3.2 |
| `settings-sandbox-states` | Parse error (last good entries, Remove/Add disabled, "Fix the file first: line 14."), warnings (3 then "Show 1 more", "Edit the file to fix it."), no file yet (Create and open, as in Permissions), applies line variants, All projects with nothing added, no projects, remove failed, duplicate highlighted, loading / load error + Retry / kinds missing | §3.3, §3.4, §3.5 |
| `settings-permissions-rules` | Settings → Permissions: explainer + Sandbox link, Policy files (unchanged apart from the rule counts), **Rules** per file (sentence, effect chip, role chip, note, raw pattern on a second line, id, Remove; removed row with Undo; empty file), **Test a call** with a matched result, then When no rule matches, Hold and the new help line | §4, §4.1, §4.3 |
| `settings-permissions-add-rule` | Add rule form open in the orch block: run a shell command / `cargo test`, preview "This rule matches" `Bash(cargo test:*)` + sentence, Allow, orch, back, tests | §4.2, §4.4 |
| `settings-permissions-rules-states` | Do anything + Allow (amber line, "I understand" unticked, Add rule disabled), preview error, Checking… (alone), refused by the core on Add (`git push` + Allow), rules parse error, neutralised rule warning, loading / load error | §4.1, §4.2 |
| `settings-permissions-test-states` | Matched a rule (r13 is the only `cargo test` rule in the fixture; the rule id is a link), default applies, decided by the core, sandbox hint for a shell command with a URL, error, empty input (Test disabled) | §4.3 |
| `sandbox-card-states` | The `sandbox` card: network (GitHub line), write with the caret menu open, local ports (parked), socket in a solo session with the caret menu open (only "Allow everywhere"), invalid, pending ("Allowing…"), error on Allow, stale; decided lines: allowed ×3 with Undo, already allowed (no Undo), after Undo, removed elsewhere, rejected, replied (clamped), Undo failed | §2 |

## Decisions for front

- **Kind glyph and colour.** The card uses the Material Design glyph **wall** (U+F07FE), like every other decision kind (Material, not Font Awesome), in place of the 🛡 placeholder. It is not a shield: permission (shield-key-outline) and trust (shield-lock-outline) already use shields. Its colour is the new Theme variable **`decision/sandbox`** (`color.decision.sandbox` in `tokens.json`, added in both themes): an alias of `palette/storm/blue1` in Dark and `palette/day/blue1-aa` in Light. That is the same hue as trust on purpose, since both are security calls; the glyph tells them apart, per the separate-palettes rule in docs/design/README.md. The same colour is the card border and the left bar of the Permissions vs Sandbox explainer (a `surface/raised` block).
- **Card actions.** "Allow for this project [a]" is a split button: primary plus a caret segment (`⇧a`). The caret opens a 250 px menu right-aligned under the caret. Each item has a label and a muted help line, and the first item is focused. Reject [r] and Reply [y] are on the next row (ghost buttons; secondary when the value is invalid, since they are then the only actions). The muted footer lines come after them.
- **Decided line.** The card collapses to a 2-line block (`surface/pane`, `border/subtle`): kind · agent · value, then the existing lifecycle marker: `LIFECYCLE.decided` (send-clock U+F1164, `lifecycle/decided`) for every decision line (allowed, already allowed, removed, rejected, replied), text in `text/secondary`. Undo is an accent link. The restart marker is `LIFECYCLE.effect` (check-circle-outline U+F05E1, `lifecycle/effect`). No special case in the marker component.
- **Sandbox rows.** Each row is 30 px: value · chips (`{role} only` accent chip, `built in` outline chip in `text/disabled`) · note (muted, ellipsis) · ghost **Remove** on the right. Built-in rows use `text/muted` for the value and have no Remove. A highlighted row gets an `accent/default` 12 % fill and an accent border for 2 s.
- **Rule rows.** Each row is 46 px: sentence · effect chip (Allow `status/done`, The Overwatch decides `text/muted`, Ask me `decision/ask`, Deny `status/error`; 14 % fill and a solid border) · role chip · note. The raw pattern is on the second line (11 px muted), and the rule id (`text/disabled`) sits before Remove.
- **Inline forms.** Forms use `surface/raised` with an `accent/default` border, labels in a 160–200 px column, and help lines (11–12 px muted) under the fields. Add / Add rule are primary buttons and disabled until valid; Cancel is a ghost button. The Sandbox form repeats the explainer in 11 px under the buttons.
- **Errors and warnings.** A block with a parse error gets a `status/error` border. Warnings use `decision/ask` with ▲.

## Copy notes

- The Rules heading has no subline. The core applies the most restrictive matching rule, not the first one, so a "first match wins" hint would be wrong, and ux-flows gives none.
- No form titles and no "e.g." label: the inline forms open under the block's Add button, and the example chips sit under the kind picker, as in ux-flows §3.2.
- The global file is `<app data>/policy.toml`, shown mid-cut as `~/…/Orchestrator/policy.toml` (as in P4.4).
- ux-flows gives two tooltips for a file with a parse error: "Fix the file first: line {n}." (§3.2, Add) and "Fix the file first." (§3.3, Remove and Add). The frames use the first for Add; ux needs to settle it.

## Tokens used

`surface/chrome`, `surface/pane`, `surface/raised`, `surface/selected`, `border/subtle`, `border/strong`, `text/primary`, `text/secondary`, `text/muted`, `text/disabled`, `accent/default`, `status/done`, `status/error`, `decision/ask`, `decision/permission`, `lifecycle/parked`, `lifecycle/decided`, `lifecycle/effect`, **`decision/sandbox` (new)**.
