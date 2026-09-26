# rchess Redesign — Design Spec

Date: 2026-09-26
Status: Approved in brainstorming, pending written-spec review

## 1. Goals

1. Replace the move generation and validation core with a correct, fast, idiomatic Rust
   implementation based on bitboards.
2. Replace the greedy material-score computer player with a player whose move decisions come
   from TypeSafe's Jev model, with code owning legality and tactical facts.
3. Replace the stdin/println game loop with a full terminal UI built on `ratatui`.
4. Make every unit of work resumable by another agent through a single handoff file.

Non-goals: a strong standalone chess engine, UCI protocol support, network play, opening books,
endgame tablebases.

## 2. Current State (as of commit 99b04ba)

- ~3,000 lines. `Board` is a flat `Vec<Square>` behind `dyn BoardTrait`; pieces are a
  `PieceType` enum dispatching to per-piece modules using flat-index arithmetic.
- Known defects: file-wrap bugs in sliding and diagonal generation, panicking `Position`
  constructors on out-of-range candidates, 3 of 58 tests failing, no castling / en passant /
  promotion / draw rules, `Game::play` mutates a throwaway board clone so moves never apply,
  inverted meaning of `can_king_move_safe_position`.
- No TUI: plain `println!` and `stdin`. `drawille` declared but unused.
- `ai::generate_move`: single-ply greedy material search.
- No LLM-related code exists in the repository. "Remove LLM code" reduces to deleting the old
  greedy AI.

## 3. Architecture

```
src/
  lib.rs            re-exports
  main.rs           launches the TUI
  core/             sub-project 1: pure logic, no I/O
    bitboard.rs     Bitboard(u64) newtype, bit ops, square iterator
    square.rs       Square(u8) 0..64, File, Rank, parse/display
    piece.rs        Color, PieceKind, Piece
    attacks.rs      const leaper tables; magic-bitboard slider tables
    position.rs     Position, FEN parse/emit, Zobrist hashing
    movegen.rs      legal move generation into MoveList
    mv.rs           Move(u16) packing; UCI and SAN parse/format
    game.rs         Game: history stack, undo, outcome detection, PGN export
    perft.rs        perft for correctness tests and benchmarks
  engine/           sub-project 2
    search.rs       alpha-beta (depth 3) + quiescence, material + piece-square eval
    annotate.rs     converts per-move search facts into plain-language tags
    jev.rs          Jev HTTP client, request building, answer parsing, retries
    player.rs       ComputerPlayer: annotate, shortlist, ask Jev, veto, fallback
  tui/              sub-project 3
    app.rs          App state machine (Menu, Playing, GameOver, dialogs)
    board_widget.rs board rendering, highlights, flip
    panels.rs       status, Jev panel, move list, captured pieces
    input.rs        mouse, keyboard, command box parsing
    worker.rs       background thread for computer moves
```

Dependency direction is strictly `tui -> engine -> core`. `core` has no external runtime
dependencies (except `thiserror`), no I/O, and no trait objects.

### Crates

| Purpose | Crate |
| --- | --- |
| TUI | `ratatui`, `crossterm` |
| HTTP (sync, used on a worker thread) | `ureq` with JSON feature |
| JSON | `serde`, `serde_json` |
| Errors | `thiserror` |
| Tests | `proptest`, `insta` |
| Benchmarks | `criterion` |

Removed: `drawille`, `mockall`, the invalid `[env]` manifest key.

## 4. Sub-project 1 — `core` move generation

### 4.1 Data

- `Position { pieces: [Bitboard; 6], colors: [Bitboard; 2], side: Color, castling: CastleRights(u8), ep: Option<Square>, halfmove: u16, fullmove: u16, hash: u64 }`. `Position` is `Copy`.
- `hash` is a Zobrist key updated incrementally in `play`.
- `Move(u16)`: 6 bits from, 6 bits to, 4 bits flags (quiet, double push, king castle, queen
  castle, capture, en passant, promotion to N/B/R/Q with or without capture).
- `MoveList`: fixed `[Move; 256]` plus length. No heap allocation during generation.

### 4.2 Generation

- Knight, king and pawn attack tables are built at compile time with `const fn`.
- Sliders use magic bitboards. Magic numbers are hardcoded; attack tables are built once at
  first use through `std::sync::LazyLock`. No build script.
- Generation is fully legal (not pseudo-legal plus filter): compute checkers and pin masks once
  per position. Must handle double check (king moves only), pinned pieces restricted to their
  pin ray, castling through or out of attacked squares, and the en passant horizontal
  discovered-check case.
- `Position::play(&self, Move) -> Position` returns a new copy. Undo is a history stack of
  positions held by `Game`.

### 4.3 Public API

- `Position::from_fen`, `to_fen`, `legal_moves() -> MoveList`, `play`, `is_check`,
  `piece_at(Square)`.
- `Move::to_uci`, `Position::parse_uci`, `Position::parse_san`, `Position::to_san`.
- `Game::new`, `Game::from_fen`, `play(Move) -> Result<(), ChessError>`, `undo`,
  `outcome() -> Option<Outcome>`, `history()`, `to_pgn`.
- `Outcome` covers checkmate, stalemate, fifty-move rule, threefold repetition (via Zobrist
  hash), insufficient material, and resignation.
- `ChessError` (via `thiserror`): `InvalidFen`, `IllegalMove`, `ParseMove`. Nothing panics on
  external input; fallible constructors use `TryFrom` / `Result`.

### 4.4 Verification

- Perft against the six standard positions from the chessprogramming wiki (start position,
  Kiwipete, positions 3–6) to depth 4–5 in `cargo test`; deeper depths behind `#[ignore]`.
- Unit tests for FEN round-trip, SAN round-trip, every special move and every draw rule.
- `proptest` random legal game walks: FEN round-trip equality and incremental hash equals
  from-scratch hash at every ply.
- Speed target: perft(6) from the start position (119,060,324 nodes) in about 2 seconds or less
  in release mode on a laptop, measured with `criterion`.

## 5. Sub-project 2 — `engine` (Jev computer player)

### 5.1 Jev facts relied on

- Endpoint `POST https://api.typesafe.ai/v1/systemone`, header `Authorization: Bearer <key>`,
  model alias `jev-latest`.
- `choice` question: up to 255 options, response includes `choice`, `probabilities` per option
  and `confidence`.
- Errors: 401 (bad key), 422 (validation), 429 (rate limit), 529 (overloaded). Retry 429 and
  529 with exponential backoff.
- Documented weaknesses (jev-1.13): arithmetic, counting, multi-hop reasoning, large state full
  of irrelevant detail. Therefore code owns legality, tactics and numbers; Jev gets
  plain-language facts and makes the judgment.

### 5.2 Turn flow

1. `core` produces all legal moves.
2. `search.rs` scores every root move with alpha-beta depth 3 plus quiescence (material +
   piece-square tables) and computes static exchange evaluation for captures.
3. `annotate.rs` converts results into words per move, for example "captures undefended
   knight", "gives check", "checkmates", "loses the queen for a pawn", "castles kingside",
   "promotes to queen", "leaves bishop on e5 attacked and undefended", "equal trade", plus a
   bucket: `winning`, `good`, `neutral`, `bad`, `losing`.
4. Shortlist: when `JEV_FILTER_LOSING` is true, drop `losing` moves if any non-losing move
   exists. Then keep the top `JEV_MAX_OPTIONS` moves by search score. If every move is
   `losing`, send all of them (still capped).
5. Send one Jev `choice` question (shape in 5.3).
6. Veto: if Jev's chosen move scores more than 150 centipawns below the search best move, play
   the highest-probability option within that threshold instead, and record the veto.
7. Forced shortcuts, no API call: exactly one legal move; a mate-in-1 exists.

### 5.3 Request shape

```json
{
  "model": "jev-latest",
  "state": {
    "side_to_move": "Black",
    "phase": "middlegame",
    "material": "White up one pawn",
    "white_pieces": "King g1, Queen d1, Rook a1, ...",
    "black_pieces": "King g8, ...",
    "last_move": "White played Nf3xe5, capturing a pawn",
    "threats_against_us": "Our queen on d8 is attacked by the bishop on b5"
  },
  "questions": {
    "move": {
      "type": "choice",
      "instructions": "You play `side_to_move`. Pick the strongest chess move. Prefer moves that win material, deliver checkmate, or remove threats against our pieces.",
      "criteria": { "Nxe5": "captures undefended knight; good", "O-O": "castles kingside; neutral" }
    }
  }
}
```

Option keys are SAN strings, which are unique within a position.

### 5.4 Client and configuration

- `trait MoveChooser { fn choose(&self, req: &ChoiceRequest) -> Result<ChoiceAnswer, JevError>; }`
  with `JevClient` (over `ureq`) as the real implementation and a mock in tests.
- Environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `JEV_API_KEY` | none | Required for Jev play. Absent means local-search fallback. |
| `JEV_MODEL` | `jev-latest` | Model alias. |
| `JEV_BASE_URL` | `https://api.typesafe.ai` | Override for testing. |
| `JEV_MAX_OPTIONS` | `40` | Shortlist cap, clamped to 1..=255. |
| `JEV_FILTER_LOSING` | `true` | Drop `losing` moves when alternatives exist. |

- Request timeout 5 seconds. Up to 3 attempts with exponential backoff on 429 / 529 and
  transport errors. No retry on 401 / 422.
- The API key is never logged and never rendered in the TUI.

### 5.5 Fallback

Missing key, network failure, or exhausted retries: play the search best move. The TUI shows
"Jev unavailable — local search".

### 5.6 Output

`ComputerMove { mv, source: Jev | Forced | Vetoed | Fallback, top3: [(san, probability)], confidence, latency_ms }`.

### 5.7 Verification

- Mock-chooser tests for shortlist filtering and cap, veto, forced shortcuts, fallback paths,
  SAN key mapping, and env config parsing.
- Golden annotation tests on fixed FENs (hanging piece, fork, mate-in-1, promotion).
- One `#[ignore]` live test against the real API.
- Cost estimate: about 1–2k input tokens per move at $0.042 per million — negligible.

## 6. Sub-project 3 — `tui`

### 6.1 Screens

- `Menu`: Human vs Human; Human vs Jev (choose White, Black or random); Jev vs Jev (watch mode
  with adjustable step delay); Load FEN; Quit.
- `Playing`: main screen (layout below).
- `GameOver` overlay: result and reason; new game, save PGN, back to menu.
- Dialogs: FEN input, save path for FEN/PGN, promotion picker (Q/R/B/N), resign confirmation.

### 6.2 Playing layout

```
┌ Board ─────────────────┐┌ Status ──────────────┐
│ 8 ♜ ♞ ♝ ♛ ♚ ♝ ♞ ♜      ││ Black to move · Check│
│ ...  (8x8, 2-color sq) ││ Jev thinking... 0.3s │
│ 1 ♖ ♘ ♗ ♕ ♔ ♗ ♘ ♖      │├ Jev ─────────────────┤
│   a b c d e f g h      ││ Nf6 62% Bb4 21% d5 9%│
└────────────────────────┘│ source: Jev · conf .81│
┌ Command ───────────────┐├ Moves ───────────────┤
│ > e2e4 / Nf3 / :help   ││ 1. e4   e5           │
└────────────────────────┘│ 2. Nf3  Nc6 ...      │
                          ├ Captured ────────────┤
                          │ W: ♟♟  B: ♘          │
                          └──────────────────────┘
```

- Board scales with terminal size; each square at least two columns wide. Unicode pieces by
  default, `--ascii` flag for fallback glyphs.
- Highlights: selected piece, its legal targets, last move, king in check.

### 6.3 Input

- Mouse: click a piece, then click a target square.
- Keyboard: arrow keys move a cursor, Enter selects/confirms, Esc cancels.
- Command box (focus with `/` or `:`): UCI (`e2e4`) or SAN (`Nf3`) moves, and commands
  `:undo`, `:flip`, `:new`, `:fen <FEN>`, `:savefen <path>`, `:savepgn <path>`, `:resign`,
  `:quit`, `:help`.
- Hotkeys: `u` undo, `f` flip, `n` new game, `q` quit.
- Undo against Jev reverts to the human's previous turn (two plies). Undo is disabled while a
  computer move is in flight.

### 6.4 Concurrency

- Main thread runs the event loop, polling `crossterm` events on a 50 ms tick, and redraws.
- A computer turn spawns a worker thread with a `Position` copy; the result returns over
  `std::sync::mpsc`. Each request carries a generation counter; results whose generation no
  longer matches (after undo or new game) are discarded.
- The UI thread never blocks on the network.
- A panic hook restores the terminal (leave raw mode and alternate screen) before printing.

### 6.5 Verification

- `App` logic tested headlessly by feeding input events and asserting state.
- Render snapshot tests with ratatui `TestBackend` and `insta`.

## 7. Delivery Plan

Sub-projects run sequentially; each is merge-ready before the next starts.

1. `core` — done when the perft suite passes and the benchmark target is met.
2. `engine` — done when mock tests pass and the ignored live test passes once.
3. `tui` — done when all three game modes are playable end to end.
4. Cleanup — delete `src/pieces/`, `src/board.rs`, `src/ai.rs` and the old `Game`; remove
   `drawille`, `mockall` and the `[env]` key; update `CLAUDE.md`; extend CI with
   `cargo fmt --check` and `cargo clippy -- -D warnings`.

New code lives in new modules beside the old code until step 4, so `main` never breaks.

Git: one branch per sub-project (`feat/core-bitboards`, `feat/jev-engine`, `feat/tui`,
`chore/cleanup`), one commit per plan task. The user merges.

## 8. Handoff Protocol

Documents:

- This spec.
- Plans: `docs/superpowers/plans/2026-09-26-<subproject>.md`, one per sub-project, checkbox
  tasks.
- Handoff: `docs/handoff/HANDOFF.md`, a single living file.

Rules for every agent:

1. Read `docs/handoff/HANDOFF.md` first.
2. Run the commands under "Verify before continuing" and confirm the state matches.
3. Continue at "Next task".
4. At the end of every task, and whenever work stops (done, blocked, context limit,
   interrupted), update `HANDOFF.md` and commit it with the task.
5. After modifying code, run `graphify update .` as required by `CLAUDE.md`.

`HANDOFF.md` template:

```
## Current
Sub-project: core | Plan: docs/superpowers/plans/...core.md
Branch: feat/core-bitboards @ <commit>
Last completed task: 7 (sliding attacks via magics)
Next task: 8 (pin/check masks)
State: green | red (cargo test summary) | blocked (reason)

## Verify before continuing
cargo test core:: && cargo test --release perft

## Notes / decisions made during work
- ...

## Open questions for user
- ...

## Log (newest first)
- 2026-09-26 task 7 done, perft depth 3 matches all 6 positions
```
