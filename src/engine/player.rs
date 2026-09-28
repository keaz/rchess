//! The computer player (spec 5.6): forced moves are played directly, everything
//! else goes through a shortlist, one Jev `choice` question and a veto check.
//! It never fails: any problem degrades to the local search's best move.

use std::fmt;
use std::time::{Duration, Instant};

use crate::core::{Game, Move};

use super::annotate::{Annotation, Bucket, annotate};
use super::config::{EngineConfig, MAX_CHOICE_OPTIONS};
use super::describe::describe;
use super::jev::{ChoiceOption, ChoiceRequest, JevClient, JevExchange, MoveChooser, printable};
use super::search::{MATE, analyse};

/// Longest part of an unknown option key echoed back in a note.
const NOTE_KEY_CHARS: usize = 40;
const QUESTION: &str = "You play `side_to_move`. Which move should we play?";
const GUIDANCE: &str = "Each option says what the move does and the engine's assessment: \
winning, good, neutral, bad or losing. Prefer winning and good moves. Among similar moves, \
prefer ones that remove threats against our pieces and keep our king safe.";

/// Where a computer move came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveSource {
    /// Jev's pick.
    Jev,
    /// The only legal move; Jev was not asked.
    OnlyMove,
    /// A mate in one; Jev was not asked.
    MateInOne,
    /// Jev picked `jev_pick`, but it scored too far below the search best.
    Vetoed {
        /// SAN of the option Jev picked.
        jev_pick: String,
    },
    /// Jev was unavailable or answered unusably; the search best was played.
    Fallback,
}

impl fmt::Display for MoveSource {
    /// A short label for the TUI: `Jev`, `only move`, `mate in one`,
    /// `vetoed (Jev picked Qxd5)` or `local search`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveSource::Jev => f.write_str("Jev"),
            MoveSource::OnlyMove => f.write_str("only move"),
            MoveSource::MateInOne => f.write_str("mate in one"),
            MoveSource::Vetoed { jev_pick } => write!(f, "vetoed (Jev picked {jev_pick})"),
            MoveSource::Fallback => f.write_str("local search"),
        }
    }
}

/// A chosen move plus everything the TUI shows about how it was chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputerMove {
    /// The move to play.
    pub mv: Move,
    /// The move in SAN.
    pub san: String,
    /// Where the move came from.
    pub source: MoveSource,
    /// Up to three Jev options with probabilities, most likely first.
    pub top: Vec<(String, f32)>,
    /// Jev's confidence in its choice, when Jev answered.
    pub confidence: Option<f32>,
    /// Versioned model ID that answered.
    pub model: Option<String>,
    /// Time for the whole `choose_move` call.
    pub latency: Duration,
    /// Input tokens billed, when Jev answered.
    pub input_tokens: Option<u32>,
    /// Why a fallback or veto happened.
    pub note: Option<String>,
    /// The HTTP exchange with Jev, when [`EngineConfig::trace`] is on and Jev was
    /// asked (a Jev or vetoed move, or a fallback after Jev failed or answered
    /// unusably); `None` otherwise.
    pub exchange: Option<Box<JevExchange>>,
}

/// Chooses computer moves: local search, plus Jev through `C` when available.
pub struct ComputerPlayer<C: MoveChooser> {
    chooser: Option<C>,
    config: EngineConfig,
}

impl<C: MoveChooser> fmt::Debug for ComputerPlayer<C> {
    /// Shows the config (its API key redacted) and whether a chooser is present.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComputerPlayer")
            .field("has_chooser", &self.chooser.is_some())
            .field("config", &self.config)
            .finish()
    }
}

impl ComputerPlayer<JevClient> {
    /// A player using the real Jev client when the config has a key.
    pub fn from_config(config: EngineConfig) -> ComputerPlayer<JevClient> {
        ComputerPlayer::new(JevClient::new(&config), config)
    }
}

impl<C: MoveChooser> ComputerPlayer<C> {
    /// A player that asks `chooser`. `None` means local search only: every move
    /// that would go to Jev is the search best, reported as `Fallback`.
    pub fn new(chooser: Option<C>, config: EngineConfig) -> ComputerPlayer<C> {
        ComputerPlayer { chooser, config }
    }

    /// The configuration this player was built with.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// The move to play, or `None` when the game is over.
    pub fn choose_move(&self, game: &Game) -> Option<ComputerMove> {
        let started = Instant::now();
        if game.outcome().is_some() {
            return None;
        }
        let pos = *game.position();
        let scored = analyse(game);
        let best = *scored.first()?;
        let plain = |mv: Move, source: MoveSource, note: Option<String>| ComputerMove {
            mv,
            san: pos.to_san(mv),
            source,
            top: Vec::new(),
            confidence: None,
            model: None,
            latency: started.elapsed(),
            input_tokens: None,
            note,
            exchange: None,
        };

        if scored.len() == 1 {
            return Some(plain(best.mv, MoveSource::OnlyMove, None));
        }
        if best.score == MATE - 1 {
            return Some(plain(best.mv, MoveSource::MateInOne, None));
        }
        let Some(chooser) = &self.chooser else {
            let note = "JEV_API_KEY not set — local search".to_string();
            return Some(plain(best.mv, MoveSource::Fallback, Some(note)));
        };

        let annotations = annotate(&pos, &scored);
        let candidates = shortlist(
            &annotations,
            self.config.filter_losing,
            self.config.max_options,
        );
        let request = ChoiceRequest {
            state: serde_json::to_value(describe(game)).expect("state serializes"),
            question: QUESTION.to_string(),
            guidance: GUIDANCE.to_string(),
            options: candidates
                .iter()
                .map(|a| ChoiceOption {
                    key: a.san.clone(),
                    effect: a.effect.clone(),
                    assessment: a.bucket,
                })
                .collect(),
        };

        let mut trace = None;
        let answer = if self.config.trace {
            chooser.choose_traced(&request, &mut trace)
        } else {
            chooser.choose(&request)
        };
        let exchange = trace.map(Box::new);
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                let note = format!("Jev unavailable ({error}) — local search");
                return Some(ComputerMove {
                    exchange,
                    ..plain(best.mv, MoveSource::Fallback, Some(note))
                });
            }
        };
        let Some(pick) = candidates.iter().find(|a| a.san == answer.choice) else {
            let cut = if answer.choice.chars().count() > NOTE_KEY_CHARS {
                "…"
            } else {
                ""
            };
            let note = format!(
                "Jev returned an unknown option ({}{cut}) — local search",
                printable(&answer.choice, NOTE_KEY_CHARS)
            );
            return Some(ComputerMove {
                exchange,
                ..plain(best.mv, MoveSource::Fallback, Some(note))
            });
        };

        let margin = self.config.veto_margin_cp;
        let (chosen, source, note) = if best.score - pick.score > margin {
            // Highest-probability option within the margin; the search best always qualifies.
            let fallback = candidates[0];
            let chosen = answer
                .probabilities
                .iter()
                .filter_map(|(key, _)| candidates.iter().find(|a| &a.san == key))
                .find(|a| best.score - a.score <= margin)
                .unwrap_or(&fallback);
            let note = format!(
                "Jev picked {}, which the search rates much worse; played {}",
                pick.san, chosen.san
            );
            (
                *chosen,
                MoveSource::Vetoed {
                    jev_pick: pick.san.clone(),
                },
                Some(note),
            )
        } else {
            (*pick, MoveSource::Jev, None)
        };

        Some(ComputerMove {
            mv: chosen.mv,
            san: chosen.san.clone(),
            source,
            top: answer
                .probabilities
                .iter()
                .filter(|(key, _)| candidates.iter().any(|a| &a.san == key))
                .take(3)
                .cloned()
                .collect(),
            confidence: Some(answer.confidence),
            model: Some(answer.model),
            latency: started.elapsed(),
            input_tokens: Some(answer.input_tokens),
            note,
            exchange,
        })
    }
}

/// Options for Jev: drop `losing` moves when filtering is on and anything else
/// exists, keep search order (best first), cap at `max_options` (1..=255).
fn shortlist(
    annotations: &[Annotation],
    filter_losing: bool,
    max_options: usize,
) -> Vec<&Annotation> {
    let any_non_losing = annotations.iter().any(|a| a.bucket != Bucket::Losing);
    annotations
        .iter()
        .filter(|a| !(filter_losing && any_non_losing && a.bucket == Bucket::Losing))
        .take(max_options.clamp(1, MAX_CHOICE_OPTIONS))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::jev::{ChoiceAnswer, JevAttempt, JevError, JevExchange, http_error};
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// What the mock returns: a fixed answer or an error.
    enum Reply {
        Answer {
            choice: &'static str,
            probabilities: Vec<(&'static str, f32)>,
        },
        Error(JevError),
    }

    struct MockChooser {
        reply: Reply,
        seen: Mutex<Vec<ChoiceRequest>>,
        /// How many of the requests came through `choose_traced`.
        traced: AtomicUsize,
    }

    impl MockChooser {
        fn answering(choice: &'static str, probabilities: Vec<(&'static str, f32)>) -> MockChooser {
            MockChooser {
                reply: Reply::Answer {
                    choice,
                    probabilities,
                },
                seen: Mutex::new(Vec::new()),
                traced: AtomicUsize::new(0),
            }
        }

        fn failing(error: JevError) -> MockChooser {
            MockChooser {
                reply: Reply::Error(error),
                seen: Mutex::new(Vec::new()),
                traced: AtomicUsize::new(0),
            }
        }

        fn requests(&self) -> Vec<ChoiceRequest> {
            self.seen.lock().unwrap().clone()
        }

        fn traced_calls(&self) -> usize {
            self.traced.load(Ordering::SeqCst)
        }
    }

    /// The exchange every traced mock call records, answer or error.
    fn mock_exchange() -> JevExchange {
        JevExchange {
            method: "POST".to_string(),
            url: "http://mock/v1/systemone".to_string(),
            headers: vec![("Authorization".to_string(), "Bearer <redacted>".to_string())],
            body: json!({ "model": "jev-latest" }),
            attempts: vec![JevAttempt {
                status: Some(200),
                response: Some("{}".to_string()),
                error: None,
                elapsed: Duration::from_millis(7),
            }],
        }
    }

    impl MoveChooser for MockChooser {
        fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
            self.seen.lock().unwrap().push(request.clone());
            match &self.reply {
                Reply::Answer {
                    choice,
                    probabilities,
                } => Ok(ChoiceAnswer {
                    choice: choice.to_string(),
                    probabilities: probabilities
                        .iter()
                        .map(|(k, p)| (k.to_string(), *p))
                        .collect(),
                    confidence: 0.8,
                    model: "jev-1.13.0".to_string(),
                    input_tokens: 900,
                }),
                Reply::Error(error) => Err(error.clone()),
            }
        }

        fn choose_traced(
            &self,
            request: &ChoiceRequest,
            trace: &mut Option<JevExchange>,
        ) -> Result<ChoiceAnswer, JevError> {
            self.traced.fetch_add(1, Ordering::SeqCst);
            *trace = Some(mock_exchange());
            self.choose(request)
        }
    }

    /// A chooser that keeps the default `choose_traced`, which records nothing.
    struct Untraced(MockChooser);

    impl MoveChooser for Untraced {
        fn choose(&self, request: &ChoiceRequest) -> Result<ChoiceAnswer, JevError> {
            self.0.choose(request)
        }
    }

    fn player(mock: MockChooser) -> ComputerPlayer<MockChooser> {
        ComputerPlayer::new(Some(mock), EngineConfig::default())
    }

    /// A player with `trace` on, as the TUI builds it in debug mode.
    fn traced_player(mock: MockChooser) -> ComputerPlayer<MockChooser> {
        let config = EngineConfig {
            trace: true,
            ..EngineConfig::default()
        };
        ComputerPlayer::new(Some(mock), config)
    }

    fn game(fen: &str) -> Game {
        Game::from_fen(fen).unwrap()
    }

    /// Qxd5 loses the queen to cxd5.
    const HANGING_QUEEN_TRAP: &str = "4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1";

    #[test]
    fn no_move_when_the_game_is_over() {
        let mate = game("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3");
        assert_eq!(
            player(MockChooser::answering("x", vec![])).choose_move(&mate),
            None
        );
    }

    #[test]
    fn only_move_skips_jev() {
        let mock = MockChooser::answering("x", vec![]);
        let p = player(mock);
        let result = p
            .choose_move(&game("7k/8/8/8/8/8/6q1/7K w - - 0 1"))
            .unwrap();
        assert_eq!(result.san, "Kxg2");
        assert_eq!(result.source, MoveSource::OnlyMove);
        assert!(p.chooser.as_ref().unwrap().requests().is_empty());
    }

    #[test]
    fn mate_in_one_skips_jev() {
        let p = player(MockChooser::answering("x", vec![]));
        let result = p
            .choose_move(&game("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1"))
            .unwrap();
        assert_eq!(result.san, "Re8#");
        assert_eq!(result.source, MoveSource::MateInOne);
        assert!(p.chooser.as_ref().unwrap().requests().is_empty());
    }

    #[test]
    fn mate_in_one_on_the_hundredth_halfmove_skips_jev() {
        let p = player(MockChooser::answering("x", vec![]));
        let result = p
            .choose_move(&game("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 99 80"))
            .unwrap();
        assert_eq!(result.san, "Re8#");
        assert_eq!(result.source, MoveSource::MateInOne);
        assert!(p.chooser.as_ref().unwrap().requests().is_empty());
    }

    #[test]
    fn no_key_falls_back_to_search() {
        let p: ComputerPlayer<MockChooser> = ComputerPlayer::new(None, EngineConfig::default());
        let result = p.choose_move(&Game::new()).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert!(result.note.unwrap().contains("JEV_API_KEY not set"));
    }

    #[test]
    fn plays_jevs_pick_and_reports_details() {
        let p = player(MockChooser::answering(
            "Kd2",
            vec![("Kd2", 0.6), ("Kf2", 0.3), ("Qd4", 0.1)],
        ));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.san, "Kd2");
        assert_eq!(result.source, MoveSource::Jev);
        assert_eq!(result.top[0], ("Kd2".to_string(), 0.6));
        assert_eq!(result.confidence, Some(0.8));
        assert_eq!(result.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(result.input_tokens, Some(900));
        assert_eq!(result.note, None);
    }

    #[test]
    fn shortlist_drops_losing_moves_and_keeps_search_order() {
        let mock = MockChooser::answering("Kd2", vec![("Kd2", 1.0)]);
        let p = player(mock);
        p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        let request = &p.chooser.as_ref().unwrap().requests()[0];
        assert!(
            request.options.iter().all(|o| o.key != "Qxd5"),
            "losing move filtered"
        );
        assert!(
            request
                .options
                .iter()
                .all(|o| o.assessment != Bucket::Losing)
        );
        assert_eq!(request.question, QUESTION);
        assert_eq!(request.state["side_to_move"], "White");

        // Keys follow the search order, best first, with the losing moves removed.
        let g = game(HANGING_QUEEN_TRAP);
        let expected: Vec<String> = annotate(g.position(), &analyse(&g))
            .into_iter()
            .filter(|a| a.bucket != Bucket::Losing)
            .map(|a| a.san)
            .take(EngineConfig::default().max_options)
            .collect();
        let keys: Vec<String> = request.options.iter().map(|o| o.key.clone()).collect();
        assert!(keys.len() > 4, "{keys:?}");
        assert_eq!(keys, expected);
    }

    #[test]
    fn top_is_capped_at_three_and_sorted() {
        let answer = vec![("Kd2", 0.4), ("Kf2", 0.3), ("Qd4", 0.2), ("Ke2", 0.1)];
        let p = player(MockChooser::answering("Kd2", answer));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        let request = &p.chooser.as_ref().unwrap().requests()[0];
        for key in ["Kd2", "Kf2", "Qd4", "Ke2"] {
            assert!(
                request.options.iter().any(|o| o.key == key),
                "{key} is on the shortlist"
            );
        }
        assert_eq!(result.top.len(), 3);
        assert!(result.top.windows(2).all(|w| w[0].1 >= w[1].1));
        assert_eq!(
            result.top,
            vec![
                ("Kd2".to_string(), 0.4),
                ("Kf2".to_string(), 0.3),
                ("Qd4".to_string(), 0.2)
            ]
        );
    }

    #[test]
    fn shortlist_keeps_losing_moves_when_filter_is_off_and_honours_the_cap() {
        let config = EngineConfig {
            filter_losing: false,
            max_options: 3,
            ..EngineConfig::default()
        };
        let p = ComputerPlayer::new(
            Some(MockChooser::answering("Kd2", vec![("Kd2", 1.0)])),
            config,
        );
        p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        let options = p.chooser.as_ref().unwrap().requests()[0].options.clone();
        assert_eq!(options.len(), 3);

        let config = EngineConfig {
            filter_losing: false,
            ..EngineConfig::default()
        };
        let p = ComputerPlayer::new(
            Some(MockChooser::answering("Kd2", vec![("Kd2", 1.0)])),
            config,
        );
        p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        let options = p.chooser.as_ref().unwrap().requests()[0].options.clone();
        assert!(options.iter().any(|o| o.key == "Qxd5"));
    }

    #[test]
    fn shortlist_keeps_everything_when_every_move_loses() {
        let annotation = |san: &str, score| Annotation {
            mv: Game::new().position().legal_moves()[0],
            san: san.to_string(),
            score,
            effect: "quiet move".to_string(),
            bucket: Bucket::Losing,
        };
        let all_losing = vec![
            annotation("a", -MATE + 2),
            annotation("b", -MATE + 2),
            annotation("c", -MATE + 4),
        ];
        assert_eq!(shortlist(&all_losing, true, 40).len(), 3);
        assert_eq!(shortlist(&all_losing, true, 2).len(), 2);
        assert_eq!(
            shortlist(&all_losing, true, 0).len(),
            1,
            "cap is at least 1"
        );
    }

    #[test]
    fn vetoes_a_losing_pick() {
        let config = EngineConfig {
            filter_losing: false,
            ..EngineConfig::default()
        };
        let mock = MockChooser::answering("Qxd5", vec![("Qxd5", 0.7), ("Kd2", 0.2), ("Qd4", 0.1)]);
        let p = ComputerPlayer::new(Some(mock), config);
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(
            result.source,
            MoveSource::Vetoed {
                jev_pick: "Qxd5".to_string()
            }
        );
        assert_eq!(
            result.san, "Kd2",
            "next most likely option within the margin"
        );
        assert!(result.note.unwrap().contains("Jev picked Qxd5"));
    }

    #[test]
    fn veto_with_no_acceptable_alternative_plays_the_search_best() {
        let config = EngineConfig {
            filter_losing: false,
            ..EngineConfig::default()
        };
        let mock = MockChooser::answering("Qxd5", vec![("Qxd5", 1.0)]);
        let p = ComputerPlayer::new(Some(mock), config);
        let g = game(HANGING_QUEEN_TRAP);
        let best = analyse(&g)[0].mv;
        let result = p.choose_move(&g).unwrap();
        assert_eq!(result.mv, best);
        assert!(matches!(result.source, MoveSource::Vetoed { .. }));
    }

    #[test]
    fn unknown_option_falls_back() {
        let p = player(MockChooser::answering("Qh8", vec![("Qh8", 1.0)]));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert!(result.note.unwrap().contains("unknown option (Qh8)"));
    }

    #[test]
    fn unknown_option_note_is_short_and_printable() {
        let choice = "Qh8\r\n\x1b[2Jzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let p = player(MockChooser::answering(choice, vec![(choice, 1.0)]));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        let note = result.note.unwrap();
        assert!(!note.chars().any(char::is_control), "{note:?}");
        assert_eq!(
            note,
            format!(
                "Jev returned an unknown option (Qh8   [2J{}…) — local search",
                "z".repeat(31)
            )
        );
    }

    #[test]
    fn move_source_labels() {
        assert_eq!(MoveSource::Jev.to_string(), "Jev");
        assert_eq!(MoveSource::OnlyMove.to_string(), "only move");
        assert_eq!(MoveSource::MateInOne.to_string(), "mate in one");
        assert_eq!(
            MoveSource::Vetoed {
                jev_pick: "Qxd5".to_string()
            }
            .to_string(),
            "vetoed (Jev picked Qxd5)"
        );
        assert_eq!(MoveSource::Fallback.to_string(), "local search");
    }

    #[test]
    fn debug_shows_the_config_without_the_key() {
        let config = EngineConfig {
            api_key: Some("secret-key-123".to_string()),
            ..EngineConfig::default()
        };
        let p = ComputerPlayer::new(Some(MockChooser::answering("x", vec![])), config.clone());
        let text = format!("{p:?}");
        assert!(!text.contains("secret-key-123"), "{text}");
        assert!(text.contains("<redacted>"), "{text}");
        assert!(text.contains("has_chooser: true"), "{text}");
        let p: ComputerPlayer<MockChooser> = ComputerPlayer::new(None, config);
        assert!(format!("{p:?}").contains("has_chooser: false"));
    }

    #[test]
    fn errors_fall_back_with_the_cause() {
        let p = player(MockChooser::failing(http_error(503, "overloaded", None)));
        let result = p.choose_move(&Game::new()).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert_eq!(result.mv, analyse(&Game::new())[0].mv);
        assert!(result.note.unwrap().contains("HTTP 503: overloaded"));
    }

    #[test]
    fn top_ignores_keys_outside_the_shortlist() {
        let p = player(MockChooser::answering(
            "Kd2",
            vec![("Nf3", 0.5), ("Kd2", 0.4), ("zz", 0.1)],
        ));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Jev);
        assert_eq!(result.top, vec![("Kd2".to_string(), 0.4)]);
    }

    #[test]
    fn trace_attaches_the_exchange_to_a_jev_move() {
        let p = traced_player(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Jev);
        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));
        assert_eq!(p.chooser.as_ref().unwrap().traced_calls(), 1);
    }

    #[test]
    fn trace_attaches_the_exchange_to_a_vetoed_move() {
        let config = EngineConfig {
            filter_losing: false,
            trace: true,
            ..EngineConfig::default()
        };
        let mock = MockChooser::answering("Qxd5", vec![("Qxd5", 0.7), ("Kd2", 0.3)]);
        let p = ComputerPlayer::new(Some(mock), config);
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert!(matches!(result.source, MoveSource::Vetoed { .. }));
        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));
    }

    #[test]
    fn trace_attaches_the_exchange_to_a_fallback_after_jev_was_asked() {
        let p = traced_player(MockChooser::failing(http_error(503, "overloaded", None)));
        let result = p.choose_move(&Game::new()).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert!(result.note.unwrap().contains("HTTP 503"));
        assert_eq!(result.exchange, Some(Box::new(mock_exchange())));

        let p = traced_player(MockChooser::answering("Qh8", vec![("Qh8", 1.0)]));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert!(result.note.unwrap().contains("unknown option"));
        assert_eq!(
            result.exchange,
            Some(Box::new(mock_exchange())),
            "Jev was asked, so an unusable answer is recorded too"
        );
    }

    #[test]
    fn no_exchange_without_trace() {
        let p = player(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Jev);
        assert_eq!(result.exchange, None);
        let mock = p.chooser.as_ref().unwrap();
        assert_eq!(mock.requests().len(), 1);
        assert_eq!(mock.traced_calls(), 0, "the untraced path calls `choose`");

        let p = player(MockChooser::failing(JevError::Timeout));
        assert_eq!(p.choose_move(&Game::new()).unwrap().exchange, None);
    }

    #[test]
    fn no_exchange_when_jev_is_not_asked() {
        for (fen, source) in [
            ("7k/8/8/8/8/8/6q1/7K w - - 0 1", MoveSource::OnlyMove),
            (
                "6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1",
                MoveSource::MateInOne,
            ),
        ] {
            let p = traced_player(MockChooser::answering("x", vec![]));
            let result = p.choose_move(&game(fen)).unwrap();
            assert_eq!(result.source, source);
            assert_eq!(result.exchange, None, "{fen}");
            assert!(p.chooser.as_ref().unwrap().requests().is_empty());
        }

        let config = EngineConfig {
            trace: true,
            ..EngineConfig::default()
        };
        let p: ComputerPlayer<MockChooser> = ComputerPlayer::new(None, config);
        let result = p.choose_move(&Game::new()).unwrap();
        assert_eq!(result.source, MoveSource::Fallback);
        assert_eq!(result.exchange, None, "no key, no request");
    }

    #[test]
    fn no_exchange_from_a_chooser_that_does_not_trace() {
        let config = EngineConfig {
            trace: true,
            ..EngineConfig::default()
        };
        let chooser = Untraced(MockChooser::answering("Kd2", vec![("Kd2", 1.0)]));
        let p = ComputerPlayer::new(Some(chooser), config);
        let result = p.choose_move(&game(HANGING_QUEEN_TRAP)).unwrap();
        assert_eq!(result.source, MoveSource::Jev);
        assert_eq!(result.exchange, None);
    }
}
