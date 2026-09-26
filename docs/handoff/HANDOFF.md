# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: core | Plan: docs/superpowers/plans/2026-09-26-core.md
Branch: feat/core-bitboards
Last completed task: 5 (legal move generation and perft)
Next task: 6 (SAN and UCI parsing)
State: green

## Verify before continuing
cargo test --lib core:: && cargo test --release --lib core::perft -- --ignored

## Notes / decisions made during work
- movegen: pawn_moves takes a generic closure (impl Fn) instead of the plan's &dyn Fn, because the spec forbids trait objects in core.
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
- 2026-09-26 core task 5 done: legal move generation and perft
- 2026-09-26 core task 4 done: position, FEN, hashing, play
- 2026-09-26 core task 3 done: move type
- 2026-09-26 core task 2 done: attack tables
- 2026-09-26 core task 1 done: foundation types
- 2026-09-26 core implementation plan written (8 tasks + setup), verified by replay.
- 2026-09-26 brainstorming complete; spec and handoff file written.
