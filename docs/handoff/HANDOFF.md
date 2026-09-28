# rchess Redesign — Handoff

Read this first. Follow the protocol in section 8 of
`docs/superpowers/specs/2026-09-26-rchess-redesign-design.md`.

## Current
Sub-project: cleanup (spec section 10) | Plan: docs/superpowers/plans/2026-09-28-cleanup.md
Branch: chore/cleanup (spec section 10 and the plan committed; no code yet). main has feat/tui-polish merged locally at f18dd32, not pushed.
Last completed task: cleanup plan written from a reviewed, restructured prototype and replayed twice on a fresh clone (every patch applies, tests patches fail as stated, counts 614/621/641/658/684/687/687, clippy -D warnings clean, pty smoke green)
Next task: user reviews the plan and picks an execution method; then cleanup task 1
State: green (except 3 pre-existing old-code test failures, removed by cleanup task 1)

## Verify before continuing
env -u JEV_API_KEY -u TYPESAFE_API_KEY INSTA_UPDATE=no cargo test --lib tui:: && env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test --lib engine::
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
- Engine follow-up (2026-09-27): SEE judges the first capturer by the pins of the position as it stands and the recaptures by the pins after the first capture (pins changed by later captures are still not modelled); `new_attack`'s "defended" check is pin-aware. The Jev endpoint is fixed, not configurable (user decision): `jev::JEV_ENDPOINT` = `https://api.typesafe.ai/v1/systemone`, the TypeSafe quickstart URL, with `Authorization: Bearer <key>` and `Content-Type: application/json`; an offline test pins that wire format. Test counts: `cargo test --lib engine::` 93 passed, 2 ignored; whole lib 199 passed, 3 failed (old code), 3 ignored.
- Follow-up review fixes (2026-09-27): in SEE a king captures only onto a square no enemy piece attacks, pinned or not (a pinned piece still guards against a king), so `capture_gain(d5)` in `4k3/8/4b3/3n4/2K5/8/8/4R3 b` and `see(d1d4)` in `3rk3/8/8/4b3/3r4/2K5/8/3RQ3 w` are now 0; `JEV_ENDPOINT` is re-exported as `engine::JEV_ENDPOINT` (no engine rustdoc warnings); the wire-format test compares body keys as a set and the endpoint test checks host and path. Test counts: `cargo test --lib engine::` 96 passed, 2 ignored; whole lib 202 passed, 3 failed (old code), 3 ignored.
- Final-review fix wave: a mate on the 100th half-move now outranks the fifty-move rule; SEE ignores pinned pieces off their pin line; annotation facts corrected ("exposed to capture" wording, no such fact for the capturing piece, a king only attacks undefended pieces); base-URL variable validated (since removed by the engine follow-up); transport errors classified (new `JevError::Request`), 1 MiB body cap and offline TcpListener tests; notes stripped of control characters; public docs, `ComputerPlayer` Debug and `MoveSource` Display; harness `jev pick` column and answered-only latency.
- TUI task 4: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 104 passed (brief said 103); the extra test is from task 3's fix round 1, which added a test after the brief text was written.
- TUI task 5: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 140 passed (brief said 139); the extra test carries forward from task 4's count above (104 baseline + 36 new tests in worker.rs/event.rs/terminal.rs = 140).
- TUI task 6: implemented as specified in the brief, no deviations. `INSTA_UPDATE=no cargo test --lib tui::` reports 251 passed (brief said 250); the extra test carries forward from task 5's count above (140 baseline + 77 new tests in app.rs + 34 new tests in panels.rs = 251).
- TUI task 3 fix round 1: `movetext.rs`'s `loose_match` no longer short-circuits on the first loose-SAN or piece-spelling match; it now always also computes the promotion-without-piece candidates (renamed `missing_promotion` to `promotion_candidates`, returning candidates rather than a `Result`) and merges them in, so a pawn promotion typed without its piece (`bxc8`) that also matches another piece's move on the same square (`Bxc8`) is reported `MoveTextError::Ambiguous` (`Bxc8`, `bxc8=?`, the latter collapsing all four promotion pieces via new helper `promotion_family_labels`) instead of silently playing the other piece's move. Deviation from the plan's code (the brief's `loose_match`/`missing_promotion` are restructured), by fix-round ruling to close a verified review finding.
- TUI final-review fix wave (2026-09-27): A1 command box above the game-over overlay; A2 NO_COLOR board marks; A3 status messages and save errors fitted with `…`, paths shown with `~` and shortened in the folder part; B1 a computer move clears only turn notes; B2 Save dialog unquotes paths; B3 promotion picker Esc gives typed text back; B4 watchdog turns raw mode off before its writes; B5 stdin checked before setup; B6 move list columns; B7 pty smoke test uses cargo's target dir and checks stdin; B8/B9 run-loop tests. Test counts: `INSTA_UPDATE=no cargo test --lib tui::` 291 passed; whole lib 493 passed, 3 failed (old code), 3 ignored; `core_properties` 2 passed. Snapshots changed only by B6 (move rows of game_over_80x24, jev_reply_80x24, jev_vetoed_80x24, mid_game_80x24). Follow-up: the Status message budget counts wrapped rows (a game-over line that wraps no longer cuts a save message to `saved`), and the NO_COLOR capture/check marks survive the keyboard cursor; `tui::` 293 passed.
- Deliberate deviations from spec 6 wording on feat/tui (no code change planned):
  - 6.2/6.4: while only "Terminal too small" is shown, all input is ignored except Ctrl+C, which quits at once without the confirmation (it could not be seen) (`src/tui/app.rs:1036`).
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
- src/tui/board.rs:874 full-board rendering is covered by the 120×40 snapshot.
- src/tui/panels.rs:564 `fit_message` and `cut_to_fit` are quadratic in the message length and run every frame, so a very long pasted save path makes the UI lag while its error is shown.
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

### tui-polish (known limits)
- src/tui/graphics.rs: iTerm2 gets the Sixel protocol when it answers the sixel probe (the iTerm2 environment hint only decides when no probe answers).
- src/tui/graphics.rs: on terminals that answer more than 1 s late, keys typed while a font measurement waits are lost, and a stale font can stay until the next resize; a late start-up answer from WezTerm or Konsole (no device-attributes probe) can hold crossterm's read until the next input.
- src/tui/graphics.rs: a quit signal during the start-up query leaves the terminal's answers in the tty input queue for the shell.
- src/tui/terminal.rs: Kitty pictures are deleted at exit with one delete-by-id command for every picture built in the session (each square-size or font change builds new ones), so a long session with many resizes writes that many commands on exit.
- src/tui/board.rs: pictures are composited and encoded on the UI thread on the first Image frame and after every square-size or font change (about 0.1-0.2 s for a Sixel board at 300x100 with the dev-profile overrides, similar in release); a crate added to the encoding path later needs its own `[profile.dev.package]` override or debug builds stall again.
- src/tui/board.rs: half-block pictures are written in 24-bit colour even on the 256-colour palette (ratatui-image behaviour).
- src/tui/app.rs: quitting while a Jev answer is held (watch mode paused) drops that exchange from the history and the log.
- src/tui/debug.rs: non-UTF-8 values of RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME are skipped without a warning, and a relative HOME is accepted.
- src/tui/glyphs.rs: RCHESS_IMAGES turns images off only for the value `off`; other values are ignored without a warning.
- src/tui/panels.rs: menu warnings that do not fit at 60x20 are dropped without notice (pre-existing).

### tui-polish (deferred minors from reviews)
Copied from the git-ignored SDD ledger (.superpowers/sdd/2026-09-27-tui-polish/progress.md); line numbers are current as of the final fix wave.
- src/engine/jev.rs:497 the response body is redacted before it is parsed, so the untraced path changes too, and a key that is a substring of a valid answer corrupts the parse.
- src/engine/jev.rs:288 a key echoed with `\u` escapes inside a body that is not valid JSON is not redacted (`\/` is handled).
- src/engine/jev.rs:1057 `traced_exchange_records_a_failed_connection_without_a_status` binds port 0, drops the listener and assumes nothing listens there; a parallel test's `serve()` could get the same port.
- src/engine/jev.rs:349 the private `struct Attempt` is named almost like the public `JevAttempt` it feeds.
- src/tui/panels.rs:1846 `every_square_is_hit_at_every_size` also checks a click-to-move and a drag-to-move at one fixed size, which its name does not promise.
- src/tui/graphics.rs:486 if the 1 s deadline falls partway through the kitty answer, its unread tail reaches crossterm as key presses; `LateAnswers` only drops a run that starts with Alt+`_` then `G`.
- src/tui/graphics.rs:462 with neither a cell-size answer nor a window pixel size, `interpret` downgrades a detected Kitty or Sixel answer to half-blocks; spec 9.3 says the picker is built for the detected protocol at 10x20.
- src/tui/board.rs:275 clearing the picture cache leaves the dropped Kitty pictures in the terminal's image store until exit (they are deleted by id then).
- src/tui/board.rs:317 an error from `new_protocol` (and from `Kitty::new`, line 333) is cached as `None` for good and never logged.
- src/tui/debug.rs:221 `BodyLine::rows` rewraps a line holding a wide or zero-width character on every frame, allocating the whole line each time.
- src/tui/debug.rs:698 when `close` times out mid-write, the process can exit in the middle of a record, and the next session appends after half a line.
- tests/pty_smoke.py:939 `scenario_query_hangup` never checks the "at once" timing its docstring promises; any exit by SIGHUP within 5 s passes.
- tests/pty_smoke.py:939 `scenario_query_hangup` duplicates the hangup-detection loop of `scenario_hangup` (line 620) instead of sharing a helper.

### tui-polish (minors from the final whole-branch review, 2026-09-28; triaged as can wait)
- src/tui/graphics.rs:192 A late start-up answer that reaches crossterm in more than one read freezes the UI until the next key. A split right after its ESC turns the answer into key presses that start a game.
- src/tui/graphics.rs:233 Every start-up on a non-unix build shows the menu warning 'graphics query: unsupported; images use half-blocks', although no query was attempted and the user cannot fix it.
- src/tui/debug.rs:144 Each exchange keeps a pre-rendered copy of every response body: pretty-printed JSON, one heap-allocated BodyLine per line. That makes the history's real limit 30-150x larger than the 1 MiB cap x 3 attempts x 50 it appears to have.
- src/tui/debug.rs:630 The record channel to the debug-log thread is unbounded. While that thread is blocked, every exchange of the session stays in memory, not just the 50 in History, and no failure is ever reported.
- src/tui/debug.rs:748 open_log follows a symlink (or hard link) at the log path. It appends JSON to the link's target and changes the target's mode to 0600, while the link itself is left in place.
- src/tui/app.rs:1639 An answer held while Jev vs Jev is paused keeps its exchange out of the history, the view and the log until play resumes. If the person quits while paused, the exchange is never logged. When the exchange is recorded later, its log `time` is when it was released, not when the reply arrived.
- src/tui/debug.rs:457 ←/→ (and `d`, through `ExchangeView::open`) reset `page` and `max_scroll` to 0 until the next draw. End, PgDn and ↓ that arrive in the same input batch after them do nothing.
- src/tui/app.rs:1303 `d` on the game-over overlay does nothing and shows no message. To see the exchange behind Jev's game-ending move, the person must first press Esc (see the board), then `d`.
- src/tui/app.rs:1970 On a terminal without a graphics protocol (half-blocks), `g` still offers `image`, but at ordinary window sizes it looks exactly like Solid while the status says "glyphs: image". Half-block pictures need 11×5 squares, which takes a window of about 123×46 cells or more with a 10×20 font.
- src/tui/glyphs.rs:446 `RCHESS_IMAGES` treats only `off` as off. Values like `0`, `false` or `no` silently leave images on and still send the graphics query, unlike `RCHESS_DEBUG`, where `0` means off. No menu warning names the unrecognised value, as happens for `RCHESS_GLYPHS`.
- docs/handoff/HANDOFF.md:14 Several HANDOFF sections are out of date or weaker than on main: the verify command, the manual smoke checklist, one test count, and the line references in older deferred minors.
- src/tui/graphics.rs:462 Two deliberate departures from spec 9 are not written down in the spec or the HANDOFF deviation list.
- src/tui/app.rs:5206 `a_log_failure_found_on_the_menu_is_shown_once_a_game_starts` waits a fixed 20 x 5 ms for the debug-log thread to fail, so it depends on timing.
- src/tui/app.rs:5108 The Task 8 three-way merge left duplicated tests, one redundant field, and an inlined guard with no test. There is no dead code.
- The Task 7 ledger minor (no Sixel-answering pty scenario) is resolved: tests/pty_smoke.py has scenario_query_sixel_zoom.
- Kitty now also measures the font on each resize (spec 9.3), so the slow-terminal limits of the font measurement apply to Kitty too.

## Open questions for user
- None.

## Log (newest first)
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
