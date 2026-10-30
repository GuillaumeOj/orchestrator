# ADR-012 — UI design source of truth: a FigCraft file

- Status: Accepted · Date: 2026-10-30 · Supersedes: [ADR-008](008-ui-design-source-of-truth-penpot.md)

## Context
ADR-008 made a Penpot file the source of truth for the UI. The design has moved to FigCraft: the file lives in the repository, so the design and the code change in the same pull request, and it uses real components, variants, instances and variables.

## Decision
- One FigCraft file, [`docs/design/Orchestrator.figcraft`](../design/Orchestrator.figcraft), stored with Git LFS, is the source of truth for the interface. Its structure is in SPEC §9.0 and `docs/design/README.md`.
- **Components** are real FigCraft components on `02 Components`; **states are variants** (component sets). Screens on pages 03 to 06 use **instances** of them, never redrawn copies.
- **Tokens** are FigCraft variables (Primitives and Theme with Dark and Light modes). The tokens pipeline of ADR-008 is unchanged in shape: variable export → W3C design-token JSON → Style Dictionary → CSS variables, with a CI drift check.
- **Design first for new screens.** Small changes can be made in code and backported to the file in the same PR.
- UI/UX agents can work on the file through the FigCraft MCP.
- The gate of ADR-008 (Foundations, Shell & layouts and the P1 panes; the P4 gate in SPEC §9.0) is unchanged.

## Consequences
- The design is versioned with the code; the binary file goes through Git LFS, so contributors need `git lfs`.
- The file is a binary, so a design change is reviewed in the app, not in the diff.
- Not carried over from Penpot: text and effect styles for the typography and shadow tokens, mixed styling inside one text layer, and width and height variable bindings.
- Penpot and Figma stay supported as integrations for user projects (SPEC FR-601, FR-1203).
