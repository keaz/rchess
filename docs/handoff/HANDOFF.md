# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: laya (Laya as a second computer player; Jev vs Laya) | Spec: docs/superpowers/specs/2026-09-29-laya-integration-design.md
Branch: feat/laya (from main at fe1eb73)
Last completed task: Task 7 (side panel before the first move, debug mode per engine)
Next task: Task 8 (offline Laya on a real terminal, docs, final verification)
State: green (cargo test, fmt, clippy)
Earlier: main is ahead of origin/main (not pushed); pushing it is still pending.

## Verify before continuing
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py --no-build

## Manual test (tui-polish, by the user; spec 9.6)
Run `cargo run` in Ghostty and in one other terminal (Kitty, WezTerm or Alacritty) and check:
- the TUI fills the terminal and the board grows with the window;
- pieces are pictures and easy to tell apart (Alacritty: text pieces; pictures after `g` on a large window);
- `g` cycles Image → Solid → Outline → Ascii;
- a font zoom keeps the pictures sized to their squares;
- `cargo run -- --debug`, then a game against Jev with `JEV_API_KEY` set: `d` shows the request and
  response with the key shown as `<redacted>`;
- `~/.local/state/rchess/jev-debug.jsonl` (or `$XDG_STATE_HOME/rchess/...`) gets one line per request
  and has mode 0600;
- quitting leaves the shell normal (mouse and paste modes off, cursor visible).

## Manual smoke test (TUI from the tui sub-project, still valid)
Run `cargo run` in a real terminal (Ghostty, Kitty, WezTerm or Alacritty) and check:
- the menu appears;
- Human vs Human plays by mouse click, by drag and through the command box (`/e4`);
- Human vs Jev shows "Jev thinking…" then Jev's move with the Jev panel filled (with `JEV_API_KEY`
  set), or "Local search" wording everywhere without it;
- `g` cycles the glyph sets; `u` undoes;
- `:savepgn ~/test` writes `~/test.pgn` and the status line says `saved ~/test.pgn`;
- once a move is played `q` asks before quitting (before that it quits at once), and the shell is
  normal afterwards (mouse and paste modes off, cursor visible);
- optional: `NO_COLOR=1 cargo run` marks a picked-up piece reversed and capture targets as `( )`.

## Core API caveats (read before building on core)
- `Position::play` / `to_san` require a move from `self.legal_moves()`; debug builds assert, release builds may corrupt the position; use `Game::play` for unvalidated input.
- `Game::undo` first withdraws a pending resignation and returns `None`; check `outcome()` to tell that apart from "nothing to undo".
- `parse_san` is strict (no `b8Q`, lowercase piece letters, or long algebraic `Ng1f3`) and reports every mismatch as `IllegalMove`; SAN leniency belongs to the TUI input layer.
- The en passant square (and so the Zobrist hash) is set whenever an enemy pawn attacks it, even when that capture is illegal because of a pin; a threefold repetition involving such a position can be missed. Rare; documented only.
- `from_fen` rejects positions beyond the promotion material budget; `MoveList` capacity (321) is derived from that budget.
- `pub mod core` shadows the built-in `core` crate at the crate root: write `::core::` in `src/lib.rs`.

### Engine caveats (read before building the TUI on engine)
- Give the worker a `Game` clone, not a `Position`: `analyse` needs the move history for repetition detection and `describe` sends the recent moves.
- Worst-case `choose_move` latency is about 19 s (3 attempts × 5 s timeout + backoff or Retry-After + search) and it cannot be cancelled; discard stale results by generation counter.
- Debug builds search about 23x slower than release (Kiwipete about 1.3 s vs 56 ms); feat/tui raised the dev profile (`[profile.dev.package.chess] opt-level = 3`), so `cargo run` searches Kiwipete in about 0.10 s.
- Crowded positions (many pawns in contact) take up to ~0.85 s in release: quiescence has no pruning and there is no time limit yet.
- Never enable TRACE-level logging for `ureq` / `ureq_proto`: it would print the `Authorization` header with the API key.
- Jev receives the state fields and the options in alphabetical order (serde_json sorts object keys; `preserve_order` stays off by ruling).
- Jev is still asked when the shortlist has one entry; this inflates the harness agreement figure slightly.

## Notes / decisions made during work
- tui-polish plan: tasks are `git apply` patches (Task 1 carries the 12 piece PNGs as a binary patch). Task 7 adds tests only (no RED; Tasks 4 and 6 did the wiring). Task 8 applies the whole-prototype review fixes in one patch. The prototype and its report live in the git-ignored .superpowers/sdd/2026-09-27-tui-polish/ (proto/, proto-report.md).
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
- Engine follow-up (2026-09-27): SEE judges the first capturer by the pins of the position as it stands and the recaptures by the pins after the first capture (pins changed by later captures are still not modelled); `new_attack`'s "defended" check is pin-aware. The Jev endpoint is fixed, not configurable (user decision): `chess::engine::JEV_ENDPOINT` = `https://api.typesafe.ai/v1/systemone`, the TypeSafe quickstart URL, with `Authorization: Bearer <key>` and `Content-Type: application/json`; an offline test pins that wire format. Test counts: `cargo test --lib engine::` 93 passed, 2 ignored; whole lib 199 passed, 3 failed (old code), 3 ignored.
- Follow-up review fixes (2026-09-27): in SEE a king captures only onto a square no enemy piece attacks, pinned or not (a pinned piece still guards against a king), so `capture_gain(d5)` in `4k3/8/4b3/3n4/2K5/8/8/4R3 b` and `see(d1d4)` in `3rk3/8/8/4b3/3r4/2K5/8/3RQ3 w` are now 0; `JEV_ENDPOINT` is re-exported as `engine::JEV_ENDPOINT` (no engine rustdoc warnings); the wire-format test compares body keys as a set and the endpoint test checks host and path. Test counts: `cargo test --lib engine::` 96 passed, 2 ignored; whole lib 202 passed, 3 failed (old code), 3 ignored.
- Final-review fix wave: a mate on the 100th half-move now outranks the fifty-move rule; SEE ignores pinned pieces off their pin line; annotation facts corrected ("exposed to capture" wording, no such fact for the capturing piece, a king only attacks undefended pieces); base-URL variable validated (since removed by the engine follow-up); transport errors classified (new `JevError::Request`), 1 MiB body cap and offline TcpListener tests; notes stripped of control characters; public docs, `ComputerPlayer` Debug and `MoveSource` Display; harness `jev pick` column and answered-only latency.
- TUI task 4: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 104 passed (brief said 103); the extra test is from task 3's fix round 1, which added a test after the brief text was written.
- TUI task 5: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 140 passed (brief said 139); the extra test carries forward from task 4's count above (104 baseline + 36 new tests in worker.rs/event.rs/terminal.rs = 140).
- TUI task 6: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 251 passed (brief said 250); the extra test carries forward from task 5's count above (140 baseline + 77 new tests in app.rs + 34 new tests in panels.rs = 251).
- TUI task 3 fix round 1: `movetext.rs`'s `loose_match` no longer short-circuits on the first loose-SAN or piece-spelling match; it now always also computes the promotion-without-piece candidates (renamed `missing_promotion` to `promotion_candidates`, returning candidates rather than a `Result`) and merges them in, so a pawn promotion typed without its piece (`bxc8`) that also matches another piece's move on the same square (`Bxc8`) is reported `MoveTextError::Ambiguous` (`Bxc8`, `bxc8=?`, the latter collapsing all four promotion pieces via new helper `promotion_family_labels`) instead of silently playing the other piece's move. Deviation from the plan's code (the brief's `loose_match`/`missing_promotion` are restructured), by fix-round ruling to close a verified review finding.
- TUI final-review fix wave (2026-09-27): A1 command box above the game-over overlay; A2 NO_COLOR board marks; A3 status messages and save errors fitted with `…`, paths shown with `~` and shortened in the folder part; B1 a computer move clears only turn notes; B2 Save dialog unquotes paths; B3 promotion picker Esc gives typed text back; B4 watchdog turns raw mode off before its writes; B5 stdin checked before setup; B6 move list columns; B7 pty smoke test uses cargo's target dir and checks stdin; B8/B9 run-loop tests. Test counts: `INSTA_UPDATE=no cargo test --lib tui::` 291 passed; whole lib 493 passed, 3 failed (old code), 3 ignored; `core_properties` 2 passed. Snapshots changed only by B6 (move rows of game_over_80x24, jev_reply_80x24, jev_vetoed_80x24, mid_game_80x24). Follow-up: the Status message budget counts wrapped rows (a game-over line that wraps no longer cuts a save message to `saved`), and the NO_COLOR capture/check marks survive the keyboard cursor; `tui::` 293 passed.
- Deliberate deviations from spec 6 wording on feat/tui (no code change planned):
  - 6.2/6.4: while only "Terminal too small" is shown, all input is ignored except Ctrl+C, which quits at once without the confirmation (it could not be seen) (`App::handle` in `src/tui/app.rs`).
  - 6.4: the focus stack also places the game-over overlay: dialog > command box > game-over overlay > board. When the computer's move ends the game while the command box is focused, keys keep going to the box (Esc leaves it); when the person's own move or command ends the game, the box is left so the overlay gets the next keys (A1).
  - 6.4: extra keys beyond the spec's list: `m` (menu) on the board; `n`/`s`/`m`/`u`/`q`, arrows and Tab on the game-over overlay; Ctrl+S also from the overlay; `:resign` asks first, and undo withdraws a resignation.
  - 6.4: a bare `:fen`, `:savefen` or `:savepgn` opens its dialog instead of printing a usage message; declining an overwrite gives the typed path back where it was typed; cancelling the promotion picker gives back a move typed without its piece (B3).
  - 6.4: undo in Jev vs Jev takes back one ply and pauses watching.
  - 6.3: with `NO_COLOR` the highlights get text and attribute marks as well as tints: selection reversed, last move underlined, capture targets `( )`, king in check `+ +` (A2); under the keyboard cursor these marks move inside its `[ ]` (`[(p)]`), or on the smallest (3x1) squares keep only their right side (`[p)`, `[k+`), so the cursor never hides them; the spec names only the switch to Outline glyphs.
  - 6.2: status messages keep a fixed row budget: a message that does not fit ends in `…`; file paths are shown with `~` and lose folders from the middle first; save errors read `cannot save (<reason>): <path>` (A3). A computer move clears only messages about the turn (B1).
  - 6.2 layout: the move list writes a game that starts with Black to move as `12. ...  Kd7` (B6), not `12... Kd7`; PGN export is unchanged.
  - 6.1: `run` also refuses a non-terminal stdin, not only stdout (B5).
- TUI task 6 fix round 1: `app.rs`'s `apply_outcome` now clears `self.message` in the `EngineOutcome::Move` arm (before the `recovered` check) once the computer's move has been played, so an error set while the engine was thinking (a stray typed move, "asking the engine again", "took back N move(s)") no longer survives into the human's next turn. Deviation from the plan's code (the brief's `apply_outcome` omits this clear), by fix-round ruling to close a verified review finding.
- TUI task 5 (draw pieces as pictures): implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 371 passed (brief said 369); the extra 2 tests carry forward from the task 4 fix round's baseline (355 passed there vs. the brief's 353), same 16-test delta both times.
- TUI task 4 fix round 1: `graphics.rs`'s `ask` no longer writes the capability query at all on non-unix platforms (it used to write it, then call `read_stdin_byte`, whose non-unix stub errors without reading anything, so the terminal's reply landed on the wire for crossterm to read as key presses once the event loop started). `ask` is now a thin wrapper around a new `ask_with(readable, write, read_byte, query, stop)` that returns `QueryError::Io(Unsupported)` before writing when `readable` (the platform constant `ANSWERS_READABLE`, true only on unix) is false; two new tests (`ask_writes_nothing_where_the_answers_cannot_be_read`, `ask_writes_then_reads_where_the_answers_can_be_read`) cover both branches on any platform via the injected writer/reader. Deviation from the plan's code (the brief's `ask` is unconditional), by fix-round ruling to close a verified review finding.
- TUI task 6 (debug mode: exchange history, view and log): implemented as specified in the brief, no deviations. Starting point was 372 passed (brief said 369, carried forward from task 5's fix-round baseline); after this task `INSTA_UPDATE=no cargo test --lib tui::` reports 419 passed (brief said 416, same +3 delta carried forward) and `cargo test --lib engine::` reports 110 passed (brief said 110, matches).
- TUI task 5 fix round 1: `Cargo.toml` gains `[profile.dev.package.<crate>] opt-level = 3` for `image`, `ratatui-image`, `icy_sixel`, `png`, `fdeflate`, `miniz_oxide`, `flate2`, `base64` and `crc32fast` (corrected in the final fix wave: `quantette`, `base64-simd` and `vsimd` added, `base64` dropped, see above), alongside the existing `[profile.dev.package.chess]`, so `PieceImages::picture`'s resize/sixel/PNG/base64 encoding no longer runs at `opt-level = 0` under `cargo build`/`cargo run` (measured 1-6 s per full board rebuild in debug Sixel before this fix, vs. 0.1-0.2 s in release; unchanged code and public API). New test `board::tests::dev_builds_optimise_the_picture_encoding_crates` reads `Cargo.toml` at test time and asserts each override is present, since the cost itself only shows up in a real (non-test) build. Deviation from the plan's code (no patch in the brief touches `Cargo.toml`), by fix-round ruling to close a verified review finding.
- TUI task 7 (pty scenarios for the graphics query and debug mode): implemented as specified in the brief, no deviations (tests only, no RED step). Starting point was 421 passed (brief said 416, same +5 delta carried forward from task 6 fix round 1's baseline); after this task `INSTA_UPDATE=no cargo test --lib tui::` reports 422 passed (brief said 417, same +5 delta) and `cargo test --lib engine::` reports 110 passed (brief said 110, matches). `tests/pty_smoke.py` prints `ALL CHECKS PASSED` with the new scenarios (unanswered/late/kitty/signal/hangup graphics query, `--debug` and `RCHESS_DEBUG=1`) present in its output.
- TUI task 8 (whole-branch review fixes): the working tree at the start of this task was already mid-`git apply --3way` from an earlier, unfinished run of the same task (its extracted `patches/task8.diff`, `UU` entries and conflict markers were present in `src/tui/app.rs`, `src/tui/debug.rs`, `src/tui/graphics.rs` and `src/tui/panels.rs` before this session touched anything). Resolved rather than restarted, per the task's own conflict-resolution rule. `src/tui/debug.rs`'s conflict text was already resolved in the working tree (only `git add` was missing). `src/tui/app.rs`: kept both the earlier fix round's `a_log_failure_found_on_the_menu_is_shown_once_a_game_starts` test and the patch's own `a_log_failure_found_on_the_menu_waits_for_the_next_game` test (different names, same behaviour class, no code conflict once both are kept); `src/tui/app.rs` had a second conflict, in `report_log_failure` itself (first left out of this note and of the task report; disclosed in fix round 1): the earlier fix round's implementation from `f93fffd` was kept over the patch's version. The patch's version returned early while the Menu is up, leaving the failure unreported in `DebugSession::log_failure` until a later event on a game screen; the kept one takes the failure at once into `pending_log_failure` and shows it as soon as the screen is `Playing` or `GameOver`. Both tests above pass against the kept implementation; two more `DebugLog::open(None)` call sites outside the conflict hunks (in `the_mode_title_keeps_the_border_and_debug_moves_to_the_line_when_narrow`, not touched by `task8.diff` itself) no longer compiled against the patch's new `DebugLog::open(Result<PathBuf, String>)` signature and were updated to `DebugLog::open(Err(NO_LOG_PATH.to_string()))` to match every other call site. `src/tui/graphics.rs`: kept the earlier fix round's `ANSWERS_READABLE` guard (`ask`/`ask_with`/`write_stdout`) and added the patch's `FontMeter` on top of it, routing `FontMeter::measure` through the same `ANSWERS_READABLE` check and `write_stdout` instead of the patch's own ungated `ask`/`write_query`, which were dropped as duplicates. `src/tui/panels.rs`: kept the earlier fix round's `status_panel`/`status_lines` implementation (mode title never loses the Status border to `DEBUG`) over the patch's own independent attempt at the same fix, since both implement the same interface requirement and the earlier one already had passing tests; the patch introduced no new panels.rs tests that depended on its own version. Test counts after resolution: `tui::` 448 passed (brief expected 443; +5, mostly the kept duplicate test plus the FontMeter/graphics tests the brief's earlier count did not yet include), `engine::` 112 passed (brief expected 112, matches). `cargo fmt --check`, `cargo clippy --all-targets` (tui/engine/main.rs) and `tests/pty_smoke.py` are all clean/green.
- TUI task 8 fix round 1: Kitty pictures are now deleted by image id. ratatui-image 11.1 sends every Kitty picture as a virtual placement (`i=<id>,a=T,U=1`), and Kitty deletes virtual placements only when `d` is i/I/r/R/n/N, so the old `ESC _ G a=d,d=A ESC \` never removed them. `board.rs`'s `PieceImages::picture` builds Kitty pictures itself (`kitty_picture`: `ratatui_image::protocol::kitty::Kitty::new(image, cells, id, tmux, false)` wrapped in `Protocol::Kitty`; the composite is already the image area's exact pixel size, so this is what `Picker::new_protocol` built, minus the random id; `compress` is false because the query never probes compression). The id comes from `terminal::next_kitty_id(tmux)`: `terminal.rs`'s lock-free `KittyIds` hands out consecutive ids from a base derived from the process id (`kitty_base`, a MurmurHash3 finaliser mapped into 1..=2^31, so ids never wrap to 0 and two sessions in one terminal start far apart) and remembers them as a count, so ids of pictures dropped by cache clears are kept too. `leave()` (normal exit, `?` errors, panic hook, signals and the watchdog's forced restore all go through it) writes one `ESC _ G a=d,d=I,i=<id> ESC \` per id handed out (tmux-wrapped when the picker detected tmux) before mouse/paste off and `?1049l`, only when any Kitty picture was built; `mod.rs` no longer calls the removed `note_kitty_images`. Tests: `terminal.rs` unit tests for the per-id bytes, the tmux wrapping, the teardown order and the id allocator; a `board.rs` test that every id in a transmission (before and after a cache clear) is recorded; `tests/pty_smoke.py` now parses the transmitted ids and checks each is deleted by id once with uppercase I, with no `a=d,d=A`, before `?1006l`, on quit (with a resize that sends new ids in between) and in a new SIGTERM-after-Kitty scenario. Known limit: one delete command per picture built, so a long Kitty session with many window resizes writes a few thousand short commands at exit.
- tui-polish final fix wave (2026-09-28): spec 9.3 changed: Kitty pictures now follow a font zoom like Sixel and iTerm2 (a resize measures the font; a new size rebuilds the picker and the pictures). Kitty and Ghostty size a placeholder picture from its pixel size and the current cell size, so the old ruling ("Kitty placeholders scale with the cells") left pieces cropped or shrunk after a zoom until restart.
- tui-polish final fix wave (2026-09-28): the dev-profile picture-crate list (task 5 fix round 1) missed `quantette`, the colour quantiser icy_sixel 0.5 runs for every Sixel picture (measured by the reviewers: 1.2-4.4 s per Sixel board rebuild in dev, about 0.1-0.2 s with it optimised), and named `base64`, which the picture encoders do not use. `Cargo.toml` now also optimises `quantette`, `base64-simd` and `vsimd` (what ratatui-image's Kitty and iTerm2 encoders call) and drops `base64`; `dev_builds_optimise_the_picture_encoding_crates` checks the corrected list.
- TUI task 6 fix round 1: closed six verified findings (2 and 4 name the same gap) against spec 9.4. panels.rs: `status_panel` now sizes the mode-title room without DEBUG's contribution, so the mode title always keeps the Status border, and only adds ` DEBUG ` there when both titles still fit; when they do not, `status_lines` (which gained a `debug_first: bool` parameter) starts the turn line with a `DEBUG ` span instead of dropping the mode title. debug.rs: `open_log` now also tightens an already-existing log file's mode to 0600 with `set_permissions` (`OpenOptionsExt::mode` only takes effect when the open call actually creates the file, so a pre-existing looser-mode file used to keep leaking Jev bodies); `log_path` expands a leading `~/` in `RCHESS_DEBUG_LOG` to `HOME` via a new private `expand_tilde` helper (the same rule as `files::resolve_path`), returning `None` (the existing generic "no log file" message) when `HOME` is unset. `log_path`'s `Option<PathBuf>` return type is kept unchanged on purpose: Task 8's own patch later changes it to `Result<PathBuf, String>` and its diff hunks assume the pre-Task-8 signature, so reporting the specific "cannot expand ~" reason (rather than the generic no-log message) is left to that task, which is expected to need reconciling with this fix round's edits around `log_path`/`open_log`/`DebugLog::open`. app.rs: a debug log failure found while the Menu is up is now kept in a new `pending_log_failure: Option<Message>` field (the Menu screen never draws `message`, and `start()` clears `message` for the new game) and shown once the screen becomes `Playing` or `GameOver`, instead of being silently dropped by the next game start. Deviation from the plan's code and from task 6's own patch, by fix-round ruling to close verified review findings.
- Test counts in older notes and log entries ("3 failed (old code)", "only the 3 known old-code failures") include the legacy tests that cleanup task 1 deleted with the legacy code; since then `cargo test` has no failures.
- cleanup rulings (spec 10, amended 2026-09-28): every start-up graphics query ends with a device-attributes request after the status request (for WezTerm and Konsole it is the only one); its answer is read and dropped after the status report, or reaches crossterm last as an event when the answers are late, so a late answer never leaves crossterm's read waiting for a key. A terminal that does not answer it adds up to 200 ms to start-up, and keys typed during the 200 ms drain or before that answer are lost.
- cleanup rulings: a non-JSON Jev body still has the key replaced in place when it appears plainly or as `\/`; the whole body is withheld (`<redacted: the body contained the API key>`) only when the key survives that replacement, e.g. `\u`-escaped.
- cleanup rulings: non-UTF-8 `RCHESS_GLYPHS`, `RCHESS_IMAGES` and `NO_COLOR` always give a menu warning; `RCHESS_DEBUG_LOG`, `XDG_STATE_HOME` and `HOME` only in debug mode, the only mode that reads them. `RCHESS_IMAGES` takes `on`, `1`, `true` and `yes` as on without a warning.
- cleanup rulings: saving without overwrite hard-links the temp file into place and falls back to check-then-rename only when the file system reports hard links as `Unsupported` or, as Linux does for FAT, `PermissionDenied` (EPERM).
- cleanup rulings: the debug log is opened with `O_NOFOLLOW` (rustix's `fs` feature, the only dependency change of the sub-project), so a symbolic link planted after the link check is refused ("is a link") without its target being opened or created; `ELOOP` maps to that refusal.
- cleanup rulings: a reply held while Jev vs Jev is paused is logged on arrival, so its log line's `stale` is its state then (`held: true`); whether it was later played or dropped changes only the in-memory history.
- cleanup rulings: CI pins the Rust toolchain exactly (1.98.1); bumping it is a deliberate change that fixes whatever the new clippy finds.
- cleanup task 7: CI (`.github/workflows/rust.yml`) runs fmt, clippy `-D warnings` on all targets, `cargo test`, `cargo build` and `tests/pty_smoke.py --no-build` on ubuntu-latest with the Rust toolchain pinned to 1.98 (`dtolnay/rust-toolchain@1.98`; bump it deliberately, so a new clippy lint cannot turn CI red without a code change), each command under `env -u JEV_API_KEY -u TYPESAFE_API_KEY` and with no secret. The pty smoke test's timing bounds each sit below a fixed wait the bug they catch would fall into (the query's 1 s deadline, the 1 s stuck-UI grace); the signal and hangup checks during the graphics query are now timed from when the query was seen, waits for something that must happen got seconds, and the busy-hangup scenario uses a 250x800 window plus 20 resizes so its frames overflow a Linux pty buffer too, and on Linux checks through `/proc/<pid>/syscall` that the UI really is blocked in a write before the hangup. The split-answer and signal-with-answers scenarios skip their timing checks with a NOTE when the harness itself saw the query or acted too late for them to mean anything. There is no setting to scale the bounds. (Superseded by the final fix wave: CI no longer runs the pty smoke test, see below.)
- cleanup final fix wave: CI is a matrix of ubuntu-latest and macos-latest running `cargo fmt --check`, clippy `-D warnings` on all targets, `cargo build` and `cargo test`, with the toolchain pinned to `dtolnay/rust-toolchain@1.98.1` and a `workflow_dispatch` trigger; the pty smoke test is a local check only (CLAUDE.md, Commands). The user has built and run the branch on Ubuntu without errors. A UI that fails with an error on a terminal that closed without SIGHUP now ends by SIGHUP when stdin looks closed (`signal_after_error` in src/tui/mod.rs), instead of exiting with status 1 when the "hangup" thread's next look comes just after the 100 ms signal grace. The menu never shows "+0 more warnings": when no warning is among the notes that do not fit, there is no count.

## Known open issues
Limits kept on purpose (spec 10.7):
- src/tui/files.rs: the PGN `Date` tag is the UTC date, not the local date (a local date needs a timezone dependency).
- Saving (`files::write_file`, two fsyncs) and picture encoding (`board::PieceImages`) run on the UI thread: about 0.1-0.2 s for a Sixel board at 300x100, with the dev-profile overrides and in release. A crate added to the encoding path needs its own `[profile.dev.package]` override, or debug builds stall again.
- src/tui/board.rs: half-block pictures are written in 24-bit colour even on the 256-colour palette (ratatui-image behaviour).
- src/tui/graphics.rs: on a terminal that answers more than 1 s late, keys typed while the start-up query or a font measurement waits (and during the 200 ms drain after a query that gave up, or before the trailing device-attributes answer) are lost, and a stale font size can stay until the next resize. Kitty measures the font on every resize too, so this applies to it as well.
- No CI job builds for a non-unix target; the non-unix graphics path (the query skipped without a warning) is checked by reading only.
- Historical plan documents are not rewritten: the engine plan still uses the old "undefended against capture" wording and mentions `JEV_BASE_URL`.

### Follow-ups (not spec limits)
Minor findings from the final review, deferred; none blocks the merge.
- src/engine/player.rs:124: `redacted()` redacts the key but does not apply `printable()` (src/engine/jev.rs:241), so the "Jev unavailable" note built from another chooser's error (line 191) could carry control characters or run long.
- src/tui/graphics.rs:319: the tail of a Kitty answer split by the query deadline that arrives more than 200 ms (`DRAIN_TIME`) late is not drained and can reach the event loop.
- src/tui/terminal.rs:679: macOS reports POLLNVAL on a stdin redirected from `/dev/tty` (`< /dev/tty`), which `look` takes as a closed terminal, so the "hangup" thread ends such a session.
- src/tui/graphics.rs:340: the drain (200 ms) plus the device-attributes read (up to another 200 ms) after a late query can delay a quit by about 450 ms.
- src/tui/glyphs.rs:508: `shorten` may cut inside a ZWJ emoji sequence (it keeps combining marks, not zero-width joiners).
- src/tui/debug.rs:907: the newline check on an existing debug log re-opens the log by name (`ends_with_newline`) instead of reading through the opened, link-checked file.
- src/tui/debug.rs:917: a FIFO at the debug log path blocks the `debug-log` thread in `open` until a reader appears (quitting still ends after `LOG_GRACE`).
- src/tui/terminal.rs:1384 the test `a_closed_terminal_is_reported_once` uses the unix-only `HANGUP_LOOK` without `#[cfg(unix)]`, so the tests would not compile for a non-unix target (no CI job builds one; gate the test or drop the cfg on the constant).

## Open questions for user
- None.

## Log (newest first)
- 2026-09-28 README example switched to a position where Jev's choice matters (after 1. d4 Nf6 2. c4: twelve good moves within 50 cp, the search would play Nc6, Jev picked d5); exchange screenshots re-recorded; README notes the possible "no other option within the veto margin" shortcut (not implemented)
- 2026-09-28 README written on main (user request): architecture, when and how Jev is used, what is sent and why; screenshots (PNG) and low-res gameplay videos (MP4 plus animated WebP) recorded in Ghostty with the release build, stored in docs/media/
- 2026-09-28 chore/cleanup merged into main locally (ab2383e); merged result verified: cargo test fully green (lib 689), fmt and clippy -D warnings clean, pty smoke green
- 2026-09-28 cleanup final fix wave: CI matrix on Linux and macOS with an exact toolchain pin and a manual trigger (pty smoke local only), an error on a closed terminal ends by SIGHUP, no "+0 more warnings", HANDOFF follow-ups, CLAUDE.md fixes, stronger fallback and picture-cache tests
- 2026-09-28 cleanup task 7 done: CI workflow (fmt, clippy -D warnings, test, build, pty smoke), CLAUDE.md rewritten for core/engine/tui, HANDOFF open issues cut to the spec 10.7 limits, pty smoke timing margins for CI; cleanup complete, awaiting the user's review and merge
- 2026-09-28 cleanup task 6 done: Test quality and the pty hangup helper
- 2026-09-28 cleanup task 5 done: Saving and the debug log
- 2026-09-28 cleanup task 4 done: Input, board and panels
- 2026-09-28 cleanup task 3 done: Terminal and graphics fixes
- 2026-09-28 cleanup task 2 done: Engine fixes: king recaptures, record-only redaction, sanitised text
- 2026-09-28 cleanup task 1 done: Remove the legacy code and dependencies
- 2026-09-28 cleanup plan written: 7 tasks as git patches from a prototype reviewed for correctness and plan readiness, history rebuilt so each task's tests fail first; spec section 10 amended with the review rulings
- 2026-09-28 feat/tui-polish merged into main locally (f18dd32); merged result verified: tui:: 451, engine:: 112, whole crate only the 3 known old-code failures, pty smoke green
- 2026-09-28 tui-polish complete: tasks 1-8 applied and reviewed, final five-lens review fixed (Kitty font zoom, dev-profile Sixel crates, handoff minors) and re-reviewed; awaiting the user's manual test and merge
- 2026-09-28 tui-polish final fix wave: Kitty pictures follow a font zoom (spec 9.3 corrected); dev profile optimises `quantette`, `base64-simd` and `vsimd`; deferred review minors copied into this file
- 2026-09-28 tui-polish task 8 fix round 1: Kitty pictures built with session ids and deleted by id (`a=d,d=I,i=<id>`) on every restore path; the task 8 note now discloses the `report_log_failure` conflict; `tui::` 450 passed, `engine::` 112 passed
- 2026-09-28 tui-polish task 8 done: Whole-branch review fixes
- 2026-09-28 tui-polish task 7 done: Pseudo-terminal scenarios for the graphics query and debug mode
- 2026-09-28 tui-polish task 6 fix round 1: Status border DEBUG/mode-title precedence, existing debug-log file mode tightened to 0600, `RCHESS_DEBUG_LOG` `~` expansion, and a pending log failure found on the Menu no longer lost (verified review findings); `tui::` 421 passed, `engine::` 110 passed
- 2026-09-28 tui-polish task 6 done: Debug mode: exchange history, view and log
- 2026-09-28 tui-polish task 5 fix round 1: dev profile optimises the picture-encoding crates (verified review finding); `tui::` 372 passed
- 2026-09-28 tui-polish task 5 done: Draw pieces as pictures
- 2026-09-28 tui-polish task 4 fix round 1: non-unix `ask` no longer writes the graphics query it cannot read the answer to (verified review finding); `tui::` 355 passed
- 2026-09-28 tui-polish task 4 done: Graphics detection and the Image style
- 2026-09-27 tui-polish task 3 done: Full-screen layout with font-shaped squares
- 2026-09-27 tui-polish task 2 done: Engine: record the Jev exchange for debug mode
- 2026-09-27 tui-polish task 1 done: Piece images, dependencies and the picture compositor
- 2026-09-27 tui-polish plan written: 8 tasks as git patches from a prototype (per-task 3-lens review with skeptics, whole-prototype 5-lens review, 3 fix rounds), replayed three times on a fresh clone; spec section 9 amended with the rulings made while prototyping
- 2026-09-27 user smoke-tested the TUI: wants it to fill the terminal, bigger and clearer pieces, and a Jev debug view; tui-polish designed (spec section 9: full-screen layout, Cburnett piece images via ratatui-image, --debug exchange view and JSON-lines log)
- 2026-09-27 feat/tui merged into main locally (575c3f6); merged result verified: tui:: 293 passed, whole crate only the 3 known old-code failures, pty smoke green
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
