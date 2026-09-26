use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not};

use super::Square;

/// A set of squares, one bit per square (bit 0 = a1, bit 63 = h8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bitboard(pub u64);

impl Bitboard {
    pub const EMPTY: Bitboard = Bitboard(0);
    pub const FULL: Bitboard = Bitboard(!0);
    pub const RANK_1: Bitboard = Bitboard(0xFF);
    pub const RANK_8: Bitboard = Bitboard(0xFF << 56);
    /// a1 is a dark square.
    pub const DARK_SQUARES: Bitboard = Bitboard(0xAA55_AA55_AA55_AA55);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn any(self) -> bool {
        self.0 != 0
    }

    pub const fn contains(self, sq: Square) -> bool {
        self.0 & sq.bb().0 != 0
    }

    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    pub const fn more_than_one(self) -> bool {
        self.0 & self.0.wrapping_sub(1) != 0
    }

    /// Lowest square in the set.
    pub const fn lsb(self) -> Option<Square> {
        if self.0 == 0 {
            None
        } else {
            Some(Square::from_index_unchecked(self.0.trailing_zeros() as u8))
        }
    }
}

impl From<Square> for Bitboard {
    fn from(sq: Square) -> Bitboard {
        sq.bb()
    }
}

macro_rules! bit_ops {
    ($($trait:ident $method:ident $assign_trait:ident $assign_method:ident $op:tt;)*) => {$(
        impl $trait for Bitboard {
            type Output = Bitboard;
            fn $method(self, rhs: Bitboard) -> Bitboard {
                Bitboard(self.0 $op rhs.0)
            }
        }
        impl $assign_trait for Bitboard {
            fn $assign_method(&mut self, rhs: Bitboard) {
                self.0 = self.0 $op rhs.0;
            }
        }
    )*};
}

bit_ops! {
    BitAnd bitand BitAndAssign bitand_assign &;
    BitOr bitor BitOrAssign bitor_assign |;
    BitXor bitxor BitXorAssign bitxor_assign ^;
}

impl Not for Bitboard {
    type Output = Bitboard;
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

/// Yields squares from lowest to highest index.
pub struct BitboardIter(u64);

impl Iterator for BitboardIter {
    type Item = Square;

    fn next(&mut self) -> Option<Square> {
        if self.0 == 0 {
            return None;
        }
        let sq = Square::from_index_unchecked(self.0.trailing_zeros() as u8);
        self.0 &= self.0 - 1;
        Some(sq)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.0.count_ones() as usize;
        (n, Some(n))
    }
}

impl IntoIterator for Bitboard {
    type Item = Square;
    type IntoIter = BitboardIter;

    fn into_iter(self) -> BitboardIter {
        BitboardIter(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(s: &str) -> Square {
        s.parse().unwrap()
    }

    #[test]
    fn set_operations() {
        let a = sq("a1").bb() | sq("h8").bb();
        assert!(a.contains(sq("a1")));
        assert!(!a.contains(sq("b1")));
        assert_eq!(a.count(), 2);
        assert!(a.more_than_one());
        assert!(!sq("a1").bb().more_than_one());
        assert_eq!((a & sq("h8").bb()).lsb(), Some(sq("h8")));
        assert_eq!(Bitboard::EMPTY.lsb(), None);
        assert_eq!(!Bitboard::EMPTY, Bitboard::FULL);
    }

    #[test]
    fn iterates_low_to_high() {
        let b = sq("c3").bb() | sq("a1").bb() | sq("h8").bb();
        let squares: Vec<Square> = b.into_iter().collect();
        assert_eq!(squares, vec![sq("a1"), sq("c3"), sq("h8")]);
    }

    #[test]
    fn dark_squares() {
        assert!(Bitboard::DARK_SQUARES.contains(sq("a1")));
        assert!(Bitboard::DARK_SQUARES.contains(sq("h8")));
        assert!(!Bitboard::DARK_SQUARES.contains(sq("h1")));
    }
}
