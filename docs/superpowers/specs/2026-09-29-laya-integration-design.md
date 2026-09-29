# Laya Integration — Design Spec

Date: 2026-09-29. Builds on `2026-09-26-rchess-redesign-design.md` (sections 5 and 6). The
architecture described there does not change: `core` → `engine` → `tui`, one engine thread per
computer turn, `App::handle` returns `Action`s and never spawns threads.

## 1. Goal

Add Laya (`convaiinnovations/laya` on Hugging Face) as a second computer player next to Jev.

- The person can play against Jev or against Laya.
- The computer can play itself in four pairings: Jev vs Jev, Laya vs Laya, Jev (White) vs Laya,
  Laya (White) vs Jev.
- Human vs Human, Human vs Jev and Jev vs Jev keep working as they do today.
- Laya uses the same hybrid flow as Jev (spec 5.6): forced moves and mate in one are played
  directly, otherwise one `choice` question over the shortlist, the blunder veto, and a
  local-search fallback for every failure.

Success: with `laya-serve` running and `LAYA_URL` set, the person can start a game against Laya
and watch Jev play Laya from the menu; with Laya down or unset, every Laya move falls back to the
local search with a clear note and nothing hangs or crashes.

## 2. Laya facts relied on

- Laya is an open-weights (Apache 2.0) "System One" decision model, ModernBERT-large, about
  808 MB. It runs in Python (`pip install "laya[serve]"`).
- There is no hosted API. `laya-serve` serves `POST /v1/systemone` on `0.0.0.0:8000` by default,
  with the same request and response format as TypeSafe Jev, including `choice` questions with
  criteria.
- `laya-serve` needs no authentication unless `LAYA_API_KEY` is set on the server; then it wants
  `Authorization: Bearer <key>`.
- A malformed question gets `422` with details.
- Accuracy drops for `choice` questions with more than about 50 options (192 option tokens in a
  512-token context). The shortlist default of 40 stays below that.
- The base checkpoint is weak zero-shot on typed decisions, so Laya is expected to play weaker
  than Jev. The veto and the local search keep its moves sound.
- Whether `laya-serve` accepts, ignores or rejects the `model` field is not documented. The
  client sends `LAYA_MODEL` (default `laya`); the manual test in section 8 checks it against a
  real server.

## 3. Decisions

- Transport: the person runs `laya-serve` themselves. The app never starts Python or loads the
  model.
- Laya is opt-in: it is enabled only when `LAYA_URL` is set. Unset, the Laya rows play by local
  search and are named `Local search`, as Jev is without a key.
- No start-up probe. Each Laya move tries the server and falls back on failure.
- One client for both providers: `JevClient` sends to the endpoint in its config. There is no
  separate Laya client.
- Menu: one "Computer: Jev / Laya" toggle for the "You vs …" rows, and four watching rows.
- `examples/jev_eval.rs` stays Jev-only.

## 4. Engine layer

### 4.1 `engine::config`

```rust
pub enum Provider { Jev, Laya }   // Copy, Eq, Debug
impl Provider { pub const fn name(self) -> &'static str }   // "Jev", "Laya"
```

`EngineConfig` gains:

- `provider: Provider` (default `Jev`);
- `endpoint: String` (default `JEV_ENDPOINT`).

`EngineConfig::from_vars` keeps reading the Jev variables, unchanged. New
`EngineConfig::laya_from_vars(get)` (and `laya_from_env()`) read:

| Variable | Meaning | Default / invalid |
|---|---|---|
| `LAYA_URL` | Full endpoint URL, e.g. `http://127.0.0.1:8000/v1/systemone` | unset: Laya disabled |
| `LAYA_API_KEY` | Optional bearer key | unset: no `Authorization` header |
| `LAYA_MODEL` | `model` field sent with each request | `laya` |
| `LAYA_MAX_OPTIONS` | Shortlist cap, clamped to 1..=255 | 40, with a warning when invalid |
| `LAYA_FILTER_LOSING` | Same as `JEV_FILTER_LOSING` | `true`, with a warning when invalid |

Values are trimmed; an empty value counts as unset. The shared parsing of the max-options and
filter-losing values moves into one helper, called with the variable prefix.

- `LAYA_URL` must start with `http://` or `https://`. Anything else adds the warning
  `LAYA_URL=<value> is not an http(s) URL; Laya is off` and leaves Laya disabled. The value in the
  warning is cut to 60 characters and control characters are replaced.
- When `LAYA_API_KEY` is set and `LAYA_URL` is `http://` with a host other than `localhost`,
  `127.0.0.1` or `[::1]`, add the warning
  `LAYA_API_KEY is sent unencrypted to <host>; use https`. The request still goes out.
- The Laya timeout per attempt is 10 s (CPU inference is slower than hosted Jev). Jev stays at 5 s.
- The veto margin is the same for both (150 cp).

New `EngineConfig::enabled()`: Jev needs `api_key`; Laya needs a non-empty `endpoint`. Every
check of `api_key.is_some()` that means "the model is available" uses `enabled()` instead.

`Debug` for `EngineConfig` shows `provider` and `endpoint` and still redacts the key.

### 4.2 `engine::jev`

- `JevClient::new(config)` builds a client for `config.endpoint` when `config.enabled()`, else
  `None`. The test-only `with_endpoint` is removed; tests set `endpoint` in the config.
- The API key becomes `Option<String>` inside the client. Without a key the request has no
  `Authorization` header, and the recorded `JevExchange` lists only `Content-Type`.
- With a key: unchanged (`Bearer`, redaction in errors, recorded bodies and headers).
- Request body, retry policy (`MAX_ATTEMPTS` = 3, backoff, `Retry-After`), the 1 MiB body cap and
  error classification are unchanged. `422` is final (no retry), as any other 4xx except 429.
- The type names (`JevClient`, `JevError`, `JevExchange`, `ChoiceRequest`) stay. Their doc
  comments say they serve any System One endpoint (Jev or `laya-serve`).

### 4.3 `engine::player`

- `MoveSource::Jev` becomes `MoveSource::Model`; `Vetoed { jev_pick }` becomes
  `Vetoed { pick }`.
- `impl Display for MoveSource` is replaced by `MoveSource::label(self, provider) -> String`:
  `Jev` / `Laya`, `only move`, `mate in one`, `vetoed (Laya picked Qxd5)`, `local search`.
- Notes name the provider: `Laya unavailable (<error>) — local search`,
  `Laya returned an unknown option (…) — local search`,
  `Laya picked Qh4, which the search rates much worse; played Nf6`.
- `ComputerMove` gains `provider: Provider`, the config's provider, so the TUI knows which model
  a move belongs to.
- `ComputerPlayer::from_config` keeps its signature. It gets a client exactly when
  `config.enabled()`.
- `redacted` redacts the key only when there is one.

## 5. TUI layer

### 5.1 Modes

```rust
pub enum Mode {
    HumanVsHuman,
    HumanVsComputer { human: Side, computer: Provider },
    Watch { white: Provider, black: Provider },
}
```

- `HumanVsJev` becomes `HumanVsComputer`, `JevVsJev` becomes `Watch`.
- `Mode::player(side) -> Option<Provider>`: the provider that plays `side`, `None` for a person.
  `engine_plays(side)` is `player(side).is_some()`.
- Step delays, space pause and `+`/`-` pace (today "Jev vs Jev") apply to every `Watch`.
- Load FEN from the menu still starts Human vs Human; `:fen` in a game keeps the game's mode.

### 5.2 Engines and requests

- `App` holds `jev: Arc<dyn Engine>` and `laya: Arc<dyn Engine>`, and
  `App::engine(provider) -> &Arc<dyn Engine>`. `App::new` takes both.
- `tui::run` builds both from `EngineConfig::from_env()` and `EngineConfig::laya_from_env()`,
  each with `trace` set from debug mode. The `RCHESS_FAULT=engine-panic` injection wraps both.
- `EngineRequest` gains `provider`. `App` sets it from `Mode::player(side_to_move)`. The run loop
  calls `worker::spawn_request(app.engine(request.provider).clone(), …)`.
- Generation and hash checks, `MAX_IN_FLIGHT`, the stale-reply rules and the engine-panic
  fallback do not change. In Jev vs Laya consecutive requests go to different engines; the
  in-flight cap counts both together.
- The `Engine` trait: `uses_jev()` becomes `enabled()`; `status()` gives
  `Jev ready (jev-latest)`, `No JEV_API_KEY — local search`,
  `Laya ready (<LAYA_URL, cut to 40 characters>)` or `No LAYA_URL — local search`;
  new `provider()`.

### 5.3 Names

- `App::player_name(provider)`: `Jev` or `Laya` when that engine is enabled, else
  `Local search` (brief form `Local`).
- Mode labels and their brief forms use the per-side names: `You (White) vs Laya`,
  `You (W) vs Laya`, `W You · B Laya`; `Jev vs Laya`, `Laya vs Jev`, `Laya vs Laya`,
  `Local search vs Laya`, `Local vs Laya`. Labels still never repeat.
- Turn line: `Black to move (Laya)`.
- PGN `White` and `Black` tags: `You` or the player name of that side.

### 5.4 Menu

```
┌ rchess ──────────────────────────────┐
│ New game · Computer: [Jev]  Laya     │
│                                      │
│ > 1. Human vs Human                  │
│   2. Human vs Jev: play White        │
│   3. Human vs Jev: play Black        │
│   4. Human vs Jev: random side       │
│   5. Jev vs Jev (watch)              │
│   6. Laya vs Laya (watch)            │
│   7. Jev (White) vs Laya (watch)     │
│   8. Laya (White) vs Jev (watch)     │
│   9. Load FEN                        │
│      Quit                            │
│ …about, notes, keys…                 │
└──────────────────────────────────────┘
```

- New `App` state `computer: Provider`, default `Jev`, kept for the session only (not saved).
- The toggle sits on the "New game" heading row, so the menu grows by three rows (three more
  items), not four. Tab, `←` and `→` switch it; a mouse click on either name picks it (new
  `Hit::MenuComputer(Provider)`). It changes only rows 2-4.
- Names in rows use `player_name`, so without a key or URL the rows read `Local search`.
- `MenuItem::HumanVsJev(SidePick)` becomes `MenuItem::HumanVsComputer(SidePick)` (the provider
  comes from the toggle), and `MenuItem::JevVsJev` becomes
  `MenuItem::Watch { white: Provider, black: Provider }`.
- Digits `1`-`9` start rows 1-9. Quit has no digit; `q` still quits from the menu.
- Key hint: `arrows/jk choose · Tab computer · Enter/1-9 start · q quit`, shortened on narrow
  menus the way other hints are.
- The notes area shows both engine status lines, then the warnings of both configs and the
  start-up warnings. At the minimum 60 × 20 the menu must still fit: the existing `fit_notes`
  trims notes (with its `+N more` row) before anything else. `MIN_HEIGHT` stays 20; a snapshot at
  60 × 20 proves the fit.

### 5.5 Side panel

- The panel that shows the last computer move is titled with the name of the player that made
  it (`Jev`, `Laya` or `Local search`), from `ComputerMove::provider`. Before any computer move it
  shows the status of each engine in this game (one line per distinct provider) and
  `no move yet`.
- The source text comes from `MoveSource::label(provider)`; a veto with a note still shows just
  `vetoed`.

### 5.6 Debug mode

- `debug::Exchange` gains `engine: String` (`"Jev"` / `"Laya"`), filled from
  `ComputerMove::provider`. The JSON-lines record gets an `engine` field; the other fields are
  unchanged.
- The exchange view title reads ` Jev exchange ` or ` Laya exchange ` for the exchange shown.
- `d` works in every mode, as today.

## 6. Errors

- Laya disabled (no `LAYA_URL`): every Laya move is the search best with source `local search`,
  no note (as Jev without a key).
- Laya down (connection refused): three attempts with backoff (about 0.75 s in total), then
  a local-search move with note `Laya unavailable (…) — local search`. The next move tries again.
- `422` or another final error: one attempt, local-search move, the note carries the redacted
  error excerpt.
- Slow server: at most 3 × 10 s plus backoff ≈ 31 s per move. A request cannot be cancelled;
  undo, new game and menu drop its reply by generation, as today.
- Nothing prints to the terminal. The key never appears in notes, the screen, the exchange view
  or the debug log.

## 7. Testing

All offline, run with `JEV_API_KEY`, `TYPESAFE_API_KEY`, `LAYA_URL` and `LAYA_API_KEY` unset.
Functions that read the environment keep taking a `get` closure.

- `engine::config`: `laya_from_vars` with nothing set, a valid URL, a bad scheme, an empty value,
  out-of-range and non-numeric max options, bad filter value, the clear-text key warning (and its
  absence for `localhost`, `127.0.0.1`, `[::1]` and `https`); `enabled()` for both providers;
  `Debug` redacts the Laya key.
- `engine::jev` (scripted server on 127.0.0.1): a Laya config with no key sends no
  `Authorization` header and records none; with a key it sends `Bearer` and redacts it; retries on
  503 and connection refused; `422` is final.
- `engine::player`: notes and `MoveSource::label` name the provider; `ComputerMove::provider` is
  set; a keyless Laya chooser's errors pass through `redacted` unchanged.
- `tui::app`: toggle by Tab, arrows and click; digits `1`-`9`; each `Watch` pairing asks the
  right engine per side (a `FakeEngine` per provider); per-side names in labels, brief labels,
  turn line and PGN tags; disabled engines named `Local search`; `MAX_IN_FLIGHT` across two
  engines.
- `tui::panels`: snapshots of the menu (Jev selected, Laya selected, 60 × 20), a Jev vs Laya game
  after a Laya move, and the Laya exchange view. Each `.snap.new` is reviewed by hand as
  `CLAUDE.md` says.
- `tui::debug`: the `engine` field in the log record and the exchange view title.
- The existing sentinel-key test covers `LAYA_API_KEY` too.
- `tests/pty_smoke.py`: its menu digits still match (`1` Human vs Human, `3` play Black, `5` Jev
  vs Jev). New scenario: `LAYA_URL=http://127.0.0.1:<closed port>/v1/systemone`, start `7`
  (Jev vs Laya, both offline), check that moves keep coming with the Laya note and that quitting
  restores the terminal. The script passes `LAYA_URL` only to that scenario and strips
  `LAYA_API_KEY` like the Jev keys.
- One `#[ignore]` live test: a real `laya-serve` at `LAYA_URL` answers a `choice` request.

## 8. Manual test

With `pip install "laya[serve]"` and `laya-serve` running:

- `LAYA_URL=http://127.0.0.1:8000/v1/systemone cargo run -- --debug`;
- the menu shows `Laya ready (…)`; Tab switches the computer to Laya;
- a game against Laya shows Laya's picks in the side panel; `d` shows the request with no
  `Authorization` header and Laya's answer; this also confirms `laya-serve` accepts the `model`
  field (if it rejects it with `422`, drop the field for Laya and record that in the spec);
- Jev (White) vs Laya with `JEV_API_KEY` set: both panels' names and the title are right, and the
  debug log's records carry `"engine": "Jev"` and `"engine": "Laya"`;
- stop `laya-serve` mid-game: Laya's moves fall back with a note and the game continues.

## 9. Documentation

- `CLAUDE.md`: the Laya variables, `Provider`, the test command with the Laya variables unset,
  the per-provider engines in the TUI description.
- `README.md`: running `laya-serve`, `LAYA_URL`, the new menu rows and the Jev vs Laya pairing.
- `docs/handoff/HANDOFF.md`: the new sub-project, per spec section 8 of the redesign spec.
- The redesign spec is not edited; this spec supersedes its "Jev only" statements.

## 10. Changes during planning and implementation

- `laya-serve` may omit `model` and `usage`: `ChoiceAnswer.model` and `ChoiceAnswer.input_tokens`
  are optional, and an answer without them is used.
- A disabled engine keeps the existing note on its moves: `JEV_API_KEY not set — local search` /
  `LAYA_URL not set — local search` (section 6 said no note).
- Menu at the minimum size: the blank row under the heading is gone, and the line describing the
  highlighted row is left out when the menu does not fit, so both engine statuses keep their rows.
  The key hint has a brief form. `MIN_HEIGHT` stays 20.
- The menu toggle names the models (`Jev`, `Laya`) even when an engine is off, so the two choices
  stay distinguishable.
- `--help` lists the Laya variables.
