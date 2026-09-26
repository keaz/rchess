//! The computer player (spec 5.6): forced moves are played directly, everything
//! else goes through a shortlist, one Jev `choice` question and a veto check.
//! It never fails: any problem degrades to the local search's best move.

use std::time::{Duration, Instant};

use crate::core::{Game, Move};

use super::annotate::{Annotation, Bucket, annotate};
use super::config::{EngineConfig, MAX_CHOICE_OPTIONS};
use super::describe::describe;
use super::jev::{ChoiceOption, ChoiceRequest, JevClient, MoveChooser};
use super::search::{MATE, analyse};

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
    Vetoed { jev_pick: String },
    /// Jev was unavailable or answered unusably; the search best was played.
    Fallback,
}

/// A chosen move plus everything the TUI shows about how it was chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputerMove {
    pub mv: Move,
    pub san: String,
    pub source: MoveSource,
    /// Up to three Jev options with probabilities, most likely first.
    pub top: Vec<(String, f32)>,
    pub confidence: Option<f32>,
    /// Versioned model ID that answered.
    pub model: Option<String>,
    /// Time for the whole `choose_move` call.
    pub latency: Duration,
    pub input_tokens: Option<u32>,
    /// Why a fallback or veto happened.
    pub note: Option<String>,
}

pub struct ComputerPlayer<C: MoveChooser> {
    chooser: Option<C>,
    config: EngineConfig,
}

impl ComputerPlayer<JevClient> {
    /// A player using the real Jev client when the config has a key.
    pub fn from_config(config: EngineConfig) -> ComputerPlayer<JevClient> {
        ComputerPlayer::new(JevClient::new(&config), config)
    }
}

impl<C: MoveChooser> ComputerPlayer<C> {
    pub fn new(chooser: Option<C>, config: EngineConfig) -> ComputerPlayer<C> {
        ComputerPlayer { chooser, config }
    }

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

        let answer = match chooser.choose(&request) {
            Ok(answer) => answer,
            Err(error) => {
                let note = format!("Jev unavailable ({error}) — local search");
                return Some(plain(best.mv, MoveSource::Fallback, Some(note)));
            }
        };
        let Some(pick) = candidates.iter().find(|a| a.san == answer.choice) else {
            let note = format!(
                "Jev returned an unknown option ({}) — local search",
                answer.choice
            );
            return Some(plain(best.mv, MoveSource::Fallback, Some(note)));
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
    use crate::engine::jev::{ChoiceAnswer, JevError, http_error};
    use std::sync::Mutex;

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
    }

    impl MockChooser {
        fn answering(choice: &'static str, probabilities: Vec<(&'static str, f32)>) -> MockChooser {
            MockChooser {
                reply: Reply::Answer {
                    choice,
                    probabilities,
                },
                seen: Mutex::new(Vec::new()),
            }
        }

        fn failing(error: JevError) -> MockChooser {
            MockChooser {
                reply: Reply::Error(error),
                seen: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<ChoiceRequest> {
            self.seen.lock().unwrap().clone()
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
    }

    fn player(mock: MockChooser) -> ComputerPlayer<MockChooser> {
        ComputerPlayer::new(Some(mock), EngineConfig::default())
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
}
