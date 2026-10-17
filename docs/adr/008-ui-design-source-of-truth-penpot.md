# ADR-008 — UI design source of truth: a Penpot file

- Status: Proposed · Date: 2026-10-17

## Context
The app has many dense, stateful screens (chat, panes, canvas, pacing). Without a shared visual reference, agents and humans would each invent their own UI. The user already uses Penpot and has the Penpot MCP set up.

## Decision
- One Penpot file, **`Orchestrator — UI`**, is the **source of truth for the interface**. Its structure is defined in SPEC §9.0 (Foundations, Components, App shell, Screens, Flows, Explorations).
- **Design tokens** live in Penpot and are exported as JSON, then turned into CSS variables. The code uses tokens only.
- **Design first**: any UI change starts in Penpot and is reviewed before implementation. P1 screens must be designed before P1 UI work starts.
- UI/UX agents work on the file through the **Penpot MCP**, which also lets the project dogfood its own design flow.
- `docs/design/README.md` links to the file and tracks which screens are ready for dev.

## Alternatives
- **Figma**: also supported by the app for user projects, but Penpot is open source, self-hostable and already in the user's setup.
- **Code-first mockups only**: faster at first, but no shared component library and no overview of states.

## Consequences
- A coherent UI and a clear handoff to implementation.
- An extra design step before UI code. Accepted.
