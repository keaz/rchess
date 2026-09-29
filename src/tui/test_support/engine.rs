//! Test scaffolding that needs the engine worker and the app's events: a scripted fake
//! engine that never touches the network, and builders for the [`AppEvent`]s the run loop
//! hands to the app.

use std::collections::VecDeque;
use std::panic;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};

use serde_json::json;

use super::{char_events, chord_event, key_event, mouse_event, paste_event};
use crate::core::{Game, Move, Position as ChessPosition};
use crate::engine::{ComputerMove, JEV_ENDPOINT, JevAttempt, JevExchange, MoveSource, Provider};
use crate::tui::event::AppEvent;
use crate::tui::worker::{Engine, LAYA_LOCAL_SEARCH_STATUS, LOCAL_SEARCH_STATUS};

/// How long a test waits for a worker thread's reply: generous, so a broken worker fails
/// the test instead of hanging it.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// [`Engine::status`] of [`FakeEngine::jev`].
pub(crate) const JEV_STATUS: &str = "Jev ready (jev-test)";

// ----- app events -----

/// A key press without modifiers.
pub(crate) fn key(code: KeyCode) -> AppEvent {
    AppEvent::Term(key_event(code))
}

/// A key press with `modifiers`, such as Ctrl+S or Alt+F.
pub(crate) fn chord(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
    AppEvent::Term(chord_event(code, modifiers))
}

/// One key press per character of `text`.
pub(crate) fn chars(text: &str) -> Vec<AppEvent> {
    char_events(text).into_iter().map(AppEvent::Term).collect()
}

/// A mouse event at cell (`column`, `row`).
pub(crate) fn mouse(kind: MouseEventKind, column: u16, row: u16) -> AppEvent {
    AppEvent::Term(mouse_event(kind, column, row))
}

/// A bracketed paste of `text`.
pub(crate) fn paste(text: &str) -> AppEvent {
    AppEvent::Term(paste_event(text))
}

// ----- engines -----

/// `uci` played in `pos` as the fake Jev reports it. Everything but the move is fixed (Jev
/// as the source, runners-up e5 and d5, confidence, model, 1234 ms latency), so screens that
/// show it are deterministic.
pub(crate) fn jev_move(pos: &ChessPosition, uci: &str) -> ComputerMove {
    let mv = pos.parse_uci(uci).expect("scripted move is legal");
    let san = pos.to_san(mv);
    ComputerMove {
        mv,
        san: san.clone(),
        source: MoveSource::Model,
        provider: Provider::Jev,
        top: vec![
            (san, 0.62),
            ("e5".to_string(), 0.21),
            ("d5".to_string(), 0.09),
        ],
        confidence: Some(0.81),
        model: Some("jev-test".to_string()),
        latency: Duration::from_millis(1234),
        input_tokens: Some(512),
        note: None,
        exchange: None,
    }
}

/// The note a player without a Jev key puts on its moves.
pub(crate) const LOCAL_NOTE: &str = "JEV_API_KEY not set — local search";
/// [`Engine::status`] of [`FakeEngine::laya`].
pub(crate) const LAYA_STATUS: &str = "Laya ready (http://127.0.0.1:8000/v1/systemone)";
/// The note a Laya player without `LAYA_URL` puts on its moves.
pub(crate) const LAYA_LOCAL_NOTE: &str = "LAYA_URL not set — local search";

/// `uci` played in `pos` as the fake `provider` model reports it (see [`jev_move`]).
pub(crate) fn model_move(pos: &ChessPosition, uci: &str, provider: Provider) -> ComputerMove {
    let model = match provider {
        Provider::Jev => "jev-test",
        Provider::Laya => "laya-test",
    };
    ComputerMove {
        provider,
        model: Some(model.to_string()),
        ..jev_move(pos, uci)
    }
}

/// `uci` played in `pos` as `provider`'s player without its setting (`JEV_API_KEY`,
/// `LAYA_URL`) reports it: the local search's move, with none of the model's details
/// (runners-up, confidence, model, tokens) and 1234 ms latency, so screens that show it
/// are deterministic.
pub(crate) fn local_move_for(pos: &ChessPosition, uci: &str, provider: Provider) -> ComputerMove {
    let note = match provider {
        Provider::Jev => LOCAL_NOTE,
        Provider::Laya => LAYA_LOCAL_NOTE,
    };
    let mv = pos.parse_uci(uci).expect("scripted move is legal");
    ComputerMove {
        mv,
        san: pos.to_san(mv),
        source: MoveSource::Fallback,
        provider,
        top: Vec::new(),
        confidence: None,
        model: None,
        latency: Duration::from_millis(1234),
        input_tokens: None,
        note: Some(note.to_string()),
        exchange: None,
    }
}

/// An API key no recorded exchange, screen or log line may ever contain.
pub(crate) const SENTINEL_KEY: &str = "sk-sentinel-7f3a9c";

/// A Jev exchange as the engine records it, key redacted: a request body, a 503 answer
/// whose text holds control characters, then a JSON answer.
pub(crate) fn jev_exchange() -> JevExchange {
    let answer = json!({
        "model": "jev-test",
        "answers": {
            "move": {
                "choice": "e4",
                "probabilities": { "e4": 0.62, "d4": 0.3 },
                "confidence": 0.81,
            }
        },
        "usage": { "input_tokens": 512 },
    });
    JevExchange {
        method: "POST".to_string(),
        url: JEV_ENDPOINT.to_string(),
        headers: vec![
            ("Authorization".to_string(), "Bearer <redacted>".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ],
        body: json!({
            "model": "jev-test",
            "state": { "side_to_move": "white", "note": "a quiet opening position" },
            "questions": {
                "move": {
                    "type": "choice",
                    "criteria": {
                        "d4": { "assessment": "good", "effect": "takes the centre" },
                        "e4": { "assessment": "good", "effect": "opens the bishop's diagonal" },
                    },
                }
            },
        }),
        attempts: vec![
            JevAttempt {
                status: Some(503),
                response: Some("busy \u{1b}[2J now\n\ttry again".to_string()),
                error: Some("HTTP 503: busy \u{1b}[2J now".to_string()),
                elapsed: Duration::from_millis(120),
            },
            JevAttempt {
                status: Some(200),
                response: Some(answer.to_string()),
                error: None,
                elapsed: Duration::from_millis(850),
            },
        ],
    }
}

/// [`jev_move`] with [`jev_exchange`] recorded, as a traced Jev move arrives.
pub(crate) fn traced_jev_move(pos: &ChessPosition, uci: &str) -> ComputerMove {
    ComputerMove {
        exchange: Some(Box::new(jev_exchange())),
        ..jev_move(pos, uci)
    }
}

/// What a [`FakeEngine`] does with one request.
pub(crate) enum Turn {
    /// Plays this UCI move, reported as [`jev_move`] reports it ([`local_move_for`] for
    /// [`FakeEngine::local`]).
    Play(&'static str),
    /// Finds no move, as for a finished game.
    GameOver,
    /// Panics with a `&str` payload.
    Panic(&'static str),
    /// Panics with a `String` payload.
    PanicString(String),
    /// Panics with a payload that is not a string.
    PanicOther,
    /// Sleeps this long, then panics with a `&str` payload.
    SlowPanic(Duration),
    /// Plays this UCI move with [`jev_exchange`] recorded ([`traced_jev_move`]); for
    /// [`FakeEngine::local`], which asks Jev nothing, as [`Turn::Play`].
    Traced(&'static str),
}

/// A scripted engine that never touches the network.
///
/// Each request takes the next scripted [`Turn`]. Once the script is used up it plays the
/// first legal move in UCI order, or finds none when the game is over. Its moves look like
/// the model's ([`FakeEngine::jev`], [`FakeEngine::laya`]) or like the local search's
/// ([`FakeEngine::local`], [`FakeEngine::local_laya`]). It records the name of every thread
/// it ran on.
pub(crate) struct FakeEngine {
    provider: Provider,
    enabled: bool,
    warnings: Vec<String>,
    script: Mutex<VecDeque<Turn>>,
    threads: Mutex<Vec<Option<String>>>,
}

impl FakeEngine {
    /// A player with a Jev key: status [`JEV_STATUS`].
    pub(crate) fn jev() -> FakeEngine {
        FakeEngine::new(Provider::Jev, true)
    }

    /// A player without a Jev key: status [`LOCAL_SEARCH_STATUS`], and moves reported as
    /// the local search's ([`local_move_for`]).
    pub(crate) fn local() -> FakeEngine {
        FakeEngine::new(Provider::Jev, false)
    }

    /// A Laya player with `LAYA_URL`: status [`LAYA_STATUS`], moves reported as Laya's.
    pub(crate) fn laya() -> FakeEngine {
        FakeEngine::new(Provider::Laya, true)
    }

    /// A Laya player without `LAYA_URL`: status [`LAYA_LOCAL_SEARCH_STATUS`], moves
    /// reported as the local search's with [`LAYA_LOCAL_NOTE`].
    pub(crate) fn local_laya() -> FakeEngine {
        FakeEngine::new(Provider::Laya, false)
    }

    fn new(provider: Provider, enabled: bool) -> FakeEngine {
        FakeEngine {
            provider,
            enabled,
            warnings: Vec::new(),
            script: Mutex::new(VecDeque::new()),
            threads: Mutex::new(Vec::new()),
        }
    }

    /// Queues `turns` after any already scripted.
    #[must_use]
    pub(crate) fn scripted(mut self, turns: impl IntoIterator<Item = Turn>) -> FakeEngine {
        self.script.get_mut().expect("script lock").extend(turns);
        self
    }

    /// Queues one [`Turn::Play`] per UCI move.
    #[must_use]
    pub(crate) fn playing(self, moves: &[&'static str]) -> FakeEngine {
        self.scripted(moves.iter().map(|&uci| Turn::Play(uci)))
    }

    /// Reports `warnings` as [`Engine::warnings`].
    #[must_use]
    pub(crate) fn with_warnings(mut self, warnings: &[&str]) -> FakeEngine {
        self.warnings = warnings.iter().map(|w| (*w).to_string()).collect();
        self
    }

    /// The name of the thread of every `choose` call so far, oldest first.
    pub(crate) fn threads(&self) -> Vec<Option<String>> {
        self.threads.lock().expect("threads lock").clone()
    }
}

impl Engine for FakeEngine {
    fn choose(&self, game: &Game) -> Option<ComputerMove> {
        self.threads
            .lock()
            .expect("threads lock")
            .push(thread::current().name().map(str::to_string));
        let turn = self.script.lock().expect("script lock").pop_front();
        let computer_move = |uci: &str| {
            if self.enabled {
                model_move(game.position(), uci, self.provider)
            } else {
                local_move_for(game.position(), uci, self.provider)
            }
        };
        match turn {
            Some(Turn::Play(uci)) => Some(computer_move(uci)),
            Some(Turn::Traced(uci)) if self.enabled => Some(ComputerMove {
                exchange: Some(Box::new(jev_exchange())),
                ..model_move(game.position(), uci, self.provider)
            }),
            Some(Turn::Traced(uci)) => Some(computer_move(uci)),
            Some(Turn::GameOver) => None,
            Some(Turn::Panic(message)) => panic::panic_any(message),
            Some(Turn::PanicString(message)) => panic::panic_any(message),
            Some(Turn::PanicOther) => panic::panic_any(42_u8),
            Some(Turn::SlowPanic(delay)) => {
                thread::sleep(delay);
                panic::panic_any("slow engine exploded")
            }
            None => {
                if game.outcome().is_some() {
                    return None;
                }
                let pos = game.position();
                let mut moves: Vec<Move> = pos.legal_moves().iter().copied().collect();
                moves.sort_by_key(|m| m.to_uci());
                let first = moves.first()?.to_uci();
                let mut computer = computer_move(&first);
                computer.top.truncate(1);
                Some(computer)
            }
        }
    }

    fn status(&self) -> String {
        match (self.provider, self.enabled) {
            (Provider::Jev, true) => JEV_STATUS.to_string(),
            (Provider::Jev, false) => LOCAL_SEARCH_STATUS.to_string(),
            (Provider::Laya, true) => LAYA_STATUS.to_string(),
            (Provider::Laya, false) => LAYA_LOCAL_SEARCH_STATUS.to_string(),
        }
    }

    fn provider(&self) -> Provider {
        self.provider
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn warnings(&self) -> Vec<String> {
        self.warnings.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::START_FEN;
    use crate::tui::test_support::game_from;

    #[test]
    fn the_local_fake_reports_local_search_moves() {
        let game = game_from(START_FEN, &[]);
        let engine = FakeEngine::local().scripted([Turn::Play("e2e4"), Turn::Traced("d2d4")]);
        for expected in ["e4", "d4", "a3"] {
            let computer = engine.choose(&game).expect("a move");
            assert_eq!(computer.san, expected);
            assert_eq!(computer.source, MoveSource::Fallback, "{expected}");
            assert!(computer.top.is_empty(), "{expected}");
            assert_eq!(
                (computer.confidence, computer.model, computer.input_tokens),
                (None, None, None),
                "{expected}"
            );
            assert_eq!(computer.note.as_deref(), Some(LOCAL_NOTE));
            assert!(
                computer.exchange.is_none(),
                "a local player records no exchange"
            );
        }
        assert!(!engine.enabled());
        assert_eq!(engine.status(), LOCAL_SEARCH_STATUS);
    }

    #[test]
    fn the_laya_fakes_report_laya() {
        let game = game_from(START_FEN, &[]);
        let laya = FakeEngine::laya().playing(&["e2e4"]);
        let computer = laya.choose(&game).expect("a move");
        assert_eq!(computer.provider, Provider::Laya);
        assert_eq!(computer.source, MoveSource::Model);
        assert_eq!(computer.model.as_deref(), Some("laya-test"));
        assert_eq!(laya.status(), LAYA_STATUS);
        assert!(laya.enabled());
        assert_eq!(Engine::provider(&laya), Provider::Laya);

        let local = FakeEngine::local_laya().playing(&["e2e4"]);
        let computer = local.choose(&game).expect("a move");
        assert_eq!(computer.provider, Provider::Laya);
        assert_eq!(computer.source, MoveSource::Fallback);
        assert_eq!(computer.note.as_deref(), Some(LAYA_LOCAL_NOTE));
        assert_eq!(local.status(), LAYA_LOCAL_SEARCH_STATUS);
        assert!(!local.enabled());
    }

    #[test]
    fn the_jev_fake_reports_jev_moves() {
        let game = game_from(START_FEN, &[]);
        let engine = FakeEngine::jev().scripted([Turn::Play("e2e4"), Turn::Traced("d2d4")]);
        let played = engine.choose(&game).expect("a move");
        assert_eq!(played, jev_move(game.position(), "e2e4"));
        let traced = engine.choose(&game).expect("a move");
        assert_eq!(traced, traced_jev_move(game.position(), "d2d4"));
        let unscripted = engine.choose(&game).expect("a move");
        assert_eq!((unscripted.san.as_str(), unscripted.top.len()), ("a3", 1));
        assert_eq!(unscripted.source, MoveSource::Model);
    }
}
