use std::fmt::Write;

use super::{ChessError, Color, Move, Position};

/// Why a game ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Checkmate { winner: Color },
    Resignation { winner: Color },
    Stalemate,
    FiftyMoveRule,
    ThreefoldRepetition,
    InsufficientMaterial,
}

impl Outcome {
    pub fn winner(self) -> Option<Color> {
        match self {
            Outcome::Checkmate { winner } | Outcome::Resignation { winner } => Some(winner),
            _ => None,
        }
    }

    /// PGN result token.
    pub fn result(self) -> &'static str {
        match self.winner() {
            Some(Color::White) => "1-0",
            Some(Color::Black) => "0-1",
            None => "1/2-1/2",
        }
    }
}

/// A game: starting position plus the moves played, with undo.
#[derive(Clone, Debug)]
pub struct Game {
    /// `positions[0]` is the start; `positions[i + 1]` follows `moves[i]`. Never empty.
    positions: Vec<Position>,
    moves: Vec<Move>,
    resigned: Option<Color>,
}

impl Default for Game {
    fn default() -> Game {
        Game::new()
    }
}

impl Game {
    pub fn new() -> Game {
        Game::from_position(Position::startpos())
    }

    pub fn from_fen(fen: &str) -> Result<Game, ChessError> {
        Ok(Game::from_position(Position::from_fen(fen)?))
    }

    pub fn from_position(start: Position) -> Game {
        Game {
            positions: vec![start],
            moves: Vec::new(),
            resigned: None,
        }
    }

    pub fn position(&self) -> &Position {
        self.positions.last().expect("positions is never empty")
    }

    pub fn start_position(&self) -> &Position {
        &self.positions[0]
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    /// Plays `mv` if it is legal and the game is not over.
    pub fn play(&mut self, mv: Move) -> Result<(), ChessError> {
        if let Some(outcome) = self.outcome() {
            return Err(ChessError::IllegalMove(format!(
                "game is over ({outcome:?})"
            )));
        }
        let pos = self.position();
        if !pos.legal_moves().contains(&mv) {
            return Err(ChessError::IllegalMove(format!("{mv} in {}", pos.to_fen())));
        }
        let next = pos.play(mv);
        self.positions.push(next);
        self.moves.push(mv);
        Ok(())
    }

    /// Takes back the most recent action. A pending resignation is withdrawn
    /// first (returning `None`, moves untouched); otherwise the last move is
    /// taken back and returned (`None` when no moves remain).
    pub fn undo(&mut self) -> Option<Move> {
        if self.resigned.take().is_some() {
            return None;
        }
        let mv = self.moves.pop()?;
        self.positions.pop();
        Some(mv)
    }

    pub fn resign(&mut self, loser: Color) {
        if self.outcome().is_none() {
            self.resigned = Some(loser);
        }
    }

    pub fn outcome(&self) -> Option<Outcome> {
        if let Some(loser) = self.resigned {
            return Some(Outcome::Resignation { winner: !loser });
        }
        let pos = self.position();
        if pos.legal_moves().is_empty() {
            return Some(if pos.is_check() {
                Outcome::Checkmate {
                    winner: !pos.side_to_move(),
                }
            } else {
                Outcome::Stalemate
            });
        }
        if pos.is_insufficient_material() {
            return Some(Outcome::InsufficientMaterial);
        }
        if pos.halfmove_clock() >= 100 {
            return Some(Outcome::FiftyMoveRule);
        }
        // Repetitions can only occur since the last irreversible move.
        let window = pos.halfmove_clock() as usize + 1;
        let repeats = self
            .positions
            .iter()
            .rev()
            .take(window)
            .filter(|p| p.hash() == pos.hash())
            .count();
        if repeats >= 3 {
            return Some(Outcome::ThreefoldRepetition);
        }
        None
    }

    /// PGN with minimal headers; includes `SetUp`/`FEN` when not from the start position.
    pub fn to_pgn(&self) -> String {
        let result = self.outcome().map_or("*", Outcome::result);
        let mut pgn = String::new();
        writeln!(pgn, "[Event \"rchess game\"]").unwrap();
        writeln!(pgn, "[Result \"{result}\"]").unwrap();
        if *self.start_position() != Position::startpos() {
            writeln!(pgn, "[SetUp \"1\"]").unwrap();
            writeln!(pgn, "[FEN \"{}\"]", self.start_position().to_fen()).unwrap();
        }
        pgn.push('\n');

        let mut tokens = Vec::new();
        for (i, (&mv, pos)) in self.moves.iter().zip(&self.positions).enumerate() {
            match pos.side_to_move() {
                Color::White => tokens.push(format!("{}.", pos.fullmove_number())),
                Color::Black if i == 0 => tokens.push(format!("{}...", pos.fullmove_number())),
                Color::Black => {}
            }
            tokens.push(pos.to_san(mv));
        }
        tokens.push(result.to_string());
        pgn.push_str(&tokens.join(" "));
        pgn.push('\n');
        pgn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_uci(game: &mut Game, moves: &[&str]) {
        for text in moves {
            let mv = game.position().parse_uci(text).unwrap();
            game.play(mv).unwrap();
        }
    }

    #[test]
    fn fools_mate() {
        let mut game = Game::new();
        play_uci(&mut game, &["f2f3", "e7e5", "g2g4", "d8h4"]);
        assert_eq!(
            game.outcome(),
            Some(Outcome::Checkmate {
                winner: Color::Black
            })
        );
        let e2e4 = Position::startpos().parse_uci("e2e4").unwrap();
        assert!(game.play(e2e4).is_err(), "no moves after the game ends");
    }

    #[test]
    fn undo_restores_position() {
        let mut game = Game::new();
        play_uci(&mut game, &["e2e4", "e7e5"]);
        assert_eq!(game.undo().map(|m| m.to_uci()), Some("e7e5".to_string()));
        assert_eq!(game.undo().map(|m| m.to_uci()), Some("e2e4".to_string()));
        assert_eq!(game.undo(), None);
        assert_eq!(*game.position(), Position::startpos());
    }

    #[test]
    fn rejects_illegal_move() {
        let mut game = Game::new();
        let black_move =
            Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1")
                .unwrap()
                .parse_uci("e7e5")
                .unwrap();
        assert!(matches!(
            game.play(black_move),
            Err(ChessError::IllegalMove(_))
        ));
    }

    #[test]
    fn stalemate() {
        let game = Game::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(game.outcome(), Some(Outcome::Stalemate));
    }

    #[test]
    fn threefold_repetition() {
        let mut game = Game::new();
        play_uci(
            &mut game,
            &["g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1"],
        );
        assert_eq!(game.outcome(), None);
        play_uci(&mut game, &["f6g8"]);
        assert_eq!(game.outcome(), Some(Outcome::ThreefoldRepetition));
    }

    #[test]
    fn fifty_move_rule() {
        let mut game = Game::from_fen("4k3/8/8/8/8/8/8/R3K3 w - - 99 80").unwrap();
        assert_eq!(game.outcome(), None);
        play_uci(&mut game, &["a1a2"]);
        assert_eq!(game.outcome(), Some(Outcome::FiftyMoveRule));
    }

    #[test]
    fn insufficient_material() {
        let game = Game::from_fen("4k3/8/8/8/8/8/8/4KB2 w - - 0 1").unwrap();
        assert_eq!(game.outcome(), Some(Outcome::InsufficientMaterial));
    }

    #[test]
    fn resignation_and_undo() {
        let mut game = Game::new();
        game.resign(Color::White);
        assert_eq!(
            game.outcome(),
            Some(Outcome::Resignation {
                winner: Color::Black
            })
        );
        game.undo();
        assert_eq!(game.outcome(), None);
    }

    #[test]
    fn undo_after_resignation_keeps_moves() {
        let mut game = Game::new();
        play_uci(&mut game, &["e2e4", "e7e5"]);
        game.resign(Color::White);
        assert_eq!(
            game.undo(),
            None,
            "first undo withdraws the resignation only"
        );
        assert_eq!(game.outcome(), None);
        assert_eq!(game.moves().len(), 2);
        assert_eq!(game.undo().map(|m| m.to_uci()), Some("e7e5".to_string()));
    }

    #[test]
    fn pgn_export() {
        let mut game = Game::new();
        play_uci(&mut game, &["f2f3", "e7e5", "g2g4", "d8h4"]);
        assert_eq!(
            game.to_pgn(),
            "[Event \"rchess game\"]\n[Result \"0-1\"]\n\n1. f3 e5 2. g4 Qh4# 0-1\n"
        );

        let mut game = Game::from_fen("4k3/8/8/8/8/8/4P3/4K3 b - - 0 12").unwrap();
        play_uci(&mut game, &["e8d7", "e2e4"]);
        assert!(
            game.to_pgn()
                .contains("[FEN \"4k3/8/8/8/8/8/4P3/4K3 b - - 0 12\"]")
        );
        assert!(game.to_pgn().ends_with("12... Kd7 13. e4 *\n"));
    }
}
