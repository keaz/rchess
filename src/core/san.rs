//! Standard Algebraic Notation (SAN) and UCI move text.

use super::{ChessError, Move, PieceKind, Position, Square};

impl Position {
    /// SAN for a legal move, including `+` / `#` suffixes.
    ///
    /// `mv` must come from `self.legal_moves()`. Debug builds assert this;
    /// release builds may corrupt the position. Use `Game::play` for
    /// unvalidated moves.
    pub fn to_san(&self, mv: Move) -> String {
        let mut san = self.san_body(mv);
        let next = self.play(mv);
        if next.is_check() {
            san.push(if next.legal_moves().is_empty() {
                '#'
            } else {
                '+'
            });
        }
        san
    }

    /// SAN without check suffix.
    fn san_body(&self, mv: Move) -> String {
        if mv.is_castle() {
            return if mv.is_king_castle() { "O-O" } else { "O-O-O" }.to_string();
        }
        let piece = self.piece_at(mv.from()).expect("move source holds a piece");
        let mut san = String::new();

        if piece.kind == PieceKind::Pawn {
            if mv.is_capture() {
                san.push((b'a' + mv.from().file()) as char);
                san.push('x');
            }
            san.push_str(&mv.to().to_string());
            if let Some(kind) = mv.promotion() {
                san.push('=');
                san.push(kind.to_char().to_ascii_uppercase());
            }
            return san;
        }

        san.push(piece.kind.to_char().to_ascii_uppercase());
        let rivals: Vec<Square> = self
            .legal_moves()
            .iter()
            .filter(|m| {
                m.to() == mv.to() && m.from() != mv.from() && self.piece_at(m.from()) == Some(piece)
            })
            .map(|m| m.from())
            .collect();
        if !rivals.is_empty() {
            let file_char = (b'a' + mv.from().file()) as char;
            let rank_char = (b'1' + mv.from().rank()) as char;
            if rivals.iter().all(|r| r.file() != mv.from().file()) {
                san.push(file_char);
            } else if rivals.iter().all(|r| r.rank() != mv.from().rank()) {
                san.push(rank_char);
            } else {
                san.push(file_char);
                san.push(rank_char);
            }
        }
        if mv.is_capture() {
            san.push('x');
        }
        san.push_str(&mv.to().to_string());
        san
    }

    /// Parses SAN such as `e4`, `Nbd7`, `exd6`, `e8=Q`, `O-O`, `Qh4#`.
    /// Accepts `0-0` for castling and ignores trailing `+`, `#`, `!`, `?`.
    pub fn parse_san(&self, text: &str) -> Result<Move, ChessError> {
        let cleaned = text
            .trim()
            .trim_end_matches(['+', '#', '!', '?'])
            .replace('0', "O");
        if cleaned.is_empty() {
            return Err(ChessError::ParseMove(format!("empty move {text:?}")));
        }
        let legal = self.legal_moves();
        let mut matches = legal
            .iter()
            .copied()
            .filter(|&mv| self.san_body(mv) == cleaned);
        match (matches.next(), matches.next()) {
            (Some(mv), None) => Ok(mv),
            _ => Err(ChessError::IllegalMove(format!(
                "{text:?} in {}",
                self.to_fen()
            ))),
        }
    }

    /// Parses UCI long algebraic text such as `e2e4` or `e7e8q`.
    pub fn parse_uci(&self, text: &str) -> Result<Move, ChessError> {
        let text = text.trim();
        let parse_err = || ChessError::ParseMove(format!("bad UCI move {text:?}"));
        if !(4..=5).contains(&text.len()) || !text.is_ascii() {
            return Err(parse_err());
        }
        let from: Square = text[0..2].parse().map_err(|_| parse_err())?;
        let to: Square = text[2..4].parse().map_err(|_| parse_err())?;
        let promotion = match text[4..].chars().next() {
            None => None,
            Some(c) => match PieceKind::from_char(c) {
                Some(
                    k
                    @ (PieceKind::Knight | PieceKind::Bishop | PieceKind::Rook | PieceKind::Queen),
                ) => Some(k),
                _ => return Err(parse_err()),
            },
        };
        self.legal_moves()
            .iter()
            .copied()
            .find(|mv| mv.from() == from && mv.to() == to && mv.promotion() == promotion)
            .ok_or_else(|| ChessError::IllegalMove(format!("{text} in {}", self.to_fen())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn san_of(fen: &str, uci: &str) -> String {
        let pos = Position::from_fen(fen).unwrap();
        pos.to_san(pos.parse_uci(uci).unwrap())
    }

    #[test]
    fn basic_san() {
        let start = crate::core::START_FEN;
        assert_eq!(san_of(start, "e2e4"), "e4");
        assert_eq!(san_of(start, "g1f3"), "Nf3");
    }

    #[test]
    fn captures_promotions_castles_checks() {
        assert_eq!(san_of("7k/8/8/3p4/4P3/8/8/K7 w - - 0 1", "e4d5"), "exd5");
        assert_eq!(san_of("7k/P7/8/8/8/8/8/K7 w - - 0 1", "a7a8q"), "a8=Q+");
        assert_eq!(san_of("r3k3/8/8/8/8/8/8/4K2R w K - 0 1", "e1g1"), "O-O");
        assert_eq!(
            san_of(
                "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2",
                "d8h4"
            ),
            "Qh4#"
        );
    }

    #[test]
    fn disambiguation() {
        // Knights on b1 and f1 can both reach d2: file letter disambiguates.
        assert_eq!(san_of("7k/8/8/8/8/8/8/1N1K1N2 w - - 0 1", "b1d2"), "Nbd2");
        // Rooks on a1 and a5 can both reach a3: rank digit disambiguates.
        assert_eq!(san_of("7k/8/8/R7/8/8/8/R3K3 w - - 0 1", "a1a3"), "R1a3");
        // Queens on a1, a3 and c1 all reach b2: a1 needs both file and rank.
        assert_eq!(san_of("8/7k/8/8/8/Q7/8/Q1Q1K3 w - - 0 1", "a1b2"), "Qa1b2");
        assert_eq!(san_of("8/7k/8/8/8/Q7/8/Q1Q1K3 w - - 0 1", "a3b2"), "Q3b2");
        assert_eq!(san_of("8/7k/8/8/8/Q7/8/Q1Q1K3 w - - 0 1", "c1b2"), "Qcb2");
    }

    #[test]
    fn parse_san_round_trips_all_moves() {
        let pos = Position::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        for &mv in pos.legal_moves().iter() {
            assert_eq!(pos.parse_san(&pos.to_san(mv)), Ok(mv));
        }
        assert_eq!(
            pos.parse_san("0-0").map(|m| m.to_uci()),
            Ok("e1g1".to_string())
        );
        assert!(pos.parse_san("Ke3").is_err());
        assert!(pos.parse_san("").is_err());
    }

    #[test]
    fn parse_uci_rejects() {
        let pos = Position::startpos();
        assert!(matches!(
            pos.parse_uci("e2e5"),
            Err(ChessError::IllegalMove(_))
        ));
        assert!(matches!(pos.parse_uci("e2"), Err(ChessError::ParseMove(_))));
        assert!(matches!(
            pos.parse_uci("e7e8k"),
            Err(ChessError::ParseMove(_))
        ));
        assert!(matches!(
            pos.parse_uci("é2e4"),
            Err(ChessError::ParseMove(_))
        ));
    }
}
