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
  tui/              sub-project 3 (see section 6.1 for the full layout)
    terminal.rs     terminal guard, panic hook, signals
    event.rs        AppEvent and the poll loop
    app.rs          App state machine (Menu, Playing, GameOver, dialogs)
    worker.rs       background thread for computer moves
    board.rs        board widget and hit-testing
    panels.rs       status, Jev panel, move list, captured pieces
    input.rs        command box editor and command parsing
    movetext.rs     lenient move parsing
    glyphs.rs       piece glyph sets and palettes
    files.rs        saving FEN and PGN
```

Dependency direction is strictly `tui -> engine -> core`. `core` has no external runtime
dependencies (except `thiserror`), no I/O, and no trait objects.

### Crates

| Purpose | Crate |
| --- | --- |
| TUI | `ratatui` 0.30 (crossterm 0.29 through `ratatui::crossterm`), `signal-hook` |
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
    // api_key, model, max_options, filter_losing, timeout, veto_margin_cp, warnings
    // (no URL: the endpoint is the fixed jev::JEV_ENDPOINT)
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
    pub fn config(&self) -> &EngineConfig;
    pub fn choose_move(&self, game: &Game) -> Option<ComputerMove>;    // None only when game over
}
impl ComputerPlayer<JevClient> {
    pub fn from_config(config: EngineConfig) -> Self;                  // real client when a key is set
}

/// Every legal move with its search score, best first (used by the player and the harness).
pub fn analyse(game: &Game) -> Vec<ScoredMove>;

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
  rule, and for insufficient material. Checkmate and stalemate take precedence over the fifty-move
  rule, as in `Game::outcome`.
- **Budget**: scoring all root moves of Kiwipete takes under 250 ms in release mode.
- **SEE**: swap-off algorithm using `Position::attackers_to` with x-rays revealed by removing
  attackers from the occupancy; king value treated as effectively infinite. A piece absolutely
  pinned to its king takes part only when the target lies on its pin line. The first capturer is
  checked against the pins of the position before it moves; the recaptures against the pins of the
  position after the first capture (so a pin that capture releases or creates is honoured). Pins
  created or released by later captures in the exchange are not modelled.

### 5.4 Annotation and buckets

Per root move, `annotate.rs` produces plain words (joined with "; ") from facts code computes:

- castles kingside / castles queenside
- captures the <piece> on <square> — plus exactly one of "undefended" (no recapture possible),
  "wins material in the exchange" (SEE ≥ 50), "equal trade" (−50 < SEE < 50), "loses material in
  the exchange" (SEE ≤ −50)
- captures en passant
- promotes to a <piece>
- gives check / delivers checkmate
- attacks the <piece> on <square> — the most valuable enemy piece the moved piece newly attacks,
  when it is worth more than the mover or undefended (a king counts as worth more than anything; a
  defender pinned to its king off the line to the attacked piece does not defend it)
- moves the attacked <piece> to safety — the moved piece was losing material by SEE before and is
  not after
- leaves the <piece> on <square> exposed to capture — our most valuable piece that the
  opponent can win by SEE after the move (after a capture the destination square is left out; the
  capture qualifier covers it)
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

- `JevClient` posts with `ureq` 3.x to the fixed endpoint `engine::JEV_ENDPOINT` =
  `https://api.typesafe.ai/v1/systemone` (the TypeSafe quickstart URL), with
  `Authorization: Bearer <key>` and `Content-Type: application/json`. The endpoint is not
  configurable (user decision, 2026-09-27); the offline transport tests use a test-only
  constructor. Request/response types are plain `serde` structs, so they can be tested without
  a network.
- Retries: at most 3 attempts. Retry 429, 5xx (including 529) and transport errors; backoff 250 ms
  then 500 ms, or the `retry-after` value when present, capped at 2 s. No retry on other 4xx.
  Timeout 5 s per attempt.
- Environment variables (read by `from_env`, parsed by `from_vars`); an invalid value falls back to
  the default and adds a line to `config.warnings` for the TUI to show:

| Variable | Default | Meaning |
| --- | --- | --- |
| `JEV_API_KEY` (fallback `TYPESAFE_API_KEY`) | none | Required for Jev play. Absent or empty means local-search fallback. |
| `JEV_MODEL` | `jev-latest` | Model alias or versioned ID. |
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

Revised 2026-09-27 after `core` and `engine` were built: the design below re-validates the original
section against the APIs as merged, current ratatui/crossterm behaviour (verified by compiling and
running scratch programs under a pty), and the engine caveats in `docs/handoff/HANDOFF.md`. The user's
terminals are GPU/Linux terminals (Ghostty, Kitty, WezTerm, Alacritty).

### 6.1 Structure and dependencies

```
src/main.rs            launches the TUI (the old CLI entry is dropped; the legacy chess::Game stays
                       until the cleanup sub-project)
src/tui/
  mod.rs               run(): terminal setup/teardown, main loop
  terminal.rs          guard: raw mode, alternate screen, click-and-drag mouse capture, bracketed
                       paste; restored on every exit path; thread-aware panic hook; SIGINT/SIGTERM/SIGHUP
  event.rs             AppEvent { Term(Event), Engine(EngineReply), Tick } and the 50 ms poll loop
  app.rs               App state machine: Menu | Playing | GameOver plus a dialog stack;
                       handle(AppEvent) -> Action
  worker.rs            Arc<ComputerPlayer<JevClient>>, named "engine" thread, catch_unwind,
                       request/reply with generation and position hash
  board.rs             board widget; square_rect / square_at hit-testing (flip-aware)
  panels.rs            status, Jev panel, move list, captured pieces
  input.rs             hand-rolled single-line command editor and command parser
  movetext.rs          lenient move parsing (SAN, UCI, loose SAN with ambiguity detection)
  glyphs.rs            Solid / Outline / Ascii glyph sets and palettes (truecolor with 256-colour fallback)
  files.rs             saving FEN/PGN: `~` expansion, extension, overwrite confirmation, temp file + rename
```

- Dependencies: `ratatui = "0.30"` and `signal-hook`; crossterm is imported only through
  `ratatui::crossterm` (no direct crossterm dependency, so the versions cannot diverge). Dev:
  `insta`. No text-input or temp-file crates (`tui-textarea` 0.7 pulls ratatui 0.29 and does not
  compile against 0.30).
- `Cargo.toml` adds `[profile.dev.package.chess] opt-level = 3`, so `cargo run` searches Kiwipete in
  about 0.10 s instead of about 1.34 s (release: about 0.06 s).
- `tui` uses only `chess::core` and `chess::engine`, never the legacy crate-root `chess::Game`.
- The TUI stays in the `chess` crate; splitting it into a separate workspace member is left to the
  cleanup sub-project.

### 6.2 Screens

- `Menu`: Human vs Human; Human vs Jev (White, Black or random); Jev vs Jev (watch mode: step
  delay adjustable with `+`/`-`, default 1 s; `space` pauses); Load FEN; Quit. The menu shows the Jev
  status ("Jev ready (jev-latest)" or "No JEV_API_KEY — local search") and any
  `EngineConfig.warnings`. Without a key the computer is called "Local search" wherever the UI names
  it (menu entries, mode label, turn and thinking lines, its panel's title, PGN names), so the screen
  never says "Jev" while Jev is not playing.
- `Playing`: the main screen (layout below). The Status panel's top border names the mode, trying
  shorter forms until one fits beside the title: `You (White) vs Local search`, then
  `You (W) vs Local`, then `W You · B Local` (`Local search vs Local search`, then `Local vs Local`;
  with a key the same with `Jev`, and `Jev vs Jev` always fits). The turn and thinking lines drop the
  player's name rather than wrap.
- `GameOver` overlay: result and reason; New game, Save PGN, Menu.
- Dialogs: FEN input, save path, overwrite confirmation, promotion picker (Q R B N, mouse or keys),
  resign confirmation, help. Every dialog renders `Clear` before drawing.
- Below 60×20 the screen shows only "Terminal too small (need 60×20)".

Playing layout:

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

### 6.3 Board rendering

- Squares are 3×1 cells by default, 5×2 when the terminal has room, 7×3 on large terminals, with rank
  and file labels.
- Palette: mid-tone squares (`Rgb(0xB5,0x88,0x63)` / `Rgb(0x7A,0x56,0x34)` when `COLORTERM` is
  `truecolor`/`24bit`, else `Indexed(137)` / `Indexed(94)`); pieces pure white/black
  (`Rgb(255,255,255)`/`Rgb(0,0,0)` or `Indexed(231)`/`Indexed(16)`), never ANSI white/black.
- Highlights: last move (tint), selected piece, legal targets (dot), keyboard cursor (outline), king in
  check (red).
- Glyph sets: Solid (default: ♚♛♜♝♞♟ for both sides, colour carries the side; the pawn is written as
  `U+265F U+FE0E`; U+FE0F is never emitted), Outline, Ascii (`Piece::to_fen_char`). `g` cycles them;
  `--glyphs solid|outline|ascii` and `RCHESS_GLYPHS` set the start; `NO_COLOR` switches to Outline.
  A test asserts every glyph and filler is one cell wide.
- `f` flips; a game as Black against Jev starts flipped.

### 6.4 Input

- Focus stack: dialog > command box > board; each event goes to the topmost focus only. Key events
  count only when `kind == Press`.
- Mouse: click a piece then a target, or drag and drop; the wheel scrolls the move list; menus and
  dialogs are clickable. Hit-testing uses the rectangles saved during the last draw.
- Keyboard: arrows move a cursor; Enter selects/moves; Esc cancels. Hotkeys (board focus only):
  `u` undo, `f` flip, `n` new, `g` glyphs, `?` help, `q` or Ctrl+C quit (asks for confirmation while a
  game is in progress); Ctrl+S saves PGN from the board or the command box.
- Alt chords: Esc typed quickly before a key arrives as Alt+key. Outside text fields (board, menu,
  yes/no dialogs, game-over overlay) it is handled as Esc followed by the key; while typing (the
  command box, the FEN and save-path dialogs) Alt chords are ignored, so readline habits (Alt+B,
  Alt+F, ...) neither change the text nor reach the board.
- Command box (`/` or `:`): single line with ←/→/Home/End/Backspace/Delete; bracketed paste with
  control characters stripped; a newline in pasted text submits. Commands: `:undo`, `:flip`, `:new`,
  `:fen <FEN>`, `:savefen <path>`, `:savepgn <path>`, `:resign`, `:glyphs`, `:help`, `:quit`. No move
  or command starts with a space, so space in an empty command box does what it does on the board:
  it retries a failed engine, and in Jev vs Jev pauses or resumes; after any text it is a space.
- Move text: strict `parse_san`, then UCI (case-insensitive), then loose SAN (case-insensitive; `x`,
  `+`, `#`, `=` optional). A move is played only when exactly one legal move matches; otherwise the
  status panel shows "ambiguous: Bc3, bxc3" or "not a legal move: <text>" and the text stays in the
  box for editing. Echoed input (a move, an unknown command or argument, a pasted FEN) is cut to its
  first 24 characters plus "…", so a pasted FEN shows only its start. FEN errors show only core's
  reason (`invalid FEN: <reason>`), never the FEN text core appends to it.
- Undo against Jev returns to the human's previous turn (two plies, or one if Jev's reply has not
  arrived). Undo, new game and menu stay available while Jev is thinking; the late reply is discarded.
- Saving: `~` expanded, `.pgn`/`.fen` appended when missing, an existing file triggers an overwrite
  prompt, written through a temp file and rename. PGN gets the Seven Tag Roster with a real Date and
  White/Black set to "You", "Jev" or "Local search", and move text wrapped at 80 columns.

### 6.5 Concurrency

- The main loop polls terminal events with a 50 ms timeout, drains engine replies, and redraws on
  state change or tick. The app sees only `AppEvent`s.
- One `Arc<ComputerPlayer<JevClient>>` is built from `EngineConfig::from_env()`. Each computer turn
  spawns a thread named "engine" with `(generation, position hash, game.clone())` and runs
  `choose_move` inside `catch_unwind`. After a panic the same thread runs the local search and replies
  with its best move, noted "engine error — local search"; the UI never calls `analyse` (a search can
  take seconds and must neither block nor crash the UI). Only if the local search panics too is the
  reply a failure: the UI stops asking and says so until space (on the board or in an empty command
  box) retries.
- At most two requests are out at once (`MAX_IN_FLIGHT = 2`: the live one plus one discarded one still
  running). Threads cannot be cancelled, so undo, new game or menu while the computer thinks leave the
  old request running; without the cap a held key would start a burst of paid Jev calls (or CPU-bound
  searches). A request over the cap waits for an old one to answer, and the status panel says so.
- A reply is applied only when its generation and position hash both match the current game (a legal
  but stale reply must not be played). `None` means the game is over.
- While the engine thinks, the status panel shows a spinner with elapsed seconds; the Jev panel shows
  the last `ComputerMove`'s source, top 3 with probabilities, confidence, model, latency and note.
- Jev vs Jev requests the next move after the step delay unless paused; the game ending stops it.

### 6.6 Terminal safety

- Setup: `ratatui::try_init`, then a click-and-drag-only mouse capture (`?1000h ?1002h ?1006h`; no
  `?1003h` motion reporting) and bracketed paste.
- A guard always disables mouse capture and bracketed paste, calls `ratatui::try_restore` and shows
  the cursor, ignoring every error: on normal exit, on error return, and from the panic hook. Nothing on
  the restore path may print: on a hung-up tty `eprintln!` panics, and `ratatui::restore`, ratatui's
  panic hook and `Terminal`'s `Drop` all print their errors, which turns a closed terminal into an
  abort. So the `Terminal` is never dropped.
- The panic hook replaces ratatui's and is thread-aware: on "main" it restores, then calls the hook
  that was in place before `ratatui::try_init`; a panic on any other thread leaves the terminal alone
  (the engine thread's panic is caught by `catch_unwind`).
- `signal-hook` flags for SIGINT, SIGTERM and SIGHUP are checked every tick, so the app exits through
  the guard, then ends by that signal. If the UI does not react within 1 s (crossterm keeps polling a
  hung-up tty), the signal thread restores and ends the process itself. When the UI loop instead ends
  with an I/O error, it waits up to 100 ms (`SIGNAL_GRACE`) after the restore for a quit signal: a tty
  that hangs up fails the next write at about the moment its SIGHUP arrives, and the process should
  end by that signal, not by the error.
- Logging is never configured to TRACE for `ureq` (it would print the API key).

### 6.7 Verification

- Headless, offline tests only: engines use a mock `MoveChooser` or `ComputerPlayer::new(None, ..)`,
  never `from_env`.
- App logic: scripted `AppEvent` sequences for menu flows, click and drag moves, the promotion
  picker, undo while thinking, stale-reply discard, commands, dialogs and the glyph toggle.
- Pure functions: `square_rect`/`square_at` round trips (flipped too), lenient move parsing including
  ambiguity, PGN wrapping and headers, path handling, glyph widths.
- Render snapshots with `insta` and `TestBackend` at 80×24, 120×40 and a too-small size; colours
  checked with buffer debug snapshots; latency, tokens, model and confidence blanked.
- `main.rs` does only terminal setup. The last plan step is a manual smoke test in a real terminal by
  the user (the agent cannot drive a TUI).

## 7. Delivery Plan

Sub-projects run sequentially; each is merge-ready before the next starts.

1. `core` — done when the perft suite passes and the benchmark target is met.
2. `engine` — done when the offline tests pass, the ignored live test passes once, and the
   evaluation harness has been run once with its summary recorded in `HANDOFF.md`.
3. `tui` — done when all three game modes are playable end to end, the headless tests and snapshots
   pass, and the user has smoke-tested `cargo run` in a real terminal.
4. `tui-polish` — full-screen layout, piece images and Jev debug mode (section 9); done per 9.6.
5. Cleanup — delete `src/pieces/`, `src/board.rs`, `src/ai.rs` and the old `Game`; remove
   `drawille`, `mockall` and the `[env]` key; update `CLAUDE.md`; extend CI with
   `cargo fmt --check` and `cargo clippy -- -D warnings`.

New code lives in new modules beside the old code until step 5, so `main` never breaks.

Git: one branch per sub-project (`feat/core-bitboards`, `feat/jev-engine`, `feat/tui`,
`feat/tui-polish`, `chore/cleanup`), one commit per plan task. The user merges.

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

## 9. Sub-project 4 — `tui-polish` (full-screen board, piece images, Jev debug mode)

Added 2026-09-27 after the user's manual smoke test of the merged TUI: the TUI does not fill the
terminal, the board is small and the piece glyphs are hard to identify, and there is no way to see
what is sent to Jev. Sections 6.x still hold except where this section changes them. Branch
`feat/tui-polish`.

### 9.1 Scope and dependencies

- `src/tui/` changes throughout (layout, board, glyphs, app, panels, a new `pieces.rs`, a new
  `debug.rs`); `src/engine/` gains exchange recording (9.5). `src/core/` and the legacy modules are
  not touched.
- New dependencies: `ratatui-image = { version = "11.1", default-features = false, features =
  ["crossterm"] }` (MIT; built on ratatui `^0.30.1`; the default `chafa-dyn` feature would need
  libchafa, so default features stay off) and `image = { version = "0.25", default-features = false,
  features = ["png"] }` for decoding and compositing, and `rustix = { version = "1", features =
  ["event"] }` (already in the tree through crossterm) for `poll` on stdin during the graphics query
  (9.3). Nothing else.
- Piece art: the Cburnett set (Colin M.L. Burnett, Wikimedia Commons `Chess_{k,q,r,b,n,p}{l,d}t45.svg`),
  used under its BSD licence (it is also offered under GPL and GFDL). The licence is confirmed from
  the Commons file pages when the files are fetched; if BSD is not offered, stop and ask the user.
  The 12 SVGs are rasterized once to 256×256 RGBA PNGs with `rsvg-convert` and committed under
  `assets/pieces/` with `LICENSE` (BSD text and attribution) and a `README.md` recording the source
  URLs and the conversion command. The PNGs are embedded with `include_bytes!`, so the binary needs
  no files at runtime.

### 9.2 Layout fills the terminal

- The Playing screen uses every cell. The left column holds the board panel and the command box; the
  right column (status, Jev, moves, captured) takes all remaining width. `SIDE_MAX_WIDTH` and the
  centring of the whole layout are removed; `SIDE_MIN_WIDTH` (30) stays.
- Square size is no longer one of three presets. For a square height `square_h` (≥ 1 row) the
  width is `square_w = max(3, round(square_h × cell_h / cell_w))`, bumped up to the next odd number
  so a text glyph sits in the middle column; squares then look square in pixels. `cell_w × cell_h` is
  the font size in pixels reported by the terminal (9.3), or 10×20 when unknown. The board uses the
  largest `square_h` for which 8 squares plus labels and borders fit the terminal height and the
  width left after `SIDE_MIN_WIDTH`. Rank and file labels stay one cell. If the board is limited by width, it is centred vertically inside its panel,
  and the panel still fills the column.
- The right column's panels stretch: Status and Captured keep their current heights; Jev and Moves
  share the remaining rows (Moves at least `MOVES_MIN_ROWS`, Jev grows first up to its content).
- Menu, dialogs, help and the game-over overlay stay centred boxes drawn over a full-screen
  background. The 60×20 minimum and the too-small notice are unchanged.
- Hit-testing keeps using the rectangles saved during the last draw, so mouse input follows the new
  sizes without other changes.

### 9.3 Piece images

- Styles: `GlyphSet` gains `Image` as the first entry: `g` cycles Image → Solid → Outline → Ascii →
  Image; `--glyphs image|solid|outline|ascii` and `RCHESS_GLYPHS` accept `image`. `NO_COLOR` still
  starts in Outline (the colour-independent marks of 6.3 apply to text styles only).
- Graphics detection: at start-up, after raw mode and the alternate screen are entered and before
  mouse capture, bracketed paste and the event loop start, the TUI writes the capability query from
  `ratatui_image::picker::cap_parser::Parser::query` and reads the answers itself on the UI thread
  with `poll` and a 1 s deadline, feeding `Parser::push` until the status report arrives.
  (`Picker::from_query_stdio` is not used: when a terminal never answers, its reader thread stays
  blocked on stdin after the timeout and swallows keystrokes.) The result picks Kitty (Ghostty,
  Kitty), iTerm2 (WezTerm, iTerm2, from the environment as ratatui-image does), Sixel or
  Halfblocks, and the font size from the cell-size answer, else from
  `crossterm::terminal::window_size()` pixels, else 10×20; the `Picker` is then built for that
  protocol and font size. The query
  is skipped (text styles only, `Image` removed from the `g` cycle) when `--glyphs`/`RCHESS_GLYPHS`
  names a text style, when `NO_COLOR` is set, or when `RCHESS_IMAGES=off`. A query error or timeout
  never stops the program: it falls back to Halfblocks and adds a menu warning. A reported or
  measured font size outside 1..=256 pixels per cell is ignored (the next source is used), and no
  picture is built larger than 4096 pixels per side (the square shows the Solid glyph instead).
  When the query timed out, a kitty answer that arrives later (a slow SSH link) would reach
  crossterm as key presses (`Alt+_`, `G`, `i`, `=`, ...); for 10 s after such a query one run of key
  presses shaped like that answer is dropped. The other answers never become key presses.
- Font changes: after a Resize, when the protocol is Sixel or iTerm2 (their pictures are encoded at a
  pixel size; Kitty placeholders and half-blocks scale with the cells), the font size is measured
  again the same way (cell-size query and status request, `poll`, 1 s deadline, else window pixels ÷
  cells, else unchanged), once per batch of resize events; a changed size rebuilds the picker and
  clears the picture cache. The font is never guessed from pixel sizes and padding alone.
- Default style: `Image` when the picker found Kitty, iTerm2 or Sixel; otherwise Solid, with `Image`
  still in the cycle (drawn with half-blocks).
- Drawing: in `Image` style each occupied square at least 5×2 cells (Kitty, iTerm2, Sixel) or 11×5
  cells (half-blocks, which are unrecognisable smaller) gets an image; smaller squares fall back to
  the Solid glyph for that frame. A picture is never drawn under a dialog or the game-over box. The image area is the square minus its leftmost and
  rightmost columns, which stay text cells for the keyboard cursor's `[ ]` and the colour-independent
  side marks. The piece PNG is scaled to fit the image area's pixel size (aspect kept), centred, and
  composited (alpha over) onto a solid RGB rectangle of exactly that pixel size in the square's current
  background colour: light or dark, or the highlight colour for selection, last move, capture target
  or check.
  Terminal transparency handling is never relied on. The composite is scaled with a filter that
  keeps edges clean (`image::imageops::FilterType::Lanczos3`) and cached in a `pieces::ImageCache`
  keyed by (piece, background RGB, pixel width, pixel height); a size change clears the cache. The
  cache holds the ratatui-image protocol object built for that composite, so a piece that moves to a
  square of the same colour reuses it. Legal-target dots on empty squares, the keyboard cursor and
  the labels are text as in 6.3; nothing is drawn on top of an image. When the session drew Kitty
  pictures, every restore path writes Kitty's delete-all command (`ESC _ G a=d,d=A ESC \`, tmux-wrapped
  when needed) before leaving the alternate screen, so the pictures do not stay in the terminal's
  image memory.
- Indexed palettes (no truecolor) composite onto the RGB value of the indexed colour's xterm
  default. Colours used for compositing come from the same `Palette` as the text rendering.
- Tests: snapshots keep using text styles; unit tests cover square-size maths, the composite (pixel
  checks on a known background), the cache key and eviction, style cycling with and without a
  graphics protocol, and a Halfblocks render of one piece into a `TestBackend` (deterministic). The
  pty smoke test runs with the query skipped, plus one scenario where the query times out (the pty
  does not answer) to prove start-up continues, no query bytes are echoed, and the terminal is
  restored.

### 9.4 Debug mode (TUI)

- On with `--debug` or `RCHESS_DEBUG` set to anything other than empty or `0`; off by default. The
  Status panel's top border shows `DEBUG` beside the mode title while it is on; when both do not fit,
  the mode title keeps the border and `DEBUG` starts the first status line. `--help` lists the flag and the variables
  `RCHESS_DEBUG`, `RCHESS_DEBUG_LOG` and `RCHESS_IMAGES`.
- Exchange view: `d` (board focus) opens a full-screen "Jev exchange" screen. Header: `exchange N of
  M · move <fullmove> · <SAN played> · <source> · <status> · <attempts> attempt(s) · <latency> ms`,
  plus `stale — not played` for a reply that was discarded. Body: `REQUEST` (method, URL, headers
  with `Authorization: Bearer <redacted>`, then the pretty-printed JSON body), then one `RESPONSE`
  block per attempt (status or error, elapsed ms, and the body pretty-printed when it parses as JSON,
  otherwise the raw text with control characters escaped). Keys: ↑/↓, PgUp/PgDn, Home/End scroll;
  ←/→ older/newer exchange; Esc closes. The view is a screen above Playing: engine replies are still
  applied underneath and the view's exchange list grows while it is open. With no exchanges yet it
  says "no Jev requests yet". With debug off, `d` shows "debug mode is off (start with --debug)".
- History: the last 50 exchanges in memory, newest last, including stale replies (marked).
- Log file: one JSON object per line, appended to `RCHESS_DEBUG_LOG` when set (a leading `~` is
  expanded as for save paths), else `$XDG_STATE_HOME/rchess/jev-debug.jsonl`, else
  `~/.local/state/rchess/jev-debug.jsonl` (macOS too). Missing directories are created with mode
  0700 (existing ones are left alone) and the file is opened for append with mode 0600; an existing
  log with a looser mode is set to 0600. Fields:
  `time` (UTC, RFC 3339), `ply` (game ply when requested), `played` (SAN), `source` (the
  `MoveSource` label), `stale` (bool), `request` (method, URL, redacted headers, JSON body),
  `attempts` (each: `status` or null, `elapsed_ms`, `response` as JSON when it parses else a string,
  `error` or null). The UI thread sends each record over a channel to a dedicated thread named
  `debug-log`, which writes and flushes; the UI never waits on disk. The first write error shows one
  status warning ("debug log disabled: <reason>", kept until a game screen can show it) and stops
  logging for the session; the screen view keeps working. The key is also redacted when a server
  echoes it JSON-escaped (for example `\/`), both in the raw text and after decoding.

### 9.5 Engine changes for debug mode

- New public types in `engine::jev`: `JevExchange { method: String, url: String, headers:
  Vec<(String, String)>, body: serde_json::Value, attempts: Vec<JevAttempt> }` and `JevAttempt {
  status: Option<u16>, response: Option<String>, error: Option<String>, elapsed: Duration }`. The
  `Authorization` header value is always the literal `Bearer <redacted>`.
- `MoveChooser` gains `fn choose_traced(&self, request: &ChoiceRequest, trace: &mut
  Option<JevExchange>) -> Result<ChoiceAnswer, JevError>` with a default body that calls `choose`
  and leaves `trace` as `None`, so existing fakes compile unchanged. `JevClient` overrides it: it
  fills the exchange and records every attempt, including retried ones, with the response body read
  under the existing 1 MiB cap.
- `EngineConfig` gains `pub trace: bool` (default false; not read from the environment by the
  engine — the TUI sets it from `--debug`/`RCHESS_DEBUG`). `ComputerMove` gains `pub exchange:
  Option<Box<JevExchange>>`, filled only when `trace` is true and Jev was asked (Jev, Vetoed, and
  Fallback after a Jev error); never for OnlyMove, MateInOne or the no-key fallback. With `trace`
  false the request path is unchanged.
- Tests: the existing offline transport tests (local TCP server) gain cases that assert the exchange
  records method, URL, redacted headers, body, statuses and bodies across a retry, and that a
  sentinel API key appears nowhere in the exchange, its `Debug` output, the rendered exchange view or
  the log line. Player tests with a fake chooser cover when `exchange` is filled.

### 9.6 Done criteria

- Offline tests, snapshots (updated for the new layout; each change reviewed by hand) and the pty
  smoke test pass; `cargo fmt --check` clean; clippy clean for `src/tui`, `src/engine` and
  `src/main.rs`.
- The user confirms in a real terminal (Ghostty and one other) that the TUI fills the terminal, the
  board scales with it, pieces are images and easy to identify, `g` cycles styles, and `--debug`
  plus `d` shows the Jev request and response and writes the log file.
