use std::fmt;

use super::{PieceKind, Square};

// Flag layout (upper 4 bits): bit 2 = capture, bit 3 = promotion,
// low 2 bits = promotion piece (N, B, R, Q) or special-move code.
pub(crate) const QUIET: u16 = 0;
pub(crate) const DOUBLE_PUSH: u16 = 1;
pub(crate) const KING_CASTLE: u16 = 2;
pub(crate) const QUEEN_CASTLE: u16 = 3;
pub(crate) const CAPTURE: u16 = 4;
pub(crate) const EN_PASSANT: u16 = 5;
pub(crate) const PROMOTION: u16 = 8;

const PROMOTION_KINDS: [PieceKind; 4] = [
    PieceKind::Knight,
    PieceKind::Bishop,
    PieceKind::Rook,
    PieceKind::Queen,
];

/// A move packed into 16 bits: 6 bits from, 6 bits to, 4 bits flags.
///
/// Moves are only created by move generation, so a `Move` obtained from
/// `Position::legal_moves` is always legal in that position.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move(u16);

impl Move {
    pub(crate) const NULL: Move = Move(0);

    pub(crate) const fn new(from: Square, to: Square, flags: u16) -> Move {
        Move(from.index() as u16 | (to.index() as u16) << 6 | flags << 12)
    }

    /// Promotion move; `kind` must be knight, bishop, rook or queen.
    pub(crate) const fn promotion_move(
        from: Square,
        to: Square,
        kind: PieceKind,
        capture: bool,
    ) -> Move {
        let piece_bits = kind.index() as u16 - PieceKind::Knight.index() as u16;
        let capture_bit = if capture { CAPTURE } else { 0 };
        Move::new(from, to, PROMOTION | capture_bit | piece_bits)
    }

    pub const fn from(self) -> Square {
        Square::from_index_unchecked((self.0 & 63) as u8)
    }

    pub const fn to(self) -> Square {
        Square::from_index_unchecked((self.0 >> 6 & 63) as u8)
    }

    const fn flags(self) -> u16 {
        self.0 >> 12
    }

    /// True for normal captures, en passant and capturing promotions.
    pub const fn is_capture(self) -> bool {
        self.flags() & CAPTURE != 0
    }

    pub const fn is_en_passant(self) -> bool {
        self.flags() == EN_PASSANT
    }

    pub const fn is_double_push(self) -> bool {
        self.flags() == DOUBLE_PUSH
    }

    pub const fn is_castle(self) -> bool {
        matches!(self.flags(), KING_CASTLE | QUEEN_CASTLE)
    }

    pub const fn is_king_castle(self) -> bool {
        self.flags() == KING_CASTLE
    }

    pub const fn promotion(self) -> Option<PieceKind> {
        if self.flags() & PROMOTION == 0 {
            None
        } else {
            Some(PROMOTION_KINDS[(self.flags() & 3) as usize])
        }
    }

    /// UCI long algebraic notation, e.g. `e2e4`, `e7e8q`, `e1g1` (castling).
    pub fn to_uci(self) -> String {
        match self.promotion() {
            Some(kind) => format!("{}{}{}", self.from(), self.to(), kind.to_char()),
            None => format!("{}{}", self.from(), self.to()),
        }
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_uci())
    }
}

impl fmt::Debug for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Move({})", self.to_uci())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(s: &str) -> Square {
        s.parse().unwrap()
    }

    #[test]
    fn packs_and_unpacks() {
        let mv = Move::new(sq("e2"), sq("e4"), DOUBLE_PUSH);
        assert_eq!(mv.from(), sq("e2"));
        assert_eq!(mv.to(), sq("e4"));
        assert!(mv.is_double_push());
        assert!(!mv.is_capture());
        assert_eq!(mv.promotion(), None);
        assert_eq!(mv.to_uci(), "e2e4");
    }

    #[test]
    fn flags_are_distinct() {
        assert!(Move::new(sq("e5"), sq("d6"), EN_PASSANT).is_capture());
        assert!(Move::new(sq("e5"), sq("d6"), EN_PASSANT).is_en_passant());
        assert!(Move::new(sq("e1"), sq("g1"), KING_CASTLE).is_castle());
        assert!(Move::new(sq("e1"), sq("g1"), KING_CASTLE).is_king_castle());
        assert!(!Move::new(sq("e1"), sq("c1"), QUEEN_CASTLE).is_king_castle());
        assert!(!Move::new(sq("e1"), sq("c1"), QUEEN_CASTLE).is_capture());
    }

    #[test]
    fn promotions() {
        for kind in PROMOTION_KINDS {
            for capture in [false, true] {
                let mv = Move::promotion_move(sq("b7"), sq("a8"), kind, capture);
                assert_eq!(mv.promotion(), Some(kind));
                assert_eq!(mv.is_capture(), capture);
                assert!(!mv.is_castle());
            }
        }
        assert_eq!(
            Move::promotion_move(sq("e7"), sq("e8"), PieceKind::Queen, false).to_uci(),
            "e7e8q"
        );
    }
}
