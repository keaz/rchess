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
  engine/           sub-project 2 (see section 5.2 for the full layout)
    eval.rs         material + piece-square evaluation, game phase
    search.rs       negamax alpha-beta (depth 3) + quiescence, mate and draw scores
    see.rs          static exchange evaluation
    annotate.rs     converts per-move facts into plain-language effects and a bucket
    describe.rs     position-to-words state for Jev
    jev.rs          Jev request/response types, HTTP client, retry policy
    config.rs       EngineConfig from environment variables
    player.rs       ComputerPlayer: forced moves, shortlist, ask Jev, veto, fallback
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
| HTTP (sync, used on a worker thread) | `ureq` 3.x with the `json` feature |
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

Revised 2026-09-26 after `core` was built: the design below re-validates the original section
against the `core` API as merged and the current TypeSafe docs, and adds an evaluation harness.

### 5.1 Jev facts relied on

- Endpoint `POST https://api.typesafe.ai/v1/systemone`, header `Authorization: Bearer <key>`,
  model alias `jev-latest` (currently `jev-1.13.0`; the response's `model` field names the version
  that answered).
- `choice` question: up to 255 options; `instructions` and each criterion may be a string or a JSON
  object. The answer carries `choice`, `probabilities` per option, `confidence`, and `usage`
  (input tokens).
- Limits: 64k tokens per request (32k for `state` plus the longest question); rate limits are
  dynamic (about 1,200 requests per minute today).
- Errors: 401 (bad key), 422 (validation), 429 (rate limit), 529 (overloaded). Retry 429 and 529
  with exponential backoff, honouring `retry-after` when present.
- Documented weaknesses (jev-1.13): arithmetic, counting, multi-hop reasoning (indirection), large
  state full of irrelevant detail. Therefore code owns legality, tactics and numbers; Jev gets
  plain-language facts written from our side's point of view and makes the judgment.

### 5.2 Module layout and public API

```
src/engine/
  mod.rs        re-exports
  eval.rs       static evaluation: material + piece-square tables (centipawns), game phase
  search.rs     negamax alpha-beta, depth 3 + quiescence; mate scores; draws via game history
  see.rs        static exchange evaluation for captures and hanging pieces
  annotate.rs   per-move facts to plain words + bucket
  describe.rs   position-to-words state for Jev (pieces, material, phase, threats, recent moves)
  jev.rs        serde request/response types, JevClient (ureq 3), retry policy
  config.rs     EngineConfig::from_env / from_vars
  player.rs     ComputerPlayer: forced shortcuts, shortlist, ask Jev, veto, fallback
examples/jev_eval.rs   evaluation harness (section 5.8)
```

```rust
pub struct EngineConfig {
    // api_key, model, base_url, max_options, filter_losing, timeout, veto_margin_cp, warnings
}
impl EngineConfig {
    pub fn from_env() -> EngineConfig;                                  // never fails
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> EngineConfig;
}

pub trait MoveChooser: Send + Sync {
    fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError>;
}
pub struct JevClient { /* private; Debug hides the key */ }            // impl MoveChooser

pub struct ComputerPlayer<C: MoveChooser> { /* private */ }
impl<C: MoveChooser> ComputerPlayer<C> {
    pub fn new(chooser: Option<C>, config: EngineConfig) -> Self;
    pub fn choose_move(&self, game: &Game) -> Option<ComputerMove>;    // None only when game over
}

pub struct ComputerMove {
    pub mv: Move,
    pub san: String,
    pub source: MoveSource,
    pub top: Vec<(String, f32)>,       // up to 3 Jev options with probabilities, best first
    pub confidence: Option<f32>,
    pub model: Option<String>,         // versioned model ID that answered
    pub latency: Duration,             // whole choose_move call
    pub input_tokens: Option<u32>,
    pub note: Option<String>,          // why a fallback or veto happened
}
pub enum MoveSource { Jev, OnlyMove, MateInOne, Vetoed { jev_pick: String }, Fallback }
```

Rules:
- `engine` depends on `core` only; `core` never depends on `engine`.
- The player never returns an error: every failure degrades to `Fallback` with a `note`.
- Generics rather than trait objects, and all public types are `Send`, so the TUI worker thread can
  own a `ComputerPlayer<JevClient>`.
- The API key is read once, never logged, and never printed (`JevClient`'s `Debug` hides it).

### 5.3 Evaluation, search and SEE

- **Eval** (centipawns, from the side to move): material P 100, N 320, B 330, R 500, Q 900 plus
  Tomasz Michniewski's "Simplified Evaluation Function" piece-square tables. The king uses the
  endgame table when both sides lack queens, or when every side that has a queen has no rook and at
  most one minor piece.
- **Search**: every root move is searched with a full window (so each gets an exact score), then two
  more plies (depth 3 in total), then quiescence. Quiescence uses stand-pat plus captures and queen
  promotions in MVV-LVA order; when in check it searches all evasions and never stands pat; it is
  capped at 8 plies. Move ordering: captures by MVV-LVA, then promotions, then quiet moves.
- **Scores**: mate = ±(30000 − ply); stalemate = 0; draw = 0 for a repetition of any position hash in
  the game history since the last irreversible move or on the current search path, for the fifty-move
  rule, and for insufficient material.
- **Budget**: scoring all root moves of Kiwipete takes under 250 ms in release mode.
- **SEE**: swap-off algorithm using `Position::attackers_to` with x-rays revealed by removing
  attackers from the occupancy; king value treated as effectively infinite.

### 5.4 Annotation and buckets

Per root move, `annotate.rs` produces plain words (joined with "; ") from facts code computes:

- castles kingside / castles queenside
- captures the <piece> on <square> — plus exactly one of "undefended", "equal trade", "loses
  material in the exchange" (from SEE)
- captures en passant
- promotes to a <piece>
- gives check / delivers checkmate
- attacks the <piece> on <square> — the most valuable enemy piece the moved piece newly attacks,
  when it is worth more than the mover or undefended
- moves the attacked <piece> to safety — the moved piece was losing material by SEE before and is
  not after
- leaves the <piece> on <square> undefended against capture — our most valuable piece that the
  opponent can win by SEE after the move
- allows checkmate — search shows we are mated

Bucket from `d = best_score − move_score` (mate scores included):

| Bucket | Rule |
| --- | --- |
| `winning` | move score ≥ +300 or a mate for us, and d ≤ 50 |
| `good` | d ≤ 50 |
| `neutral` | 50 < d ≤ 150 |
| `bad` | 150 < d ≤ 300 |
| `losing` | d > 300, or the move allows checkmate |

Numbers never reach Jev; only the words and the bucket do.

### 5.5 Request shape

State is written from our side's point of view:

```json
{
  "model": "jev-latest",
  "state": {
    "side_to_move": "Black",
    "move_number": 12,
    "phase": "middlegame",
    "material": "White is ahead by a pawn",
    "in_check": "no",
    "our_pieces": "King g8, Queen d8, Rooks a8 and f8, Bishop d6, Knight f6, Pawns a7 b7 c7 f7 g7 h7",
    "their_pieces": "King g1, Queen h5, ...",
    "recent_moves": "11. Nxe5 Bd6 12. Qh5",
    "threats_against_us": ["Our knight on f6 is attacked by the queen on h5"]
  },
  "questions": {
    "move": {
      "type": "choice",
      "instructions": {
        "question": "You play `side_to_move`. Which move should we play?",
        "guidance": "Each option says what the move does and the engine's assessment: winning, good, neutral, bad or losing. Prefer winning and good moves. Among similar moves, prefer ones that remove threats against our pieces and keep our king safe."
      },
      "criteria": {
        "Nxe5": { "effect": "captures the knight on e5, undefended", "assessment": "good" },
        "O-O": { "effect": "castles kingside", "assessment": "neutral" }
      }
    }
  }
}
```

Option keys are SAN strings (unique within a position since `from_fen` validates en passant).

State fields, all computed by `describe.rs`:
- `move_number`: the fullmove number.
- `phase`: `endgame` by the eval's endgame rule; otherwise `opening` while the fullmove number is at
  most 10; otherwise `middlegame`.
- `material`: material-only difference `diff` in centipawns; "material is equal" when
  `|diff| < 50`, else "<Colour> is ahead by about N pawns of material" with `N = round(|diff| / 100)`
  (singular "pawn" when N is 1).
- `our_pieces` / `their_pieces`: King, Queen, Rooks, Bishops, Knights, Pawns in that order, each with
  its squares in a1..h8 order ("Rooks a8 and f8", "Pawns a7 b7 c7").
- `recent_moves`: the last six plies in SAN with move numbers; empty string at the start.
- `threats_against_us`: each of our pieces the opponent can win by SEE, most valuable first, at most
  three, phrased "Our <piece> on <square> is attacked by the <piece> on <square>" (naming the least
  valuable attacker).

### 5.6 Player flow

`ComputerPlayer::choose_move(&Game)`:
1. Game over: return `None`.
2. Exactly one legal move: `OnlyMove`, no API call.
3. Search all root moves. If a mate in one exists: `MateInOne`, no API call.
4. No chooser (no key): `Fallback` with note "JEV_API_KEY not set — local search".
5. Annotate. Shortlist: when `filter_losing` is on and a non-losing move exists, drop `losing`
   moves; sort by score, best first; keep the first `max_options` (clamped to 1..=255, which also
   covers positions with more than 255 legal moves). The search best move always survives.
6. Ask Jev. On error: `Fallback` with a note naming the cause (HTTP status, timeout, overloaded,
   invalid response).
7. If the chosen key is not in the shortlist: `Fallback` with a note.
8. Veto: if `best_score − score(pick) > veto_margin_cp` (150), play the highest-probability option
   within the margin (the search best always qualifies) as `Vetoed { jev_pick }`. Otherwise `Jev`.

`top` holds the three highest-probability options; `latency` covers the whole call.

### 5.7 Client, retries and configuration

- `JevClient` posts with `ureq` 3.x. Request/response types are plain `serde` structs, so they can be
  tested without a network.
- Retries: at most 3 attempts. Retry 429, 5xx (including 529) and transport errors; backoff 250 ms
  then 500 ms, or the `retry-after` value when present, capped at 2 s. No retry on other 4xx.
  Timeout 5 s per attempt.
- Environment variables (read by `from_env`, parsed by `from_vars`); an invalid value falls back to
  the default and adds a line to `config.warnings` for the TUI to show:

| Variable | Default | Meaning |
| --- | --- | --- |
| `JEV_API_KEY` (fallback `TYPESAFE_API_KEY`) | none | Required for Jev play. Absent or empty means local-search fallback. |
| `JEV_MODEL` | `jev-latest` | Model alias or versioned ID. |
| `JEV_BASE_URL` | `https://api.typesafe.ai` | Override for testing. |
| `JEV_MAX_OPTIONS` | `40` | Shortlist cap, clamped to 1..=255. |
| `JEV_FILTER_LOSING` | `true` | Drop `losing` moves when alternatives exist (`true/false/1/0/yes/no`). |

### 5.8 Verification and evaluation harness

Offline tests (no network):
- eval colour symmetry; search finds mate in one and mate in two, refuses to hang the queen, scores
  stalemate, repetition and fifty-move draws as 0;
- SEE on known exchanges;
- golden annotations and buckets on fixed FENs; golden state JSON;
- request JSON and response parsing against the documented examples; the retry decision table;
  `Debug` for `JevClient` hides the key;
- config parsing including invalid values and the key fallback;
- player paths with a mock `MoveChooser`: game over, only move, mate in one, no key, shortlist
  filter / cap / order, veto, unknown option, error fallback, `top` ordering.

Plus one `#[ignore]` live test against the real API, and an `#[ignore]` release timing test for the
search budget.

`examples/jev_eval.rs` (needs a key; exits with a message otherwise) runs 20 fixed positions
(openings, tactics, endgames, defence) through the real player and prints, per position, Jev's pick,
the search best, the source, whether a veto happened, latency and tokens; then a summary with
agreement %, veto %, mean latency and token cost. It is the tool for tuning the guidance text.

Cost estimate: about 1–2k input tokens per move at $0.042 per million — negligible.

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
2. `engine` — done when the offline tests pass, the ignored live test passes once, and the
   evaluation harness has been run once with its summary recorded in `HANDOFF.md`.
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
