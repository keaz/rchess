# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build                 # build
cargo run                   # launch the terminal UI (chess::tui); needs a real terminal
cargo test                  # full suite
cargo test pieces::rook     # all tests in one module (substring filter)
cargo test -- --exact pieces::queen::test::test_possible_moves   # one test
cargo test -- --nocapture   # show println!/log output
cargo fmt && cargo clippy
cargo build && env -u JEV_API_KEY -u TYPESAFE_API_KEY python3 tests/pty_smoke.py   # chess::tui on a pty
```

Logging: tests call a local `init()` that installs `env_logger`. The `[env] RUST_LOG = "debug"`
key in `Cargo.toml` is **not** a valid manifest key (cargo warns and ignores it), so set the level
yourself: `RUST_LOG=debug cargo test -- --nocapture`.

CI (`.github/workflows/rust.yml`) runs only `cargo build --verbose` and `cargo test --verbose` on
push/PR to `main`. Note that `cargo test` is currently **red on `main`** — 3 of 58 tests fail
(`test::test_from_index_invalid_upper`, `ai::test::test_generate_move`,
`pieces::queen::test::test_possible_moves`). Don't assume a failure you see is yours; check
against a clean tree first.

## Architecture

Package name is `chess` (directory is `rchess`). `src/main.rs` is a 5-line shim — all logic lives
in the library, so `use chess::...` in tests and binaries.

### Flat index board — the central mental model

`Board` holds a single `Vec<Square>` of 64 entries. `Position { x: char, y: i8 }` maps to it via
`Position::to_index() = (x - 'a') + (y - 1) * 8`, so a1 = 0, h8 = 63, rank 1 is the low end.

Every piece's move logic is index arithmetic on that flat vector, not (file, rank) deltas:

| delta | meaning |
| --- | --- |
| ±8 | one square along the file (up/down) |
| ±1 | one square along the rank (left/right) |
| ±7, ±9 | diagonals |

Consequences that bite: `±1` wraps across rank boundaries (h4 + 1 = a5), and diagonal deltas wrap
similarly. Sliding generators guard this by watching `index % 8`; several do not guard both ends.
`Position::new` and `Position::from_index` **panic** on out-of-range input, so an unguarded
candidate index aborts the whole move generation rather than returning a short list. Both current
`possible_moves` failures are this bug class. Fix by bounds-checking before constructing a
`Position`, not by relaxing the panics — callers rely on `Position` always being legal.

### Piece dispatch

`PieceType` is a `Copy` enum whose variants carry `(Color, Position)` — plus `bool is_first_move`
for `Pawn`. The `Piece` trait (`move_to` / `can_move_to` / `color` / `possible_moves`) is
implemented **once**, on `PieceType`, and each arm forwards to a free function in
`src/pieces/<piece>.rs`. Every piece module exports the same trio:

- `move_to(&PieceType, to, &mut dyn BoardTrait) -> Result<Option<PieceType>, ChessError>`
  — validates via `can_move_to`, then mutates the board and returns the captured piece.
- `can_move_to(&current_position, &color, to, &dyn BoardTrait) -> Result<(), ChessError>`
  — pure validation; the error variant is the reason (`InvalidMove`, `BlockedMove`,
  `InvalidCapture`, `UnSafeKing`, ...).
- `possible_moves(&current_position, &color, &dyn BoardTrait) -> Vec<Position>`

Pawn is the exception: its `move_to` is named `pawn_move_to`, and its `can_move_to` /
`possible_moves` take an extra `is_first_move` argument. Queen is composed — it delegates
straight to `bishop::possible_moves` + `rook::possible_moves`, and its `can_move_to` reuses
`bishop::bishop_move` / `rook::rook_move` for the blocking scan.

Adding or changing a piece means touching its module plus the four `match` arms in
`src/pieces/mod.rs` (`move_to`, `can_move_to`, `color`, `possible_moves`).

### Position is stored twice

A piece's square index is implied by its slot in `squares`, *and* duplicated inside the
`PieceType` variant. `move_to` must clear the origin square and write a **freshly constructed**
variant carrying the destination (`Some(PieceType::Rook(*color, position))`) — copying the old
value forward silently desynchronizes the two, and `is_check` / `evaluate` read the embedded
position.

### Object-safe traits + clone-for-speculation

`BoardTrait` and `Piece` are used as `dyn`, so cloning goes through the `CloneAsBoard` /
`CloneAsPiece` helper traits (blanket-implemented for any `Clone` implementor) exposing
`clone_as_a() -> Box<dyn ...>`. That is how speculative evaluation works: clone the board, apply
a candidate move to the clone, evaluate, discard. See `ai::generate_move` and
`king::can_king_move_safe_position`.

The concrete `Board` struct is private. Construct boards only via `board::new_board()` (full
starting position) or `board::empty_board()` (all 64 squares empty). Tests overwhelmingly use
`empty_board()` and place pieces directly:

```rust
let mut board = board::empty_board();
board.square_mut(&Position::new('e', 8)).piece = Some(PieceType::King(Color::Black, Position::new('e', 8)));
```

`ai.rs` also defines a hand-rolled `MockBoard` in its test module with `todo!()` for unneeded
methods — `mockall` is a declared dependency but not actually used anywhere yet.

### Evaluation and AI

`BoardTrait::evaluate(color)` is relative material: `+value` for own pieces, `-value` for the
opponent's, so a fresh board scores 0. King value is `u8::MAX` (255), which will dominate any sum
it enters — don't treat `evaluate` as a general position score.

`ai::generate_move` is a single-ply greedy search seeded with `best_score = 0`, so it returns
`None` unless some move gains material. It is not yet wired into the game loop — `Game::play` is
human-vs-human.

### Known traps in the current game loop

- `king::can_king_move_safe_position` returns `true` when the king has **no** safe escape — the
  body returns `false` early on finding one. `Game::play` depends on that inverted sense to
  declare checkmate. Rename or invert deliberately and update both call sites together.
- `Game::play` calls `game.board.clone_as_a().move_piece(...)`, mutating a throwaway clone, so
  accepted moves never reach `game.board`.
- Move input is 4 raw chars (`e2e4`) parsed with `unwrap()`; anything shorter or non-numeric
  panics.
- `drawille` is a declared dependency for terminal rendering, but no board-drawing code exists —
  `Game::play` never prints the position.

## graphify

This project has a graphify knowledge graph at graphify-out/.

Rules:
- Before answering architecture or codebase questions, read graphify-out/GRAPH_REPORT.md for god nodes and community structure
- If graphify-out/wiki/index.md exists, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code files in this session, run `graphify update .` to keep the graph current (AST-only, no API cost)
