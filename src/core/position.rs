use super::{
    Bitboard, ChessError, Color, Move, Piece, PieceKind, Square,
    attacks::{bishop_attacks, king_attacks, knight_attacks, pawn_attacks, rook_attacks},
    zobrist::KEYS,
};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// Castling availability as four flag bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CastleRights(u8);

impl CastleRights {
    pub const NONE: CastleRights = CastleRights(0);
    pub const WHITE_KING: CastleRights = CastleRights(1);
    pub const WHITE_QUEEN: CastleRights = CastleRights(2);
    pub const BLACK_KING: CastleRights = CastleRights(4);
    pub const BLACK_QUEEN: CastleRights = CastleRights(8);
    pub const ALL: CastleRights = CastleRights(15);

    pub const fn has(self, right: CastleRights) -> bool {
        self.0 & right.0 == right.0
    }

    pub const fn king_side(color: Color) -> CastleRights {
        match color {
            Color::White => CastleRights::WHITE_KING,
            Color::Black => CastleRights::BLACK_KING,
        }
    }

    pub const fn queen_side(color: Color) -> CastleRights {
        match color {
            Color::White => CastleRights::WHITE_QUEEN,
            Color::Black => CastleRights::BLACK_QUEEN,
        }
    }

    const fn index(self) -> usize {
        self.0 as usize
    }
}

/// For each square, the castling rights that survive a move from or to it.
const CASTLE_KEEP: [u8; 64] = {
    let mut keep = [15u8; 64];
    keep[Square::A1.index()] = 15 & !CastleRights::WHITE_QUEEN.0;
    keep[Square::H1.index()] = 15 & !CastleRights::WHITE_KING.0;
    keep[Square::E1.index()] = 15 & !(CastleRights::WHITE_KING.0 | CastleRights::WHITE_QUEEN.0);
    keep[Square::A8.index()] = 15 & !CastleRights::BLACK_QUEEN.0;
    keep[Square::H8.index()] = 15 & !CastleRights::BLACK_KING.0;
    keep[Square::E8.index()] = 15 & !(CastleRights::BLACK_KING.0 | CastleRights::BLACK_QUEEN.0);
    keep
};

/// A complete chess position. `Copy`, so making a move returns a new value and
/// undo is simply keeping the old one.
///
/// Invariant: `ep` is only set when a pawn of the side to move attacks that
/// square, so equal positions always hash equal regardless of how they arose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pieces: [Bitboard; 6],
    colors: [Bitboard; 2],
    side: Color,
    castling: CastleRights,
    ep: Option<Square>,
    halfmove: u16,
    fullmove: u16,
    hash: u64,
}

impl Default for Position {
    fn default() -> Position {
        Position::startpos()
    }
}

impl Position {
    pub fn startpos() -> Position {
        Position::from_fen(START_FEN).expect("START_FEN is valid")
    }

    pub fn side_to_move(&self) -> Color {
        self.side
    }

    pub fn castling(&self) -> CastleRights {
        self.castling
    }

    pub fn ep_square(&self) -> Option<Square> {
        self.ep
    }

    pub fn halfmove_clock(&self) -> u16 {
        self.halfmove
    }

    pub fn fullmove_number(&self) -> u16 {
        self.fullmove
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

    /// Pieces of `kind` of both colors.
    pub fn pieces(&self, kind: PieceKind) -> Bitboard {
        self.pieces[kind.index()]
    }

    pub fn pieces_of(&self, color: Color, kind: PieceKind) -> Bitboard {
        self.pieces[kind.index()] & self.colors[color.index()]
    }

    pub fn occupied_by(&self, color: Color) -> Bitboard {
        self.colors[color.index()]
    }

    pub fn occupied(&self) -> Bitboard {
        self.colors[0] | self.colors[1]
    }

    pub fn piece_at(&self, sq: Square) -> Option<Piece> {
        let color = if self.colors[0].contains(sq) {
            Color::White
        } else if self.colors[1].contains(sq) {
            Color::Black
        } else {
            return None;
        };
        let kind = self.kind_at(sq)?;
        Some(Piece::new(color, kind))
    }

    fn kind_at(&self, sq: Square) -> Option<PieceKind> {
        PieceKind::ALL
            .into_iter()
            .find(|k| self.pieces[k.index()].contains(sq))
    }

    pub fn king_square(&self, color: Color) -> Square {
        self.pieces_of(color, PieceKind::King)
            .lsb()
            .expect("each side always has a king")
    }

    /// Pieces of either color attacking `sq`, given occupancy `occupied`.
    pub fn attackers_to(&self, sq: Square, occupied: Bitboard) -> Bitboard {
        let bishops_queens = self.pieces(PieceKind::Bishop) | self.pieces(PieceKind::Queen);
        let rooks_queens = self.pieces(PieceKind::Rook) | self.pieces(PieceKind::Queen);
        (pawn_attacks(Color::Black, sq) & self.pieces_of(Color::White, PieceKind::Pawn))
            | (pawn_attacks(Color::White, sq) & self.pieces_of(Color::Black, PieceKind::Pawn))
            | (knight_attacks(sq) & self.pieces(PieceKind::Knight))
            | (king_attacks(sq) & self.pieces(PieceKind::King))
            | (bishop_attacks(sq, occupied) & bishops_queens)
            | (rook_attacks(sq, occupied) & rooks_queens)
    }

    /// Enemy pieces giving check to the side to move.
    pub fn checkers(&self) -> Bitboard {
        let king = self.king_square(self.side);
        self.attackers_to(king, self.occupied()) & self.occupied_by(!self.side)
    }

    pub fn is_check(&self) -> bool {
        self.checkers().any()
    }

    /// Neither side can ever checkmate: bare kings, one minor piece, or only
    /// bishops that all stand on the same square color.
    pub fn is_insufficient_material(&self) -> bool {
        let heavy = self.pieces(PieceKind::Pawn)
            | self.pieces(PieceKind::Rook)
            | self.pieces(PieceKind::Queen);
        if heavy.any() {
            return false;
        }
        let knights = self.pieces(PieceKind::Knight);
        let bishops = self.pieces(PieceKind::Bishop);
        if (knights | bishops).count() <= 1 {
            return true;
        }
        knights.is_empty()
            && ((bishops & Bitboard::DARK_SQUARES).is_empty()
                || (bishops & !Bitboard::DARK_SQUARES).is_empty())
    }

    /// Plays `mv`, which must come from `self.legal_moves()`.
    pub fn play(&self, mv: Move) -> Position {
        let mut next = *self;
        next.apply(mv);
        next
    }

    fn apply(&mut self, mv: Move) {
        let us = self.side;
        let them = !us;
        let (from, to) = (mv.from(), mv.to());
        let kind = self.kind_at(from).expect("move source holds a piece");

        if let Some(ep) = self.ep.take() {
            self.hash ^= KEYS.ep_file[ep.file() as usize];
        }
        self.halfmove += 1;

        if mv.is_en_passant() {
            let captured = Square::from_file_rank(to.file(), from.rank()).expect("on board");
            self.toggle(them, PieceKind::Pawn, captured);
        } else if mv.is_capture() {
            let captured = self.kind_at(to).expect("capture target holds a piece");
            self.toggle(them, captured, to);
        }
        if mv.is_capture() || kind == PieceKind::Pawn {
            self.halfmove = 0;
        }

        self.toggle(us, kind, from);
        self.toggle(us, mv.promotion().unwrap_or(kind), to);

        if mv.is_castle() {
            let (rook_from, rook_to) = if mv.is_king_castle() {
                (Square::H1, Square::F1)
            } else {
                (Square::A1, Square::D1)
            };
            let (rook_from, rook_to) = match us {
                Color::White => (rook_from, rook_to),
                Color::Black => (rook_from.flip_rank(), rook_to.flip_rank()),
            };
            self.toggle(us, PieceKind::Rook, rook_from);
            self.toggle(us, PieceKind::Rook, rook_to);
        }

        self.hash ^= KEYS.castling[self.castling.index()];
        self.castling =
            CastleRights(self.castling.0 & CASTLE_KEEP[from.index()] & CASTLE_KEEP[to.index()]);
        self.hash ^= KEYS.castling[self.castling.index()];

        if mv.is_double_push() {
            let ep = Square::from_file_rank(from.file(), (from.rank() + to.rank()) / 2)
                .expect("on board");
            if (pawn_attacks(us, ep) & self.pieces_of(them, PieceKind::Pawn)).any() {
                self.ep = Some(ep);
                self.hash ^= KEYS.ep_file[ep.file() as usize];
            }
        }

        if us == Color::Black {
            self.fullmove += 1;
        }
        self.side = them;
        self.hash ^= KEYS.side;
    }

    /// Adds or removes one piece, keeping the hash in sync.
    fn toggle(&mut self, color: Color, kind: PieceKind, sq: Square) {
        self.pieces[kind.index()] ^= sq.bb();
        self.colors[color.index()] ^= sq.bb();
        self.hash ^= KEYS.pieces[color.index()][kind.index()][sq.index()];
    }

    fn compute_hash(&self) -> u64 {
        let mut hash = 0;
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                for sq in self.pieces_of(color, kind) {
                    hash ^= KEYS.pieces[color.index()][kind.index()][sq.index()];
                }
            }
        }
        if self.side == Color::Black {
            hash ^= KEYS.side;
        }
        hash ^= KEYS.castling[self.castling.index()];
        if let Some(ep) = self.ep {
            hash ^= KEYS.ep_file[ep.file() as usize];
        }
        hash
    }

    pub fn from_fen(fen: &str) -> Result<Position, ChessError> {
        let err = |msg: &str| ChessError::InvalidFen(format!("{msg} in {fen:?}"));
        let fields: Vec<&str> = fen.split_whitespace().collect();
        if !(4..=6).contains(&fields.len()) {
            return Err(err("expected 4 to 6 fields"));
        }

        let mut pos = Position {
            pieces: [Bitboard::EMPTY; 6],
            colors: [Bitboard::EMPTY; 2],
            side: Color::White,
            castling: CastleRights::NONE,
            ep: None,
            halfmove: 0,
            fullmove: 1,
            hash: 0,
        };

        let ranks: Vec<&str> = fields[0].split('/').collect();
        if ranks.len() != 8 {
            return Err(err("board needs 8 ranks"));
        }
        for (i, rank_text) in ranks.iter().enumerate() {
            let rank = 7 - i as u8;
            let mut file = 0u8;
            for c in rank_text.chars() {
                if let Some(skip) = c.to_digit(10).filter(|d| (1..=8).contains(d)) {
                    file += skip as u8;
                } else {
                    let piece = Piece::from_fen_char(c).ok_or_else(|| err("bad piece letter"))?;
                    let sq =
                        Square::from_file_rank(file, rank).ok_or_else(|| err("rank too long"))?;
                    pos.pieces[piece.kind.index()] |= sq.bb();
                    pos.colors[piece.color.index()] |= sq.bb();
                    file += 1;
                }
                if file > 8 {
                    return Err(err("rank too long"));
                }
            }
            if file != 8 {
                return Err(err("rank too short"));
            }
        }

        pos.side = match fields[1] {
            "w" => Color::White,
            "b" => Color::Black,
            _ => return Err(err("side must be w or b")),
        };

        if fields[2] != "-" {
            for c in fields[2].chars() {
                pos.castling.0 |= match c {
                    'K' => CastleRights::WHITE_KING.0,
                    'Q' => CastleRights::WHITE_QUEEN.0,
                    'k' => CastleRights::BLACK_KING.0,
                    'q' => CastleRights::BLACK_QUEEN.0,
                    _ => return Err(err("bad castling field")),
                };
            }
        }

        if fields[3] != "-" {
            let ep: Square = fields[3]
                .parse()
                .map_err(|_| err("bad en passant square"))?;
            let expected_rank = if pos.side == Color::White { 5 } else { 2 };
            if ep.rank() != expected_rank {
                return Err(err("en passant square on wrong rank"));
            }
            // Normalize: keep it only when the side to move has a pawn that could capture.
            if (pawn_attacks(!pos.side, ep) & pos.pieces_of(pos.side, PieceKind::Pawn)).any() {
                pos.ep = Some(ep);
            }
        }

        if let Some(text) = fields.get(4) {
            pos.halfmove = text.parse().map_err(|_| err("bad halfmove clock"))?;
        }
        if let Some(text) = fields.get(5) {
            pos.fullmove = text.parse().map_err(|_| err("bad fullmove number"))?;
        }

        pos.validate().map_err(err)?;
        pos.hash = pos.compute_hash();
        Ok(pos)
    }

    fn validate(&self) -> Result<(), &'static str> {
        for color in Color::ALL {
            if self.pieces_of(color, PieceKind::King).count() != 1 {
                return Err("each side needs exactly one king");
            }
        }
        if (self.pieces(PieceKind::Pawn) & (Bitboard::RANK_1 | Bitboard::RANK_8)).any() {
            return Err("pawn on first or last rank");
        }
        let rights = [
            (CastleRights::WHITE_KING, Color::White, Square::H1),
            (CastleRights::WHITE_QUEEN, Color::White, Square::A1),
            (CastleRights::BLACK_KING, Color::Black, Square::H8),
            (CastleRights::BLACK_QUEEN, Color::Black, Square::A8),
        ];
        for (right, color, rook_sq) in rights {
            let king_sq = if color == Color::White {
                Square::E1
            } else {
                Square::E8
            };
            if self.castling.has(right)
                && !(self.pieces_of(color, PieceKind::King).contains(king_sq)
                    && self.pieces_of(color, PieceKind::Rook).contains(rook_sq))
            {
                return Err("castling right without king and rook on home squares");
            }
        }
        let their_king = self.king_square(!self.side);
        if (self.attackers_to(their_king, self.occupied()) & self.occupied_by(self.side)).any() {
            return Err("side not to move is in check");
        }
        Ok(())
    }

    pub fn to_fen(&self) -> String {
        let mut fen = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                let sq = Square::from_file_rank(file, rank).expect("on board");
                match self.piece_at(sq) {
                    Some(piece) => {
                        if empty > 0 {
                            fen.push_str(&empty.to_string());
                            empty = 0;
                        }
                        fen.push(piece.to_fen_char());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                fen.push_str(&empty.to_string());
            }
            if rank > 0 {
                fen.push('/');
            }
        }
        fen.push_str(if self.side == Color::White {
            " w "
        } else {
            " b "
        });
        if self.castling == CastleRights::NONE {
            fen.push('-');
        } else {
            for (right, c) in [
                (CastleRights::WHITE_KING, 'K'),
                (CastleRights::WHITE_QUEEN, 'Q'),
                (CastleRights::BLACK_KING, 'k'),
                (CastleRights::BLACK_QUEEN, 'q'),
            ] {
                if self.castling.has(right) {
                    fen.push(c);
                }
            }
        }
        match self.ep {
            Some(sq) => fen.push_str(&format!(" {sq}")),
            None => fen.push_str(" -"),
        }
        fen.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        fen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(s: &str) -> Square {
        s.parse().unwrap()
    }

    #[test]
    fn startpos_layout() {
        let pos = Position::startpos();
        assert_eq!(pos.side_to_move(), Color::White);
        assert_eq!(pos.castling(), CastleRights::ALL);
        assert_eq!(
            pos.piece_at(sq("e1")),
            Some(Piece::new(Color::White, PieceKind::King))
        );
        assert_eq!(
            pos.piece_at(sq("d8")),
            Some(Piece::new(Color::Black, PieceKind::Queen))
        );
        assert_eq!(pos.piece_at(sq("e4")), None);
        assert_eq!(pos.occupied().count(), 32);
        assert!(!pos.is_check());
    }

    #[test]
    fn fen_round_trip() {
        for fen in [
            START_FEN,
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        ] {
            assert_eq!(Position::from_fen(fen).unwrap().to_fen(), fen);
        }
    }

    #[test]
    fn fen_drops_uncapturable_en_passant() {
        let pos = Position::from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
            .unwrap();
        assert_eq!(pos.ep_square(), None);
        assert!(pos.to_fen().contains(" b KQkq - "));
    }

    #[test]
    fn fen_accepts_missing_clocks() {
        let pos = Position::from_fen("4k3/8/8/8/8/8/8/4K3 w - -").unwrap();
        assert_eq!(pos.halfmove_clock(), 0);
        assert_eq!(pos.fullmove_number(), 1);
    }

    #[test]
    fn fen_rejects_bad_input() {
        for fen in [
            "",
            "8/8/8/8/8/8/8/8 w - - 0 1",      // no kings
            "4k3/8/8/8/8/8/8/4K3 x - - 0 1",  // bad side
            "4k3/8/8/8/8/8/8/4K2 w - - 0 1",  // short rank
            "4k3/8/8/8/8/8/8/4K4 w - - 0 1",  // long rank
            "4k3/8/8/8/8/8/8/4K3 w K - 0 1",  // castling right without rook
            "4k3/8/8/8/8/8/8/P3K3 w - - 0 1", // pawn on rank 1
            "4k3/8/8/8/8/8/8/4RK2 w - - 0 1", // Black in check with White to move
        ] {
            assert!(Position::from_fen(fen).is_err(), "{fen}");
        }
    }

    #[test]
    fn detects_check() {
        let pos = Position::from_fen("4k3/8/8/8/8/8/8/4RK2 b - - 0 1").unwrap();
        assert!(pos.is_check());
        assert_eq!(pos.checkers(), sq("e1").bb());
    }

    #[test]
    fn insufficient_material() {
        let cases = [
            ("4k3/8/8/8/8/8/8/4K3 w - - 0 1", true),
            ("4k3/8/8/8/8/8/8/4KN2 w - - 0 1", true),
            ("4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1", false), // bishops on both colors
            ("4k3/8/8/8/8/8/8/2B1K1B1 w - - 0 1", true), // c1 and g1 are both dark
            ("4k3/8/8/8/8/8/8/4KNN1 w - - 0 1", false),
            ("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1", false),
        ];
        for (fen, expected) in cases {
            assert_eq!(
                Position::from_fen(fen).unwrap().is_insufficient_material(),
                expected,
                "{fen}"
            );
        }
    }
}
