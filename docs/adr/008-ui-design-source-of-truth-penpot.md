# ADR-008 — UI design source of truth: a Penpot file

- Status: Superseded by [ADR-012](012-ui-design-source-of-truth-figcraft.md) (the design moved from Penpot to FigCraft) · Date: 2026-10-17 · Revised: 2026-10-17 (lighter gate after review)

## Context
The app has dense, stateful panes (agents, canvas, pacing). Without a shared visual reference, the UI would drift. The user uses Penpot and has the Penpot MCP set up. The process must stay proportionate for one person.

## Decision
- One Penpot file, **`Orchestrator — UI`**, is the source of truth for the interface. Its structure is in SPEC §9.0.
- **States are component variants**, not separate screens. Storyboards (annotated frame sequences, no clickable prototypes) only for J2, J5 and J8, with key sequences annotated.
- **Tokens pipeline**: Penpot tokens → W3C design-token JSON → Style Dictionary → CSS variables, with a CI drift check. Code uses tokens only.
- **Design first for new screens.** Small changes can be made in code and **backported to Penpot in the same PR**.
- **Gate**: Foundations, Shell & layouts and the P1 panes (Overview, agent pane, Week planner, Inbox) before P1 UI work.
- UI/UX agents can work on the file through the Penpot MCP.

## Alternatives
- **Figma**: supported for user projects, but Penpot is open source and already in the user's setup.
- **Code-first only**: faster at first, but no shared component library or overview of states.

## Consequences
- A coherent UI with a clear handoff, at the cost of a short design step that is accepted.
