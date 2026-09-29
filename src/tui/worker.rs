//! Engine worker (spec 6.5): each computer turn runs on its own thread named
//! "engine", so the UI never blocks on the (up to ~19 s) Jev round trip or on the
//! local search.
//!
//! A request carries a generation counter and the position hash. The reply
//! echoes both, and the app applies it only when [`is_current`] says the game has
//! not moved on (undo, new game, load or resign bump the generation), so a legal
//! but stale reply is never played. Threads are detached and never joined:
//! quitting must not wait for a slow HTTP call.
//!
//! When the engine panics, the same thread runs the local search itself and
//! replies with its best move, noted [`ENGINE_ERROR_NOTE`]. The UI thread only
//! ever applies ready-made moves: a search can take tens of seconds on a crowded
//! position, and a panic in it must not reach the UI.
//!
//! In debug mode the engine records its exchange with Jev in the move; the thread
//! moves it into the reply as a [`debug::Exchange`](super::debug::Exchange). Its text
//! is rendered on the UI thread, and only while the exchange view shows it.

use std::any::Any;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use super::debug::Exchange;
use crate::core::Game;
use crate::engine::{
    ComputerMove, ComputerPlayer, EngineConfig, MoveChooser, MoveSource, Provider, analyse,
    printable,
};

/// Name of every engine thread. The panic hook in `terminal` leaves the terminal
/// alone for panics on any thread other than "main", including this one.
const ENGINE_THREAD: &str = "engine";

/// The note on a move the worker found by local search after the engine panicked.
/// The app shows it in the status panel as well as in the Jev panel.
pub const ENGINE_ERROR_NOTE: &str = "engine error — local search";

/// [`Engine::status`] of a player without a Jev key.
pub const LOCAL_SEARCH_STATUS: &str = "No JEV_API_KEY — local search";
/// [`Engine::status`] of a Laya player without `LAYA_URL`.
pub const LAYA_LOCAL_SEARCH_STATUS: &str = "No LAYA_URL — local search";
/// Longest part of `LAYA_URL` shown in the Laya status.
const STATUS_URL_CHARS: usize = 40;

/// Something that picks computer moves. `ComputerPlayer` in production; tests use
/// scripted fakes so they never touch the network.
pub trait Engine: Send + Sync {
    /// The move to play in `game`, or `None` when the game is over. May block for
    /// many seconds, so only [`spawn_request`] calls it, off the UI thread.
    fn choose(&self, game: &Game) -> Option<ComputerMove>;

    /// One line for the menu: `Jev ready (<model>)`, `Laya ready (<url>)`,
    /// [`LOCAL_SEARCH_STATUS`] or [`LAYA_LOCAL_SEARCH_STATUS`].
    fn status(&self) -> String;

    /// The model this engine asks.
    fn provider(&self) -> Provider;

    /// True when the model is available (Jev has a key, Laya a URL); otherwise every
    /// move comes from the local search and the UI calls the player "Local search".
    fn enabled(&self) -> bool;

    /// Human-readable notes about ignored or adjusted settings, shown on the menu.
    fn warnings(&self) -> Vec<String>;
}

impl<C: MoveChooser> Engine for ComputerPlayer<C> {
    fn choose(&self, game: &Game) -> Option<ComputerMove> {
        self.choose_move(game)
    }

    fn status(&self) -> String {
        status_for(self.config())
    }

    fn provider(&self) -> Provider {
        self.config().provider
    }

    /// `ComputerPlayer` does not expose whether it holds a chooser, so this is
    /// read from its config: `ComputerPlayer::from_config` builds a client exactly
    /// when the config is enabled, which makes the two equivalent for the player the
    /// TUI runs.
    fn enabled(&self) -> bool {
        self.config().enabled()
    }

    fn warnings(&self) -> Vec<String> {
        self.config().warnings.clone()
    }
}

/// The menu status line for a player built from `config`.
fn status_for(config: &EngineConfig) -> String {
    match (config.provider, config.enabled()) {
        (Provider::Jev, true) => format!("Jev ready ({})", config.model),
        (Provider::Jev, false) => LOCAL_SEARCH_STATUS.to_string(),
        (Provider::Laya, true) => {
            format!(
                "Laya ready ({})",
                printable(&config.endpoint, STATUS_URL_CHARS)
            )
        }
        (Provider::Laya, false) => LAYA_LOCAL_SEARCH_STATUS.to_string(),
    }
}

/// One computer turn to compute.
#[derive(Clone, Debug)]
pub struct EngineRequest {
    /// The app's generation counter when the request was made.
    pub generation: u64,
    /// `game.position().hash()` when the request was made.
    pub hash: u64,
    /// A snapshot of the game; the engine thread owns it.
    pub game: Game,
    /// The model that must answer: the player of the side to move.
    pub provider: Provider,
}

impl EngineRequest {
    /// A request to `provider` for `game`'s current position, stamped with `generation`.
    #[must_use]
    pub fn new(generation: u64, game: Game, provider: Provider) -> EngineRequest {
        EngineRequest {
            generation,
            hash: game.position().hash(),
            game,
            provider,
        }
    }
}

/// What the engine thread produced.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineOutcome {
    /// The move to play and how it was chosen. After an engine panic this is the
    /// local search's best move, with source `Fallback` and note
    /// [`ENGINE_ERROR_NOTE`].
    Move(ComputerMove),
    /// The engine found no move: the game is over.
    GameOver,
    /// No move could be produced: the engine panicked and so did the local
    /// search, or no engine thread could be started. The text says why. The app
    /// stops asking until the person retries.
    Failed(String),
}

/// The answer to one [`EngineRequest`], echoing its generation and hash.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineReply {
    /// Copied from the request.
    pub generation: u64,
    /// Copied from the request.
    pub hash: u64,
    /// What the engine produced.
    pub outcome: EngineOutcome,
    /// The exchange with Jev behind a move, when the engine recorded one (debug mode).
    /// Taken out of the move, so [`EngineOutcome::Move`] never holds it.
    pub exchange: Option<Box<Exchange>>,
}

impl EngineReply {
    /// The reply to `request` with `outcome`. The exchange a traced move carries
    /// (`ComputerMove::exchange`) moves into [`EngineReply::exchange`], with the move
    /// and the position it was for.
    #[must_use]
    pub fn new(request: &EngineRequest, mut outcome: EngineOutcome) -> EngineReply {
        let exchange = match &mut outcome {
            EngineOutcome::Move(computer) => computer
                .exchange
                .take()
                .map(|http| Box::new(Exchange::new(&request.game, computer, *http))),
            EngineOutcome::GameOver | EngineOutcome::Failed(_) => None,
        };
        EngineReply {
            generation: request.generation,
            hash: request.hash,
            outcome,
            exchange,
        }
    }
}

/// Runs `request` on a new thread named "engine" and sends the reply on `tx`. A
/// panic inside the engine is answered with the local search's best move (see
/// [`EngineOutcome::Move`]); only when that panics as well is the reply
/// [`EngineOutcome::Failed`]. Nothing is computed on the calling thread. If the
/// receiver is gone (the UI quit), the reply is dropped.
///
/// The thread's handle is returned for tests, which join it; the app drops it, which
/// detaches the thread, so quitting never waits for a slow engine call.
///
/// # Errors
///
/// Only when the OS cannot spawn the thread; no reply will arrive.
pub fn spawn_request(
    engine: Arc<dyn Engine>,
    request: EngineRequest,
    tx: Sender<EngineReply>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name(ENGINE_THREAD.to_string())
        .spawn(move || {
            let outcome = run_engine(
                engine.as_ref(),
                &request.game,
                request.provider,
                local_search_move,
            );
            // A send error means the UI has gone; nobody is left to tell.
            let _ = tx.send(EngineReply::new(&request, outcome));
        })
}

/// True when `reply` answers the request for the app's current `generation`
/// and position `hash`; anything else is stale and must be discarded.
#[must_use]
pub fn is_current(reply: &EngineReply, generation: u64, hash: u64) -> bool {
    reply.generation == generation && reply.hash == hash
}

/// Calls the engine; if it panics, calls `fallback` (the local search in
/// production), and if that panics too, reports both panics as
/// [`EngineOutcome::Failed`]. `started` is when the turn began, so a fallback
/// move's latency covers the failed engine call as well.
fn run_engine(
    engine: &dyn Engine,
    game: &Game,
    provider: Provider,
    fallback: fn(&Game, Instant, Provider) -> Option<ComputerMove>,
) -> EngineOutcome {
    let started = Instant::now();
    // `ComputerPlayer<JevClient>` holds ureq state that is not `UnwindSafe`. After
    // a panic the engine is only ever asked again from scratch, with a fresh game
    // snapshot, so no half-updated state is observed.
    let engine_panic = match panic::catch_unwind(AssertUnwindSafe(|| engine.choose(game))) {
        Ok(Some(mv)) => return EngineOutcome::Move(mv),
        Ok(None) => return EngineOutcome::GameOver,
        Err(payload) => panic_message("engine", payload.as_ref()),
    };
    log::warn!("{engine_panic}; falling back to local search");
    match panic::catch_unwind(AssertUnwindSafe(|| fallback(game, started, provider))) {
        Ok(Some(mv)) => EngineOutcome::Move(mv),
        Ok(None) => EngineOutcome::GameOver,
        Err(payload) => EngineOutcome::Failed(format!(
            "{engine_panic}; {}",
            panic_message("local search", payload.as_ref())
        )),
    }
}

/// The local search's best move for `game`, credited to `provider` (the engine that
/// panicked) and noted [`ENGINE_ERROR_NOTE`], or
/// `None` when the game is over.
fn local_search_move(game: &Game, started: Instant, provider: Provider) -> Option<ComputerMove> {
    if game.outcome().is_some() {
        return None;
    }
    let best = analyse(game).first()?.mv;
    Some(ComputerMove {
        mv: best,
        san: game.position().to_san(best),
        source: MoveSource::Fallback,
        provider,
        top: Vec::new(),
        confidence: None,
        model: None,
        latency: started.elapsed(),
        input_tokens: None,
        note: Some(ENGINE_ERROR_NOTE.to_string()),
        exchange: None,
    })
}

/// `<what> panicked: <detail>` for a panic payload (`panic!` produces `&str` or
/// `String`), or `<what> panicked` for any other payload.
fn panic_message(what: &str, payload: &(dyn Any + Send)) -> String {
    let detail = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
    match detail {
        Some(detail) => format!("{what} panicked: {detail}"),
        None => format!("{what} panicked"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::engine::{JevClient, Provider};
    use crate::tui::test_support::engine::{
        FakeEngine, REPLY_TIMEOUT, Turn, jev_exchange, jev_move,
    };
    use crate::tui::test_support::game_from;

    /// Fool's mate: White is checkmated, so nobody has a move.
    const MATED: &str = "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3";

    /// An engine that takes `turn` for its first request.
    fn fake(turn: Turn) -> Arc<FakeEngine> {
        Arc::new(FakeEngine::jev().scripted([turn]))
    }

    /// Spawns `request` on `engine` and waits for its reply.
    fn round_trip(engine: Arc<dyn Engine>, request: EngineRequest) -> EngineReply {
        let (tx, rx) = mpsc::channel();
        spawn_request(engine, request, tx).expect("engine thread spawns");
        rx.recv_timeout(REPLY_TIMEOUT).expect("engine replies")
    }

    /// A fallback that panics like `panic!` with a message.
    fn exploding_search(_: &Game, _: Instant, _: Provider) -> Option<ComputerMove> {
        panic!("search exploded")
    }

    /// A fallback that panics with a payload that is not a string.
    fn exploding_search_silently(_: &Game, _: Instant, _: Provider) -> Option<ComputerMove> {
        panic::panic_any(7_u8)
    }

    /// Runs `turn` with a fallback that panics too, and returns the failure text.
    fn double_failure(
        turn: Turn,
        fallback: fn(&Game, Instant, Provider) -> Option<ComputerMove>,
    ) -> String {
        let engine = fake(turn);
        match run_engine(engine.as_ref(), &Game::new(), Provider::Jev, fallback) {
            EngineOutcome::Failed(message) => message,
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn request_new_stamps_the_position_hash() {
        let game = Game::new();
        let request = EngineRequest::new(7, game.clone(), Provider::Jev);
        assert_eq!(request.generation, 7);
        assert_eq!(request.hash, game.position().hash());
    }

    #[test]
    fn normal_move_echoes_generation_and_hash() {
        let game = Game::new();
        let engine = fake(Turn::Play("e2e4"));
        let reply = round_trip(
            engine.clone(),
            EngineRequest::new(3, game.clone(), Provider::Jev),
        );

        assert_eq!(reply.generation, 3);
        assert_eq!(reply.hash, game.position().hash());
        assert_eq!(
            reply.outcome,
            EngineOutcome::Move(jev_move(game.position(), "e2e4"))
        );
        assert_eq!(engine.threads(), vec![Some("engine".to_string())]);
    }

    #[test]
    fn a_traced_move_sends_its_exchange_beside_it() {
        let game = game_from(crate::core::START_FEN, &["e2e4"]);
        let reply = round_trip(
            fake(Turn::Traced("e7e5")),
            EngineRequest::new(2, game.clone(), Provider::Jev),
        );
        assert_eq!(
            reply.outcome,
            EngineOutcome::Move(jev_move(game.position(), "e7e5")),
            "the move no longer holds the exchange"
        );
        let exchange = reply.exchange.expect("the exchange travels in the reply");
        assert_eq!(exchange.http, jev_exchange());
        assert_eq!((exchange.ply, exchange.fullmove), (1, 1));
        assert_eq!(exchange.san, "e5");
        assert_eq!(exchange.source, "Jev");
        assert!(
            !exchange.body().is_empty(),
            "its body renders when it is shown"
        );
    }

    #[test]
    fn untraced_replies_carry_no_exchange() {
        let request = EngineRequest::new(1, Game::new(), Provider::Jev);
        assert_eq!(
            round_trip(fake(Turn::Play("e2e4")), request.clone()).exchange,
            None
        );
        assert_eq!(
            round_trip(fake(Turn::GameOver), request.clone()).exchange,
            None
        );
        let failed = EngineReply::new(&request, EngineOutcome::Failed("no thread".into()));
        assert_eq!(
            (failed.generation, failed.hash, failed.exchange),
            (1, request.hash, None)
        );
    }

    #[test]
    fn no_move_means_game_over() {
        let reply = round_trip(
            fake(Turn::GameOver),
            EngineRequest::new(5, Game::new(), Provider::Jev),
        );
        assert_eq!(reply.generation, 5);
        assert_eq!(reply.outcome, EngineOutcome::GameOver);
    }

    #[test]
    fn a_panic_is_answered_with_the_local_search_move() {
        for turn in [
            Turn::Panic("chooser exploded"),
            Turn::PanicString("chooser exploded: 42".to_string()),
            Turn::PanicOther,
        ] {
            let game = Game::new();
            let reply = round_trip(
                fake(turn),
                EngineRequest::new(9, game.clone(), Provider::Jev),
            );
            assert_eq!(reply.generation, 9);
            assert_eq!(reply.hash, game.position().hash());
            let EngineOutcome::Move(mv) = reply.outcome else {
                panic!("expected the fallback move, got {:?}", reply.outcome);
            };
            let best = analyse(&game)[0].mv;
            assert_eq!(mv.mv, best);
            assert_eq!(mv.san, game.position().to_san(best));
            assert_eq!(mv.source, MoveSource::Fallback);
            assert_eq!(mv.note.as_deref(), Some(ENGINE_ERROR_NOTE));
            assert!(mv.top.is_empty() && mv.confidence.is_none() && mv.model.is_none());
        }
    }

    #[test]
    fn the_fallback_latency_includes_the_failed_call() {
        let reply = round_trip(
            fake(Turn::SlowPanic(Duration::from_millis(60))),
            EngineRequest::new(1, Game::new(), Provider::Jev),
        );
        let EngineOutcome::Move(mv) = reply.outcome else {
            panic!("expected the fallback move, got {:?}", reply.outcome);
        };
        assert!(mv.latency >= Duration::from_millis(60), "{:?}", mv.latency);
    }

    #[test]
    fn a_panic_in_a_finished_game_is_game_over() {
        // Fool's mate: White is checkmated, so the local search has no move either.
        let mate = Game::from_fen(MATED).unwrap();
        let reply = round_trip(
            fake(Turn::Panic("chooser exploded")),
            EngineRequest::new(1, mate, Provider::Jev),
        );
        assert_eq!(reply.outcome, EngineOutcome::GameOver);
    }

    #[test]
    fn failed_only_when_the_local_search_panics_too() {
        assert_eq!(
            double_failure(Turn::Panic("chooser exploded"), exploding_search),
            "engine panicked: chooser exploded; local search panicked: search exploded"
        );
        assert_eq!(
            double_failure(
                Turn::PanicString("chooser exploded: 42".to_string()),
                exploding_search
            ),
            "engine panicked: chooser exploded: 42; local search panicked: search exploded"
        );
        assert_eq!(
            double_failure(Turn::PanicOther, exploding_search_silently),
            "engine panicked; local search panicked"
        );
    }

    #[test]
    fn a_working_engine_never_reaches_the_fallback() {
        let engine = fake(Turn::Play("g1f3"));
        let outcome = run_engine(
            engine.as_ref(),
            &Game::new(),
            Provider::Jev,
            exploding_search,
        );
        assert!(matches!(outcome, EngineOutcome::Move(ref mv) if mv.san == "Nf3"));
        let engine = fake(Turn::GameOver);
        let outcome = run_engine(
            engine.as_ref(),
            &Game::new(),
            Provider::Jev,
            exploding_search,
        );
        assert_eq!(outcome, EngineOutcome::GameOver);
    }

    #[test]
    fn the_same_engine_answers_again_after_a_panic() {
        let engine = Arc::new(
            FakeEngine::jev().scripted([Turn::Panic("chooser exploded"), Turn::Play("g1f3")]),
        );
        let first = round_trip(
            engine.clone(),
            EngineRequest::new(1, Game::new(), Provider::Jev),
        );
        assert!(matches!(
            &first.outcome,
            EngineOutcome::Move(mv) if mv.note.as_deref() == Some(ENGINE_ERROR_NOTE)
        ));
        let second = round_trip(
            engine.clone(),
            EngineRequest::new(2, Game::new(), Provider::Jev),
        );
        assert_eq!(second.generation, 2);
        assert!(matches!(
            &second.outcome,
            EngineOutcome::Move(mv) if mv.source == MoveSource::Model && mv.san == "Nf3"
        ));
        assert_eq!(engine.threads().len(), 2, "one engine, asked twice");
    }

    #[test]
    fn reply_to_a_closed_channel_is_dropped_quietly() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let engine = fake(Turn::Play("e2e4"));
        let thread = spawn_request(
            engine.clone(),
            EngineRequest::new(1, Game::new(), Provider::Jev),
            tx,
        )
        .expect("engine thread spawns");
        // The thread ends after its send, so a panic there would fail the join.
        thread
            .join()
            .expect("the send to the closed channel does not panic");
        assert_eq!(engine.threads().len(), 1, "the engine was asked");
    }

    #[test]
    fn staleness_needs_both_generation_and_hash() {
        let reply = EngineReply {
            generation: 4,
            hash: 0xABCD,
            outcome: EngineOutcome::GameOver,
            exchange: None,
        };
        assert!(is_current(&reply, 4, 0xABCD));
        assert!(!is_current(&reply, 5, 0xABCD), "generation moved on");
        assert!(!is_current(&reply, 4, 0x1234), "position changed");
        assert!(!is_current(&reply, 3, 0x1234));
    }

    #[test]
    fn stale_after_the_game_moves_on() {
        let mut game = Game::new();
        let request = EngineRequest::new(1, game.clone(), Provider::Jev);
        let reply = round_trip(fake(Turn::Play("e2e4")), request);
        assert!(is_current(&reply, 1, game.position().hash()));

        let e4 = game.position().parse_uci("e2e4").unwrap();
        game.play(e4).unwrap();
        assert!(!is_current(&reply, 1, game.position().hash()));
    }

    #[test]
    fn computer_player_status_without_key() {
        let player = ComputerPlayer::<JevClient>::new(None, EngineConfig::default());
        assert_eq!(player.status(), LOCAL_SEARCH_STATUS);
        assert!(Engine::warnings(&player).is_empty());
    }

    #[test]
    fn computer_player_status_with_key_names_the_model() {
        // Building the client is offline; only `choose` would reach the network.
        let config = EngineConfig::from_vars(|name| match name {
            "JEV_API_KEY" => Some("test-key".to_string()),
            "JEV_MODEL" => Some("jev-latest".to_string()),
            _ => None,
        });
        let player = ComputerPlayer::from_config(config);
        assert_eq!(player.status(), "Jev ready (jev-latest)");
    }

    #[test]
    fn computer_player_is_enabled_exactly_when_a_key_is_set() {
        // Building the client is offline; only `choose` would reach the network.
        let with_key = ComputerPlayer::from_config(EngineConfig {
            api_key: Some("test-key".to_string()),
            ..EngineConfig::default()
        });
        assert!(Engine::enabled(&with_key));
        let without_key = ComputerPlayer::from_config(EngineConfig::default());
        assert!(!Engine::enabled(&without_key));
    }

    #[test]
    fn computer_player_passes_config_warnings_through() {
        let config =
            EngineConfig::from_vars(|name| (name == "JEV_MAX_OPTIONS").then(|| "lots".to_string()));
        let expected = config.warnings.clone();
        assert!(!expected.is_empty());
        let player = ComputerPlayer::<JevClient>::new(None, config);
        assert_eq!(Engine::warnings(&player), expected);
    }

    #[test]
    fn computer_player_without_chooser_runs_on_the_worker() {
        // No chooser: local search only, fully offline.
        let player: Arc<dyn Engine> = Arc::new(ComputerPlayer::<JevClient>::new(
            None,
            EngineConfig::default(),
        ));
        let game = Game::new();
        let reply = round_trip(
            player.clone(),
            EngineRequest::new(1, game.clone(), Provider::Jev),
        );
        let EngineOutcome::Move(mv) = reply.outcome else {
            panic!("expected a move, got {:?}", reply.outcome);
        };
        assert_eq!(mv.source, MoveSource::Fallback);
        assert!(game.position().legal_moves().contains(&mv.mv));

        // Fool's mate: White is checkmated, so there is no move.
        let mate = Game::from_fen(MATED).unwrap();
        let reply = round_trip(player, EngineRequest::new(2, mate, Provider::Jev));
        assert_eq!(reply.outcome, EngineOutcome::GameOver);
    }

    #[test]
    fn laya_status_and_provider() {
        let off = ComputerPlayer::from_config(EngineConfig::laya_from_vars(|_| None));
        assert_eq!(off.status(), LAYA_LOCAL_SEARCH_STATUS);
        assert_eq!(Engine::provider(&off), Provider::Laya);
        assert!(!Engine::enabled(&off));

        let url = "http://127.0.0.1:8000/v1/systemone";
        let on = ComputerPlayer::from_config(EngineConfig::laya_from_vars(|name| {
            (name == "LAYA_URL").then(|| url.to_string())
        }));
        assert_eq!(on.status(), format!("Laya ready ({url})"));
        assert!(Engine::enabled(&on));

        let long = format!("http://{}/v1/systemone", "h".repeat(60));
        let long = ComputerPlayer::from_config(EngineConfig::laya_from_vars(|name| {
            (name == "LAYA_URL").then(|| long.clone())
        }));
        let status = long.status();
        assert!(
            status.chars().count() <= "Laya ready ()".len() + 40,
            "{status}"
        );
    }

    #[test]
    fn a_request_carries_its_provider() {
        let request = EngineRequest::new(7, Game::new(), Provider::Laya);
        assert_eq!(request.provider, Provider::Laya);
        assert_eq!(request.generation, 7);
    }

    #[test]
    fn a_laya_panic_falls_back_to_a_move_credited_to_laya() {
        let engine = Arc::new(FakeEngine::laya().scripted([Turn::Panic("laya exploded")]));
        let reply = round_trip(engine, EngineRequest::new(1, Game::new(), Provider::Laya));
        let EngineOutcome::Move(mv) = reply.outcome else {
            panic!("expected a move, got {:?}", reply.outcome);
        };
        assert_eq!(mv.source, MoveSource::Fallback);
        assert_eq!(mv.note.as_deref(), Some(ENGINE_ERROR_NOTE));
        assert_eq!(mv.provider, Provider::Laya);
    }
}
