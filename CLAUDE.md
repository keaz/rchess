# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

The user's shell has `JEV_API_KEY` set. Run tests and the pty smoke test with both key variables
unset, so nothing can reach the Jev API:

```sh
cargo build
cargo run                               # the terminal UI; needs a real terminal
cargo run -- --debug                    # debug mode: Jev exchanges on `d` and in the debug log
env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test                  # whole suite
env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test tui::panels      # tests whose path contains a string
env -u JEV_API_KEY -u TYPESAFE_API_KEY cargo test -- --exact tui::input::tests::a_pasted_tab_becomes_a_space
cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py --no-build   # the TUI on a pty
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo bench                             # criterion perft benchmark
cargo run --release --example jev_eval  # Jev evaluation harness; needs the key, calls the API
```

`cargo test` is fully green. Three tests are `#[ignore]`d on purpose: the deep perft suite, the
search time budget (run it with `--release`) and a live Jev round trip (needs the key).

Snapshot tests (insta) live in `src/tui/snapshots/`. A mismatch writes a `.snap.new`; never accept
snapshots blindly (`INSTA_UPDATE=always`, `cargo insta accept`). Review each `.snap.new` by hand,
remove insta's `assertion_line:` line and move it over the `.snap`. `INSTA_UPDATE=no` stops the
`.snap.new` files being written.

`tests/cli.rs` (part of `cargo test`) runs the built binary without a terminal: `--version`/`-V`,
and `--help` or `--version` into a closed pipe, which exits 0 quietly.

`tests/pty_smoke.py` runs the real binary on pseudo-terminals (terminal setup and restore, signals,
hangups, the graphics query with scripted answers, Kitty/Sixel/iTerm2 pictures, debug mode, fault
injection through `RCHESS_FAULT` in debug builds). It gives the binary its own environment
without the keys. `--no-build` uses the existing debug binary; `--release` builds and runs the
release one. It prints `ALL CHECKS PASSED` or the failed checks. Its docstring lists every
scenario and explains the timing bounds.

## Architecture

Package `chess` (directory `rchess`), Rust 2024. `src/main.rs` only calls `chess::tui::run`.
`src/lib.rs` declares three layers, each built only on the ones before it:

- `core`: chess rules. Pure logic, no I/O, no trait objects.
- `engine`: the computer player (local search plus the Jev API).
- `tui`: the terminal UI (ratatui and crossterm).

The design is in `docs/superpowers/specs/2026-09-26-rchess-redesign-design.md` (sections 4-6, 9
and 10). Plans in `docs/superpowers/plans/` are historical and are not edited. Work in progress
is tracked in `docs/handoff/HANDOFF.md` (the protocol is spec section 8).

### core

- `Square` 0 = a1, 7 = h1, 63 = h8. `Bitboard` is a `u64` over those squares.
- Attack tables: leapers at compile time, sliders by magic bitboards. The magic numbers were
  found offline and are checked by an exhaustive test; do not regenerate them.
- `movegen`: fully legal generation from check and pin masks into a fixed-capacity
  `MoveList` (321, derived from the promotion material budget `from_fen` enforces).
- `Position` is `Copy`: `play` returns a new position, and undo is keeping the old one.
  FEN in and out, Zobrist hash, SAN and UCI move text (`san.rs`).
- `Game`: move history, outcomes (checkmate, stalemate, repetition, fifty-move rule,
  insufficient material, resignation), `undo`, and PGN output (`to_pgn`).
- `perft` plus `tests/core_properties.rs` (proptest) and `benches/perft.rs`.

### engine

- `eval`: material plus piece-square tables. `search`: fixed-depth negamax alpha-beta with
  quiescence; `analyse` scores every legal root move. `see`: static exchange evaluation,
  pin-aware.
- `annotate`: plain-language facts about each move plus a bucket from its score (Jev sees the
  words, never the numbers). `describe`: the position as the `state` object sent to Jev.
- `config`: `EngineConfig::from_env` (`JEV_API_KEY` or `TYPESAFE_API_KEY`, `JEV_MODEL`,
  `JEV_MAX_OPTIONS`, `JEV_FILTER_LOSING`); a bad value falls back to its default with a warning.
  The endpoint is the fixed `chess::engine::JEV_ENDPOINT`.
- `jev`: the HTTP client (ureq), retries, a 1 MiB body cap. With `trace` on (debug mode) it
  records each exchange with the API key redacted. Every transport test is offline, against a
  scripted server on 127.0.0.1.
- `player`: `ComputerPlayer::choose_move` plays forced moves and mate-in-one directly, otherwise
  asks Jev one `choice` question over a shortlist and vetoes blunders. It never fails: every
  problem (no key, HTTP errors, a bad answer) falls back to the local search's best move, noted
  in `ComputerMove.note`, with `MoveSource` saying where the move came from.

### tui

- `mod.rs`: `run`: command line (`--glyphs`, `--debug`, `--help`, `--version`), environment
  warnings for the menu, terminal set-up, the graphics query, the main loop.
- `app.rs`: the state machine. `App::handle` takes `AppEvent`s and returns `Action`s (start the
  engine, measure the font); it never blocks or spawns threads. Screens are Menu, Playing and
  GameOver with a dialog stack on top.
- `panels.rs`: draws every screen from `App`'s accessors and returns the `HitMap` for the next
  mouse event. `board.rs`: board layout, drawing, hit-testing, and `PieceImages`, the encoded
  piece pictures kept between frames.
- `glyphs.rs`: glyph sets (Image, Solid, Outline, Ascii), palettes, `NO_COLOR`, and the width
  rules (`char_width`, `shorten`): every glyph is exactly one cell, which the tests enforce.
- `pieces.rs`: the embedded Cburnett PNGs (`assets/pieces/`) and the picture compositor.
- `graphics.rs`: the start-up graphics query (Kitty, Sixel, iTerm2 or half-blocks, and the font
  size). It reads stdin itself on the UI thread with a 1 s deadline and no thread, then drains
  late answers for up to 200 ms so they never become keys. `FontMeter` measures the font again
  after a resize (font zoom).
- `terminal.rs`: raw mode, alternate screen, mouse and paste modes; `leave` restores all of it on
  every exit path (normal, `?` error, panic hook, SIGINT/SIGTERM/SIGHUP, a stuck UI after 1 s, a
  closed terminal without SIGHUP); Kitty picture ids and their deletion.
- `event.rs`: collects terminal input and engine replies into batches. `worker.rs`: one
  "engine" thread per computer turn; replies carry a generation counter and position hash so
  stale ones are dropped; an engine panic becomes a local-search move.
- `input.rs`: the command-box editor and command parser; `movetext.rs`: lenient move text
  (SAN, UCI, long algebraic, loose SAN).
- `files.rs`: saving FEN and PGN (`~` expansion, atomic write, no silent overwrite).
- `debug.rs`: debug mode (`--debug` or `RCHESS_DEBUG`): the last 50 Jev exchanges, the
  full-screen exchange view (`d`), and a JSON-lines log (`RCHESS_DEBUG_LOG`, default
  `$XDG_STATE_HOME/rchess/jev-debug.jsonl` or `~/.local/state/rchess/jev-debug.jsonl`, mode 0600)
  written by a `debug-log` thread through a queue of 64 records.
- `test_support/`: test-only builders, a scripted `FakeEngine` and a `TestBackend` harness that
  drives `App` the way the run loop does.

Functions that read the environment take a `get` closure (`|k| std::env::var(k).ok()` in
production), so tests pass their own values instead of touching the process environment.

## Invariants and traps

- `pub mod core` shadows the built-in `core` crate at the crate root: write `::core::` in
  `src/lib.rs`.
- `Position::play` and `to_san` need a move from `legal_moves()`; debug builds assert, release
  builds may corrupt the position. Use `Game::play` for unvalidated input.
- `Game::undo` first withdraws a pending resignation and returns `None`; check `outcome()`.
- `parse_san` is strict; leniency belongs to `tui::movetext`.
- The en passant square (and the hash) is set whenever an enemy pawn attacks it, even when that
  capture is illegal because of a pin, so a rare threefold repetition can be missed.
- `choose_move` can take about 19 s (3 attempts × 5 s timeout plus backoff and search) and cannot
  be cancelled: give the worker a `Game` clone (the search needs the history) and drop stale
  replies by generation.
- Debug builds search about 23× slower, so `Cargo.toml` builds `chess` at opt-level 3 in the dev
  profile, and also the picture-encoding crates (a test checks that list). A crate added to the
  encoding path needs its own `[profile.dev.package]` entry, or debug builds stall on pictures.
- Never enable TRACE logging for `ureq` or `ureq_proto`: it prints the `Authorization` header
  with the key. Never print or log the key; the engine redacts it in everything it records.
- Jev gets object keys in alphabetical order (serde_json without `preserve_order`, by ruling).
- Nothing may panic while restoring the terminal: after a hangup every write fails, and
  `eprintln!` panics on a failed write. `leave` ignores errors and runs every step.
- The app hit-tests mouse events against the `HitMap` of the last draw; the run loop redraws
  before a mouse event, key or paste that follows a layout change in the same batch.

## Logging

There is no logger. The crate uses only the `log` macros (the panic hook and the engine worker),
so nothing is printed unless a binary installs a logger; `RUST_LOG` does nothing. The TUI owns the
terminal, so anything written to it would corrupt the screen.

## CI

`.github/workflows/rust.yml` runs on pushes and pull requests to `main` (ubuntu-latest, Rust 1.98
with rustfmt and clippy, cached): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, `cargo build`, then `python3 tests/pty_smoke.py --no-build`, each with
`JEV_API_KEY` and `TYPESAFE_API_KEY` unset. It uses no secrets. Run the same commands locally
before pushing. The toolchain is pinned so that a new clippy lint cannot turn CI red on its own;
bumping it is a deliberate change that fixes whatever the new version finds.

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
