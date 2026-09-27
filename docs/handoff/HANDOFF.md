# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: cleanup (not started) | Plan: to be written from spec section 7 step 4
Branch: feat/tui (complete after the user's manual smoke test; ready for user to merge)
Last completed task: tui final-review fix wave (A1–A4, B1–B10) — tui sub-project DONE pending manual smoke test
Next task: user runs `cargo run` for the manual smoke test below and merges feat/tui; then plan the cleanup sub-project
State: green (except 3 pre-existing old-code test failures)

## Verify before continuing
INSTA_UPDATE=no cargo test --lib tui:: && cargo test --lib engine:: && cargo test --lib core::
cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py --no-build

## Manual smoke test (TUI, by the user)
Run `cargo run` in a real terminal (Ghostty, Kitty, WezTerm or Alacritty) and check:
- the menu appears;
- Human vs Human plays by mouse click, by drag and through the command box (`/e4`);
- Human vs Jev shows "Jev thinking…" then Jev's move with the Jev panel filled (with `JEV_API_KEY`
  set), or "Local search" wording everywhere without it;
- `g` cycles the glyph sets; `u` undoes;
- `:savepgn ~/test` writes `~/test.pgn` and the status line says `saved ~/test.pgn`;
- `q` asks before quitting, and the shell is normal afterwards (mouse and paste modes off, cursor
  visible);
- optional: `NO_COLOR=1 cargo run` marks a picked-up piece reversed and capture targets as `( )`.

## Core API caveats (read before building on core)
- `Position::play` / `to_san` require a move from `self.legal_moves()`; debug builds assert, release builds may corrupt the position; use `Game::play` for unvalidated input.
- `Game::undo` first withdraws a pending resignation and returns `None`; check `outcome()` to tell that apart from "nothing to undo".
- `parse_san` is strict (no `b8Q`, lowercase piece letters, or long algebraic `Ng1f3`) and reports every mismatch as `IllegalMove`; SAN leniency belongs to the TUI input layer.
- The en passant square (and so the Zobrist hash) is set whenever an enemy pawn attacks it, even when that capture is illegal because of a pin; a threefold repetition involving such a position can be missed. Rare; documented only.
- `from_fen` rejects positions beyond the promotion material budget; `MoveList` capacity (321) is derived from that budget.
- `pub mod core` shadows the built-in `core` crate at the crate root: write `::core::` in `src/lib.rs`.
- CI (`cargo test --verbose`) stops at the 3 known old-code failures, so `core_properties` and the bench are not exercised in CI until the cleanup sub-project (consider `--no-fail-fast`).
- `CLAUDE.md` still says "3 of 58" failing tests; now 3 of 496 lib tests (493 pass, 3 ignored) plus 2 property tests. Its Commands section names the TUI (`cargo run`, `tests/pty_smoke.py`), but its Architecture section still describes only the legacy crate-root code. Update in cleanup.

### Engine caveats (read before building the TUI on engine)
- Give the worker a `Game` clone, not a `Position`: `analyse` needs the move history for repetition detection and `describe` sends the recent moves.
- Worst-case `choose_move` latency is about 19 s (3 attempts × 5 s timeout + backoff or Retry-After + search) and it cannot be cancelled; discard stale results by generation counter.
- Debug builds search about 23x slower than release (Kiwipete about 1.3 s vs 56 ms); feat/tui raised the dev profile (`[profile.dev.package.chess] opt-level = 3`), so `cargo run` searches Kiwipete in about 0.10 s.
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
- TUI plan: plan code blocks are long; implementers should extract fenced blocks from the task brief with a script, not retype. app.rs and panels.rs import each other, so they share Task 6. Test helpers are staged: test_support/mod.rs (Task 2), engine.rs (Task 5), harness.rs (Task 6).
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
- Engine follow-up (2026-09-27): SEE judges the first capturer by the pins of the position as it stands and the recaptures by the pins after the first capture (pins changed by later captures are still not modelled); `new_attack`'s "defended" check is pin-aware. The Jev endpoint is fixed, not configurable (user decision): `jev::JEV_ENDPOINT` = `https://api.typesafe.ai/v1/systemone`, the TypeSafe quickstart URL, with `Authorization: Bearer <key>` and `Content-Type: application/json`; an offline test pins that wire format. Test counts: `cargo test --lib engine::` 93 passed, 2 ignored; whole lib 199 passed, 3 failed (old code), 3 ignored.
- Follow-up review fixes (2026-09-27): in SEE a king captures only onto a square no enemy piece attacks, pinned or not (a pinned piece still guards against a king), so `capture_gain(d5)` in `4k3/8/4b3/3n4/2K5/8/8/4R3 b` and `see(d1d4)` in `3rk3/8/8/4b3/3r4/2K5/8/3RQ3 w` are now 0; `JEV_ENDPOINT` is re-exported as `engine::JEV_ENDPOINT` (no engine rustdoc warnings); the wire-format test compares body keys as a set and the endpoint test checks host and path. Test counts: `cargo test --lib engine::` 96 passed, 2 ignored; whole lib 202 passed, 3 failed (old code), 3 ignored.
- Final-review fix wave: a mate on the 100th half-move now outranks the fifty-move rule; SEE ignores pinned pieces off their pin line; annotation facts corrected ("exposed to capture" wording, no such fact for the capturing piece, a king only attacks undefended pieces); base-URL variable validated (since removed by the engine follow-up); transport errors classified (new `JevError::Request`), 1 MiB body cap and offline TcpListener tests; notes stripped of control characters; public docs, `ComputerPlayer` Debug and `MoveSource` Display; harness `jev pick` column and answered-only latency.
- TUI task 4: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 104 passed (brief said 103); the extra test is from task 3's fix round 1, which added a test after the brief text was written.
- TUI task 5: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 140 passed (brief said 139); the extra test carries forward from task 4's count above (104 baseline + 36 new tests in worker.rs/event.rs/terminal.rs = 140).
- TUI task 6: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 251 passed (brief said 250); the extra test carries forward from task 5's count above (140 baseline + 77 new tests in app.rs + 34 new tests in panels.rs = 251).
- TUI task 3 fix round 1: `movetext.rs`'s `loose_match` no longer short-circuits on the first loose-SAN or piece-spelling match; it now always also computes the promotion-without-piece candidates (renamed `missing_promotion` to `promotion_candidates`, returning candidates rather than a `Result`) and merges them in, so a pawn promotion typed without its piece (`bxc8`) that also matches another piece's move on the same square (`Bxc8`) is reported `MoveTextError::Ambiguous` (`Bxc8`, `bxc8=?`, the latter collapsing all four promotion pieces via new helper `promotion_family_labels`) instead of silently playing the other piece's move. Deviation from the plan's code (the brief's `loose_match`/`missing_promotion` are restructured), by fix-round ruling to close a verified review finding.
- TUI final-review fix wave (2026-09-27): A1 command box above the game-over overlay; A2 NO_COLOR board marks; A3 status messages and save errors fitted with `…`, paths shown with `~` and shortened in the folder part; B1 a computer move clears only turn notes; B2 Save dialog unquotes paths; B3 promotion picker Esc gives typed text back; B4 watchdog turns raw mode off before its writes; B5 stdin checked before setup; B6 move list columns; B7 pty smoke test uses cargo's target dir and checks stdin; B8/B9 run-loop tests. Test counts: `INSTA_UPDATE=no cargo test --lib tui::` 291 passed; whole lib 493 passed, 3 failed (old code), 3 ignored; `core_properties` 2 passed. Snapshots changed only by B6 (move rows of game_over_80x24, jev_reply_80x24, jev_vetoed_80x24, mid_game_80x24).
- Deliberate deviations from spec 6 wording on feat/tui (no code change planned):
  - 6.2/6.4: while only "Terminal too small" is shown, all input is ignored except Ctrl+C, which quits at once without the confirmation (it could not be seen) (`src/tui/app.rs:1036`).
  - 6.4: the focus stack also places the game-over overlay: dialog > command box > game-over overlay > board. When the computer's move ends the game while the command box is focused, keys keep going to the box (Esc leaves it); when the person's own move or command ends the game, the box is left so the overlay gets the next keys (A1).
  - 6.4: extra keys beyond the spec's list: `m` (menu) on the board; `n`/`s`/`m`/`u`/`q`, arrows and Tab on the game-over overlay; Ctrl+S also from the overlay; `:resign` asks first, and undo withdraws a resignation.
  - 6.4: a bare `:fen`, `:savefen` or `:savepgn` opens its dialog instead of printing a usage message; declining an overwrite gives the typed path back where it was typed; cancelling the promotion picker gives back a move typed without its piece (B3).
  - 6.4: undo in Jev vs Jev takes back one ply and pauses watching.
  - 6.3: with `NO_COLOR` the highlights get text and attribute marks as well as tints: selection reversed, last move underlined, capture targets `( )`, king in check `+ +` (A2); the spec names only the switch to Outline glyphs.
  - 6.2: status messages keep a fixed row budget: a message that does not fit ends in `…`; file paths are shown with `~` and lose folders from the middle first; save errors read `cannot save (<reason>): <path>` (A3). A computer move clears only messages about the turn (B1).
  - 6.2 layout: the move list writes a game that starts with Black to move as `12. ...  Kd7` (B6), not `12... Kd7`; PGN export is unchanged.
  - 6.1: `run` also refuses a non-terminal stdin, not only stdout (B5).
- TUI task 6 fix round 1: `app.rs`'s `apply_outcome` now clears `self.message` in the `EngineOutcome::Move` arm (before the `recovered` check) once the computer's move has been played, so an error set while the engine was thinking (a stray typed move, "asking the engine again", "took back N move(s)") no longer survives into the human's next turn. Deviation from the plan's code (the brief's `apply_outcome` omits this clear), by fix-round ruling to close a verified review finding.

## Known open issues
- Engine minor: annotate's "undefended" qualifier (annotate.rs, capture branch) counts a king as a recapturer even when the recapture would be illegal, e.g. `8/8/8/2k5/3n4/5B2/3QK3/8 w - - 0 1` Qxd4 reads "wins material in the exchange" instead of "undefended". SEE values are correct.
- Engine minor: `ComputerMove.model` and Transport/Request error text skip the control-character sanitiser.
- Docs nit: the spec (5.2) and older Notes lines name the endpoint `jev::JEV_ENDPOINT`; the public path is `chess::engine::JEV_ENDPOINT`.
- Historical engine plan text still uses the old "undefended against capture" wording and mentions `JEV_BASE_URL`.

### TUI (deferred minors from reviews)
- src/tui/glyphs.rs:323 `char_width` returns 1 for control characters, which ratatui's buffer drops (0 cells).
- src/tui/glyphs.rs:332 `shorten` cuts by char count, so 24 wide or escaped characters can still flood a message, and the cut can split a grapheme cluster.
- src/tui/glyphs.rs:258 the recommended env accessor `std::env::var(k).ok()` treats a non-UTF-8 `NO_COLOR` or `RCHESS_GLYPHS` as unset, with no warning (crossterm does the same for `NO_COLOR`).
- src/tui/glyphs.rs:278 the CLI and `RCHESS_GLYPHS` branches of `initial_glyphs` repeat the same parse-or-reject shape.
- src/tui/test_support/mod.rs:87 the doc of `char_events` says "as the terminal reports them", but every character is sent with no modifiers.
- src/tui/board.rs:182 the label column and row only get a symbol; their style is never reset, unlike square cells.
- src/tui/board.rs:923 no test renders a full 7×3 board (cursor outline over 3-row squares, the middle-row glyph cell, label rows).
- src/tui/input.rs:69 `insert_str` deletes tabs outright, so a pasted `:fen\t<FEN>` joins words and FEN fields.
- src/tui/input.rs:69 if the box already holds text, a paste made only of line breaks does not submit it.
- src/tui/input.rs:158 `is_ignored` misses some invisible format characters (U+061C, U+00AD, U+180E, U+034F, U+FFF9-U+FFFB).
- src/tui/movetext.rs:92 the `kind_of` closure is defined identically in `loose_match` and `long_algebraic`.
- src/tui/files.rs:129 the no-overwrite check and the rename are two steps, so a file created between them is silently replaced.
- src/tui/files.rs:127 overwriting a symlink replaces the link with a regular file and leaves its target unchanged.
- src/tui/files.rs:234 the PGN `Date` tag uses the UTC date, not the local date.
- src/tui/files.rs:152 `write_file` does two blocking fsyncs (file, then folder) on the UI thread.
- src/tui/files.rs:184 only the NotFound and IsADirectory branches of `describe` are tested.
- src/tui/files.rs:300 the `(None, None)` arm in `pgn_export`'s roster loop is dead code duplicating `Game::to_pgn`.
- src/tui/terminal.rs:298 `leave_once` clears ACTIVE before the restore runs, so a second concurrent caller returns at once and the process can exit mid-restore.
- src/tui/terminal.rs:161 `leave` stops at the first failed step inside each compound restore call.
- src/tui/terminal.rs:403 the panic-hook test replaces the process-global hook while other tests run, and leaves the probe hook installed if an early assertion fails.
- src/tui/terminal.rs:316 a comment in the panic hook says an engine panic reaches the UI as a failed reply; it becomes a local-search move.
- src/tui/terminal.rs:241 if the terminal closes and no SIGHUP ever reaches chess (a supervisor that ignores SIGHUP, zsh `trap '' HUP`), the UI thread spins at 100% CPU in crossterm's read loop.
- src/tui/worker.rs:388 `engine_survives_a_panic_and_keeps_answering` uses two engines, so it does not show the same engine keeps answering.
- src/tui/worker.rs:411 `reply_to_a_closed_channel_is_dropped_quietly` returns before the send it means to test has run.
- src/tui/test_support/engine.rs:112 `FakeEngine::local()` still reports every move as a Jev move with top three, confidence and model.
- src/tui/app.rs:1059 an Alt+key chord outside a text field is split into Esc plus the key; if that Esc opens or reveals a text field, the key is typed into it.
- src/tui/app.rs:1742 `q` asks for confirmation on a board with no moves; `n` and `m` do not.
- src/tui/mod.rs:328 every event in a batch is hit-tested against the hit map from the draw before the batch, even after an earlier event changed the layout.
- src/tui/mod.rs:96 `--help` into a closed pipe (`chess --help | true`) prints `chess: Broken pipe (os error 32)` and exits 1.
- src/tui/mod.rs:287 `--version` and `-V` are unknown arguments (a menu warning).
- src/tui/mod.rs:341 no test covers `perform`'s branch for a failed engine-thread spawn (the synthetic `EngineOutcome::Failed` reply).
- src/tui/mod.rs:386 the test module's `fn play(glyphs, warnings) -> Cli` helper shadows the outer `fn play(app, quit, fault)`.
- src/tui/panels.rs:202 menu warnings that do not fit are dropped without notice at 60×20.
- src/tui/terminal.rs:161 no cargo test checks the bytes `leave()` writes (tests/pty_smoke.py covers them).

## Open questions for user
- None.

## Log (newest first)
- 2026-09-27 tui final-review fix wave: game-over overlay focus, NO_COLOR marks, fitted status messages, turn-only message clearing, stdin check, move-list columns, watchdog raw mode, smoke-test target dir
- 2026-09-27 tui sub-project complete; pty smoke test green; awaiting manual smoke test
- 2026-09-27 tui task 6 fix round 1: stale error message no longer survives an engine reply
- 2026-09-27 tui task 6 done: app state machine and rendering
- 2026-09-27 tui task 5 done: engine worker, events and terminal guard
- 2026-09-27 tui task 4 done: saving FEN and PGN
- 2026-09-27 tui task 3 done: move text and command line
- 2026-09-27 tui task 2 done: board widget and hit-testing
- 2026-09-27 tui task 1 done: dependencies, module root and glyphs
- 2026-09-27 TUI plan written (7 tasks, ~15k lines incl. verified code and 19 snapshots); replay counts 20/39/81/103/139/250/272, pty smoke test green; spec 6 synced with the prototype.
- 2026-09-27 TUI brainstorming: spec section 6 revised (terminal guard, thread-aware panic hook, signals, stale-reply check, lenient move text, glyph sets, dev opt-level 3, snapshot tests).
- 2026-09-27 fix/engine-pin-and-endpoint merged into main: SEE pins recomputed after the first capture, kings respect pinned guards, fixed Jev endpoint (JEV_BASE_URL removed), wire format pinned to the quickstart.
- 2026-09-27 engine follow-up on fix/engine-pin-and-endpoint: SEE pin release fixed, pin-aware `new_attack`, fixed Jev endpoint (base-URL variable removed); ready to merge; next: TUI brainstorming.
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
