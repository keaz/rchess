# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: tui (not started) | Plan: to be written — docs/superpowers/plans/2026-09-26-tui.md
Branch: main @ 7e6bce3 (feat/jev-engine fast-forward merged, then deleted)
Last completed task: engine sub-project merged into main — engine DONE (one known issue open, see below)
Next task: brainstorm/plan the TUI from spec section 6 (read "Core API caveats", the engine caveats and "Known open issues" first)
State: green (except 3 pre-existing old-code test failures)

## Verify before continuing
cargo test --lib engine:: && cargo test --lib core:: && cargo test --test core_properties

## Core API caveats (read before building on core)
- `Position::play` / `to_san` require a move from `self.legal_moves()`; debug builds assert, release builds may corrupt the position; use `Game::play` for unvalidated input.
- `Game::undo` first withdraws a pending resignation and returns `None`; check `outcome()` to tell that apart from "nothing to undo".
- `parse_san` is strict (no `b8Q`, lowercase piece letters, or long algebraic `Ng1f3`) and reports every mismatch as `IllegalMove`; SAN leniency belongs to the TUI input layer.
- The en passant square (and so the Zobrist hash) is set whenever an enemy pawn attacks it, even when that capture is illegal because of a pin; a threefold repetition involving such a position can be missed. Rare; documented only.
- `from_fen` rejects positions beyond the promotion material budget; `MoveList` capacity (321) is derived from that budget.
- `pub mod core` shadows the built-in `core` crate at the crate root: write `::core::` in `src/lib.rs`.
- CI (`cargo test --verbose`) stops at the 3 known old-code failures, so `core_properties` and the bench are not exercised in CI until the cleanup sub-project (consider `--no-fail-fast`).
- `CLAUDE.md` still says "3 of 58" failing tests; now 3 of 199 lib tests plus 2 property tests. Update in cleanup.

### Engine caveats (read before building the TUI on engine)
- Give the worker a `Game` clone, not a `Position`: `analyse` needs the move history for repetition detection and `describe` sends the recent moves.
- Worst-case `choose_move` latency is about 19 s (3 attempts × 5 s timeout + backoff or Retry-After + search) and it cannot be cancelled; discard stale results by generation counter.
- Debug builds search about 23x slower than release (Kiwipete about 1.3 s vs 56 ms): the TUI plan should run `--release` or raise the dev profile's `opt-level`.
- Crowded positions (many pawns in contact) take up to ~0.85 s in release: quiescence has no pruning and there is no time limit yet.
- Never enable TRACE-level logging for `ureq` / `ureq_proto`: it would print the `Authorization` header with the API key.
- Jev receives the state fields and the options in alphabetical order (serde_json sorts object keys; `preserve_order` stays off by ruling).
- Jev is still asked when the shortlist has one entry; this inflates the harness agreement figure slightly.

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
- Evaluation harness run (`examples/jev_eval.rs`, 20 positions): 19 answered by Jev, agreement with search best 12/19 (63%), 0 vetoes, 0 fallbacks, mean latency 311 ms, 15900 input tokens (about $0.000668).
- TUI integration: send `game.clone()` (the whole `Game`) to a long-lived worker thread that owns the ComputerPlayer<JevClient> (ComputerPlayer::from_config(EngineConfig::from_env())), or share the player through `Arc` (it is `Sync`, not `Clone`); worst-case `choose_move` latency is about 19 s (3 attempts × 5 s timeout + backoff + search), so tag each request with a generation counter and discard stale results; show EngineConfig.warnings, ComputerMove.note, source (its `Display` label), top and model.
- The ignored live Jev round-trip test (`engine::jev::tests::live_choice_round_trip`) passed once during Task 7, with the user's key.
- Test counts after the final fix wave: `cargo test --lib engine::` 87 passed, 2 ignored; `cargo test --lib core::` 51 passed, 1 ignored; `core_properties` 2 passed; whole lib 193 passed, 3 failed (old code), 3 ignored.
- Final-review fix wave: a mate on the 100th half-move now outranks the fifty-move rule; SEE ignores pinned pieces off their pin line; annotation facts corrected ("exposed to capture" wording, no such fact for the capturing piece, a king only attacks undefended pieces); JEV_BASE_URL validated; transport errors classified (new `JevError::Request`), 1 MiB body cap and offline TcpListener tests; notes stripped of control characters; public docs, `ComputerPlayer` Debug and `MoveSource` Display; harness `jev pick` column and answered-only latency.

## Known open issues
- Engine SEE pin release (important, found by the final re-review, user chose to merge first): `src/engine/see.rs` computes pins once from the position before the first capture. When the pinning piece itself makes the first capture off its line, the released piece is still treated as pinned. Repro: `4k3/8/8/4b3/8/2p5/8/4QK2 w - - 0 1` — Qxc3 is annotated "wins material in the exchange" though Bxc3 wins the queen; `capture_gain` then yields a false threat and a false "moves ... to safety" fact. Search scores, buckets, the losing filter and the veto are unaffected. Fix: recompute the recapturing side's pins after the first capture (from `pos.play(mv)` in `see`, with the first capturer removed in `capture_gain`) and add the repro as a test.
- Engine minor follow-ups: `new_attack`'s "defended" check is pin-blind; `JEV_BASE_URL=http://` (or `https://`) passes validation and becomes `http:`; `ComputerMove.model` and Transport/Request error text skip the control-character sanitiser; the historical engine plan still uses the old "undefended against capture" wording.

## Open questions for user
- None.

## Log (newest first)
- 2026-09-27 feat/jev-engine fast-forward merged into main (engine DONE); SEE pin-release issue left open by user choice, recorded under Known open issues.
- 2026-09-26 engine final-review fix wave (F1–F18): mate before fifty-move rule, pin-aware SEE, annotation fixes, transport hardening and offline tests, API polish, harness columns, docs
- 2026-09-26 engine sub-project complete; harness run recorded
- 2026-09-26 engine task 8 done: computer player
- 2026-09-26 engine task 7 done: Jev client
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
