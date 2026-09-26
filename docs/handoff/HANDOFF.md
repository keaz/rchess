# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: core | Plan: docs/superpowers/plans/2026-09-26-core.md
Branch: docs/redesign-spec (plan committed; implementation branch feat/core-bitboards not yet created)
Last completed task: core plan written and verified (all plan code replayed in a scratch copy: every task compiles and passes)
Next task: core Task 0 (branch setup), then Task 1
State: green for new work (no code changed yet). Note: `cargo test` on `main` is already red —
3 of 58 tests fail in the old code; this is pre-existing.

## Verify before continuing
git status && ls docs/superpowers/specs docs/superpowers/plans

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
- Core plan adds `error.rs`, `zobrist.rs` and `san.rs` beyond the spec's file list (split for focus;
  the public API matches spec section 4.3; `Game::history()` is named `Game::moves()`).
- Slider magic numbers are hardcoded in the plan (generated offline with a seeded search) and
  proven by an exhaustive subset test; do not regenerate them.
- Reference performance from the prototype: full published perft suite (~594M leaf nodes)
  in 1.6 s release; criterion `startpos depth 5` about 15 ms.

## Open questions for user
- None.

## Log (newest first)
- 2026-09-26 core implementation plan written (8 tasks + setup), verified by replay.
- 2026-09-26 brainstorming complete; spec and handoff file written.
