# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: engine | Plan: docs/superpowers/plans/2026-09-26-engine.md
Branch: feat/jev-engine
Last completed task: engine task 6 (configuration)
Next task: engine task 7 (Jev client)
State: green (except 3 pre-existing old-code test failures)

## Verify before continuing
cargo test --lib engine::

## Core API caveats (read before building on core)
- `Position::play` / `to_san` require a move from `self.legal_moves()`; debug builds assert, release builds may corrupt the position; use `Game::play` for unvalidated input.
- `Game::undo` first withdraws a pending resignation and returns `None`; check `outcome()` to tell that apart from "nothing to undo".
- `parse_san` is strict (no `b8Q`, lowercase piece letters, or long algebraic `Ng1f3`) and reports every mismatch as `IllegalMove`; SAN leniency belongs to the TUI input layer.
- The en passant square (and so the Zobrist hash) is set whenever an enemy pawn attacks it, even when that capture is illegal because of a pin; a threefold repetition involving such a position can be missed. Rare; documented only.
- `from_fen` rejects positions beyond the promotion material budget; `MoveList` capacity (321) is derived from that budget.
- `pub mod core` shadows the built-in `core` crate at the crate root: write `::core::` in `src/lib.rs`.
- CI (`cargo test --verbose`) stops at the 3 known old-code failures, so `core_properties` and the bench are not exercised in CI until the cleanup sub-project (consider `--no-fail-fast`).
- `CLAUDE.md` still says "3 of 58" failing tests; now 3 of 109 lib tests plus 2 property tests. Update in cleanup.

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
- Engine design (spec 5, revised): user chose to add the `examples/jev_eval.rs` evaluation harness.
- Engine plan: capture annotations use a fourth qualifier, "wins material in the exchange" (spec 5.4 updated);
  `analyse`, `ComputerPlayer::from_config` and `ComputerPlayer::config` are public (spec 5.2 updated).
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
- 2026-09-26 engine task 6 done: configuration
- 2026-09-26 engine task 5 done: position description
- 2026-09-26 engine task 4 done: annotation and buckets
- 2026-09-26 engine task 3 done: search
- 2026-09-26 engine task 2 done: static exchange evaluation
- 2026-09-26 engine task 1 done: dependencies and static evaluation
- 2026-09-26 engine implementation plan written (9 tasks), verified by replay; prototype harness run: 63% agreement with search, 0 vetoes, 305 ms mean latency.
- 2026-09-26 engine brainstorming: spec section 5 revised (history-aware search, structured criteria, key fallback, retry-after, eval harness).
- 2026-09-26 feat/core-bitboards fast-forward merged into main; merged branches deleted.
- 2026-09-26 core final-review fix wave: FEN validation (en passant, material budget), MoveList capacity 321, debug legality assert in play, saturating clocks, API polish, fuzz test.
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
