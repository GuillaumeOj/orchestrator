# U1 visual QA (ui, T-367) — must-fix list for F7

U1 / T-367 visual QA: every §13 board compared with the ii-*.png exports at 1512 and 1280. I couldn't launch the Tauri app or run vitest (the worktree has no node_modules), so this comes from reading the code, CSS and tests. I spot-checked the main claims in code. Paths are under ui/src/.

Two items are **pending F6** (front is finishing them): ProgramPane:253 "(sub-issue of #12, added 13 Nov)", and the plan dialog's closed issue having no "Include anyway" fix (program.js issueFixes has no 'closed' case). The other F6 items didn't show up as differences here: slug (the Program view row editor has no Slug field, which matches §7.2) and the search error line. Please re-check those two against the boards once F6 merges.

MUST FIX (front)
Work browser
1. components/work/WorkRow.svelte:285: `.right` width is 200px; the README says ~270 at 1512, so the long running chip gets cut. Set 270px and keep .narrow at 160.
2. WorkRow:43: the per-row "+n" label collapse at 1512 is missing (title would keep <24 chars → labels become "+n" and the dependency becomes its glyph; #13 shows "+2"). Measure the title room.
3. WorkRow:279-282: the closed-upstream `.mark` is amber; §2.5 says text.muted. Take .mark out of the warn rule.
4. work/WorkDetails.svelte:98: dependency lines use the short chip at 1512. Use `{short: narrow}`.
5. lib/workmodel.js:727: the container children heading reads "Issues · 0 of 2 done"; the boards read "Issues · 2 issues · 0 done · 1 running". Update work-model.test.
6. Deleted upstream (S14): should offer Show feature only. Fix in three places:
   - hintsFor (workmodel.js:323-351) should return only "g show feature";
   - the details buttons and menu (WorkDetails:77-90) should hold only Show feature;
   - WorkPane:563-570 should block o, x and z.
7. WorkPane ~706: the filtered count line "N items match · M filters" (§4.2) is missing.
8. S8: ⏎ does nothing with no rows. When noRoots and narrowed, call view.resetFilters().
9. S7 (workmodel.js:669 / WorkPane:703): "GitHub ↗" should be a link to the repo's issues.
10. workmodel.js:164: the queued chip tooltip is missing " (live cap 2/2)".
11. Hint row:
    - with 1 item selected, show only "x select · Esc clear";
    - in focus mode, keep "⏎ details";
    - search with no results should show "Esc clear search" (workmodel.js:295-302, 327).
12. WorkDetails:104: Linear children should show their upstream state ("Done"/"Todo") instead of "—"; container children should show their progress.
13. WorkDetails:71-75: order Start solo and Start team run by the suggested mode (intake.js modeOf), primary first.

Mission control / programs
14. mission/ReadyRow.svelte:31: "Hide" with key x should be "Hide from Ready" with ⌫, as in §5.3 and Work.
15. mission/ProgramRow.svelte: no 1280 rule. Add a container query for "0/3 merged · 1 running · 2 more".
16. lib/mission.js:264: the Programs lane header draws ▲ (running); the boards draw the program glyph U+F0328. Keep the running colour.
17. panes/ProgramPane.svelte:208: the sub-line drops the repo (`where:''`) and puts ↗ after the title. It should read "From #12 Checkout v2 · acme/shop ↗ · started …". Update the program.test.js:765 assertion.
18. ProgramPlanDialog.svelte:68-73: the From line adds "· in v2" for a root under a milestone. "in …" belongs only when the parent is an issue. Update program.test.js:449.
19. ProgramPlanDialog:444: "Start program · 0 features" shows while the first plan is loading. Show "Start program" while !draft.
20. lib/program.js:39-60: a feature that depends on a left-out running item reads "first"; the board reads "after #13 merges". This depends on program_plan sending those edges; please check with back.

Feature pane / dialogs / palette
21. feature/TasksSection.svelte:230 and Menu/popover.js: the task menu flips against the window, not the Tasks band, so at 1280 it covers the hint row. Add a bounds prop and pass the band.
22. TasksSection:105-116: the menu has no "Move left / right ⇧H ⇧L" item.
23. panes/FeaturePane.svelte: the hint row is missing. Its content depends on focus:
    - Tasks focused: j k h l · ⏎ menu · a · m · 1–5 · e · n · o open #70 · ⇧H ⇧L · Esc;
    - otherwise: b tasks · w show in Work · e · m · p.
    Use work/HintRow.
24. Dialogs.svelte:366/424: the blocked, "On start:", ran-before, untrusted and estimate lines should sit just above the buttons (§5.2). The sub-issues line stays under the brief.
25. Dialogs.svelte:31: the prefilled Launch / New session dialogs are 560 wide; the boards use 600. Add a 600 size for those two only.
26. feature/SourceHeader.svelte:55: "· closed in GitHub" must be the tail, after the write-back text.
27. TasksSection:310: card titles wrap; they should end in an ellipsis with a tooltip ("Address form compo…" at 1280).
28. Palette, lib/app.svelte.js:
    - :2089: "Work browser" should read "Open Work (shop-api)" for the context project (§1.3);
    - :2172: add "Start team run: #13 Address step" next to "Run as program: …" (§12.7).

Tracker settings
29. lib/tracker.js:246: errorInfo has no label, so the copy reads "the repository not found…". Separately, TrackerSettings.svelte:646 renders `fix:'settings'` as a "Retry" link that does nothing (not_found, bad_glob, branch_missing). Show the field error under Repository or Glob with a red border and focus that field.
30. mission/ReadyLine.svelte:11: uses errorInfo instead of trackerError, so the §10.7 copy for linear_complexity, gh_write, linear_write and gh_scope is missing. Map limit_projects and turn_off_write_back to "Fix" (open Tracker).
31. TrackerSettings:651: the Linear preview count reads "in Linear ENG"; it should read "2 ready in ENG" (trackerTarget).
32. TrackerSettings:659-663 and 970: sample rows wrap. Make them nowrap with an ellipsis, and drop the "where" column at the narrow width.
33. Hint colour: the global `.hint` (styles/app.css:367) beats `.muted`, so hints show text-secondary and the two Advanced hints sit on one line. In TrackerSettings:723, set `.hint{display:block;color:var(--color-text-muted)}`.
34. TrackerSettings:381: the linear_complexity Fix sets projects without edit(), so a stale error shows at once. Call edit().
35. lib/tracker.js:229-241: Linear sample "where" has an extra "in "; drop it for Linear.

ACCEPTABLE
- Key hints are in tooltips, not key caps inside buttons (Work, Refresh, Filters, Start program ⌘⏎, Save, Show in Work). This follows the app convention: Kbd is only in menus, the sheet and hint rows.
- The progress bar has one segment per non-skipped feature, not a fixed three. This is identical for 3 features and still accurate beyond; I'll reword the README note if you agree.
- Only the tail of a flagged sub-row is amber, as §7.1 says.
- Cycle popover is Cancel then Apply, the app's order; the copy matches.
- "Plan failed" on a first plan has no totals, because there's no draft to compute them from.
- These differences predate this feature and are out of scope:
  - the MC subtitle;
  - the extra Ready menu items (Details, Open source);
  - the dialog note boxes, the pill origin chip and the other dialog fields;
  - the Project settings floating window at 960 (vs a tab) and the nav label "MCP overrides".
- "Run as program instead" shows in New session too, as §5.2 asks wherever the line shows.
- The source header glyph is fixed per tracker; it matches every drawn variant.
- The selected task card has a ⋯ trigger, a mouse affordance.
- Work:
  - the project row has no band fill (polish);
  - "r refresh" shows only per the focused-row rule;
  - "⌫ hide from Ready" is longer copy;
  - the Linear Sub-issues heading has no count, per the spec;
  - glyph 12 in a 16 box;
  - the full placeholder;
  - ⌄ › disclosure;
  - 3 skeleton rows;
  - an extra Refresh on S3;
  - the S4/S18 error tail position.
- Tracker polish (fine to leave or take later):
  - segmented controls have no dividers or accent outline on the selected segment;
  - write-back indent is 24 instead of 46;
  - list bullets are "•", not "·";
  - Advanced state selects hug instead of 180px;
  - the projects tag box is full width instead of ~360.
- Tracker, other:
  - gh_write under the switch: the API has no field for it, and failing writes show through the decision card and Work S15;
  - the gh_scope toast says "Preview again" because Settings has no Refresh;
  - Revert is disabled rather than hidden, and Save ⌘⏎ is in a tooltip (SettingsFooter convention);
  - "2 files match" follows P4.5;
  - Advanced is folded, per §10.2;
  - the red tag box on a teams error;
  - the local ref column is 160.
- Side note: the `@container (max-width:900px)` rules in TasksSection:330 and FeaturePane:646 never apply, because no ancestor sets container-type. Today that matches the 1280 board; it's a latent trap.

docs/design/README.md: the improve-intake rows (Cover, 02, 04.2, 04.3, 04.4, 04.7, 04.8, 05) and the design-pass paragraph now say **Implemented**. Committed as c91d98d3.
