# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: engine (not started) | Plan: to be written — docs/superpowers/plans/2026-09-26-engine.md
Branch: feat/core-bitboards (complete, ready for user to merge)
Last completed task: core task 8 (property tests, benchmark, wrap-up) — core sub-project DONE
Next task: user merges feat/core-bitboards; then write the engine plan with superpowers:writing-plans from spec section 5
State: green (except 3 pre-existing old-code test failures)

## Verify before continuing
cargo test --lib core:: && cargo test --test core_properties && cargo test --release --lib core::perft -- --ignored

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
- Game::undo withdraws a pending resignation first (returns None, keeps moves); a second undo takes back the move. Deviation from the plan's code, by controller ruling.
- Benchmark results (criterion, 10 samples): startpos depth 5 [14.654 ms 14.811 ms 15.283 ms], kiwipete depth 4 [9.9514 ms 9.9703 ms 9.9899 ms].

## Open questions for user
- None.

## Log (newest first)
- 2026-09-26 core sub-project complete; perft suite, property tests and bench green
- 2026-09-26 core task 7 done: Game, outcomes, PGN
- 2026-09-26 core task 6 done: SAN and UCI move text
- 2026-09-26 core task 5 done: legal move generation and perft
- 2026-09-26 core task 4 done: position, FEN, hashing, play
- 2026-09-26 core task 3 done: move type
- 2026-09-26 core task 2 done: attack tables
- 2026-09-26 core task 1 done: foundation types
- 2026-09-26 core implementation plan written (8 tasks + setup), verified by replay.
- 2026-09-26 brainstorming complete; spec and handoff file written.
