use std::{fmt, str::FromStr};

use super::{Bitboard, ChessError};

/// A board square. Index 0 is a1, 7 is h1, 56 is a8, 63 is h8.
///
/// The inner index is private, so every `Square` is always on the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Square(u8);

impl Square {
    pub const A1: Square = Square(0);
    pub const B1: Square = Square(1);
    pub const C1: Square = Square(2);
    pub const D1: Square = Square(3);
    pub const E1: Square = Square(4);
    pub const F1: Square = Square(5);
    pub const G1: Square = Square(6);
    pub const H1: Square = Square(7);
    pub const A8: Square = Square(56);
    pub const B8: Square = Square(57);
    pub const C8: Square = Square(58);
    pub const D8: Square = Square(59);
    pub const E8: Square = Square(60);
    pub const F8: Square = Square(61);
    pub const G8: Square = Square(62);
    pub const H8: Square = Square(63);

    pub const fn new(index: u8) -> Option<Square> {
        if index < 64 {
            Some(Square(index))
        } else {
            None
        }
    }

    /// `file` 0 = a .. 7 = h, `rank` 0 = rank 1 .. 7 = rank 8.
    pub const fn from_file_rank(file: u8, rank: u8) -> Option<Square> {
        if file < 8 && rank < 8 {
            Some(Square(rank * 8 + file))
        } else {
            None
        }
    }

    /// Caller guarantees `index < 64`.
    pub(crate) const fn from_index_unchecked(index: u8) -> Square {
        debug_assert!(index < 64);
        Square(index)
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub const fn file(self) -> u8 {
        self.0 % 8
    }

    pub const fn rank(self) -> u8 {
        self.0 / 8
    }

    pub const fn bb(self) -> Bitboard {
        Bitboard(1 << self.0)
    }

    /// Same file, mirrored rank: a1 <-> a8.
    pub const fn flip_rank(self) -> Square {
        Square(self.0 ^ 56)
    }

    /// Caller guarantees the result stays on the board.
    pub(crate) const fn offset(self, delta: i8) -> Square {
        Square::from_index_unchecked((self.0 as i8 + delta) as u8)
    }

    pub fn all() -> impl Iterator<Item = Square> {
        (0..64).map(Square)
    }
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", (b'a' + self.file()) as char, self.rank() + 1)
    }
}

impl FromStr for Square {
    type Err = ChessError;

    fn from_str(s: &str) -> Result<Square, ChessError> {
        match s.as_bytes() {
            [f @ b'a'..=b'h', r @ b'1'..=b'8'] => Ok(Square((r - b'1') * 8 + (f - b'a'))),
            _ => Err(ChessError::ParseMove(format!("bad square {s:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_layout() {
        assert_eq!(Square::A1.index(), 0);
        assert_eq!(Square::H1.index(), 7);
        assert_eq!(Square::A8.index(), 56);
        assert_eq!(Square::H8.index(), 63);
        assert_eq!(Square::new(64), None);
        assert_eq!(Square::from_file_rank(4, 3), "e4".parse().ok());
        assert_eq!(Square::from_file_rank(8, 0), None);
    }

    #[test]
    fn parse_and_display_round_trip() {
        for sq in Square::all() {
            assert_eq!(sq.to_string().parse::<Square>(), Ok(sq));
        }
        assert!("i1".parse::<Square>().is_err());
        assert!("a9".parse::<Square>().is_err());
        assert!("a".parse::<Square>().is_err());
        assert!("e44".parse::<Square>().is_err());
    }

    #[test]
    fn flip_rank_mirrors() {
        assert_eq!(Square::E1.flip_rank(), Square::E8);
        assert_eq!(
            "c3".parse::<Square>().unwrap().flip_rank().to_string(),
            "c6"
        );
    }
}
