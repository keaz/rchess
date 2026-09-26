use std::{fmt, ops::Not};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    White,
    Black,
}

impl Color {
    pub const ALL: [Color; 2] = [Color::White, Color::Black];

    pub const fn index(self) -> usize {
        self as usize
    }
}

impl Not for Color {
    type Output = Color;

    fn not(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Color::White => "White",
            Color::Black => "Black",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PieceKind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl PieceKind {
    pub const ALL: [PieceKind; 6] = [
        PieceKind::Pawn,
        PieceKind::Knight,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::Queen,
        PieceKind::King,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Lowercase letter used by FEN (black pieces) and UCI promotions.
    pub const fn to_char(self) -> char {
        match self {
            PieceKind::Pawn => 'p',
            PieceKind::Knight => 'n',
            PieceKind::Bishop => 'b',
            PieceKind::Rook => 'r',
            PieceKind::Queen => 'q',
            PieceKind::King => 'k',
        }
    }

    /// Accepts either case.
    pub const fn from_char(c: char) -> Option<PieceKind> {
        match c.to_ascii_lowercase() {
            'p' => Some(PieceKind::Pawn),
            'n' => Some(PieceKind::Knight),
            'b' => Some(PieceKind::Bishop),
            'r' => Some(PieceKind::Rook),
            'q' => Some(PieceKind::Queen),
            'k' => Some(PieceKind::King),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub color: Color,
    pub kind: PieceKind,
}

impl Piece {
    pub const fn new(color: Color, kind: PieceKind) -> Piece {
        Piece { color, kind }
    }

    /// FEN letter: uppercase for White, lowercase for Black.
    pub const fn to_fen_char(self) -> char {
        let c = self.kind.to_char();
        match self.color {
            Color::White => c.to_ascii_uppercase(),
            Color::Black => c,
        }
    }

    pub const fn from_fen_char(c: char) -> Option<Piece> {
        let Some(kind) = PieceKind::from_char(c) else {
            return None;
        };
        let color = if c.is_ascii_uppercase() {
            Color::White
        } else {
            Color::Black
        };
        Some(Piece { color, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_not_flips() {
        assert_eq!(!Color::White, Color::Black);
        assert_eq!(!Color::Black, Color::White);
    }

    #[test]
    fn fen_char_round_trip() {
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                let piece = Piece::new(color, kind);
                assert_eq!(Piece::from_fen_char(piece.to_fen_char()), Some(piece));
            }
        }
        assert_eq!(
            Piece::from_fen_char('N'),
            Some(Piece::new(Color::White, PieceKind::Knight))
        );
        assert_eq!(Piece::from_fen_char('x'), None);
    }
}
