# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: none started (design phase)
Plan: not yet written — next step is the `core` implementation plan
Branch: docs/redesign-spec
Last completed task: design spec written
Next task: user reviews spec; then write `docs/superpowers/plans/2026-09-26-core.md`
State: green for new work (no code changed). Note: `cargo test` on `main` is already red —
3 of 58 tests fail in the old code; this is pre-existing.

## Verify before continuing
git status && ls docs/superpowers/specs docs/superpowers/plans 2>/dev/null

## Notes / decisions made during work
- Move generation: own bitboard rewrite (not `shakmaty`, not fixing the old design).
- Jev role: hybrid — code annotates and shortlists, Jev picks via one `choice` question, code
  vetoes blunders and plays forced moves / mate-in-1 directly.
- Shortlist default on; configurable via `JEV_MAX_OPTIONS` (default 40) and
  `JEV_FILTER_LOSING` (default true).
- TUI: `ratatui` + `crossterm`, full scope (menu/modes, mouse + keyboard + command box, side
  panels, undo/flip/new/FEN/PGN).
- No LLM code exists in the repo; "remove LLM code" means deleting the old greedy `ai.rs`.
- `JEV_API_KEY` is present in the user's environment.

## Open questions for user
- None.

## Log (newest first)
- 2026-09-26 brainstorming complete; spec and handoff file written.
