//! Fully legal move generation. Checkers and pins are computed once per
//! position, so no generated move ever needs a make-and-test step (except en
//! passant, which is rare enough to verify by simulating the occupancy).

use std::ops::{Deref, DerefMut};

use super::{
    Bitboard, Color, Move, PieceKind, Position, Square,
    attacks::{
        between, bishop_attacks, king_attacks, knight_attacks, line, pawn_attacks, rook_attacks,
    },
    mv::{CAPTURE, DOUBLE_PUSH, EN_PASSANT, KING_CASTLE, QUEEN_CASTLE, QUIET},
    position::CastleRights,
};

/// 218 is the published maximum number of legal moves in any *reachable*
/// position (see `known_maximum_of_218_moves_fits` below). `CAPACITY` is
/// larger: it is the proven bound for every position `Position::from_fen`
/// accepts and every position reached from one by `Position::play`, since
/// promotions and captures never increase the promotion-material budget sum
/// (see `Position::validate`).
///
/// Derivation: 1 queen 27 + 2 rooks 28 + 2 bishops 26 + 2 knights 16 + 8
/// budget units x 27 (extra queens outmove pawns, which have at most 12)
/// + king 8 = 321.
const CAPACITY: usize = 321;

/// Fixed-capacity move buffer; nothing is heap-allocated.
#[derive(Clone)]
pub struct MoveList {
    moves: [Move; CAPACITY],
    len: usize,
}

impl MoveList {
    fn new() -> MoveList {
        MoveList {
            moves: [Move::NULL; CAPACITY],
            len: 0,
        }
    }

    fn push(&mut self, mv: Move) {
        self.moves[self.len] = mv;
        self.len += 1;
    }
}

impl Deref for MoveList {
    type Target = [Move];

    fn deref(&self) -> &[Move] {
        &self.moves[..self.len]
    }
}

/// So callers can sort the list for move ordering, e.g. `list.sort_by_key(...)`.
impl DerefMut for MoveList {
    fn deref_mut(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for MoveList {
    type Item = Move;
    type IntoIter = std::iter::Take<std::array::IntoIter<Move, CAPACITY>>;

    fn into_iter(self) -> Self::IntoIter {
        self.moves.into_iter().take(self.len)
    }
}

impl std::fmt::Debug for MoveList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl Position {
    pub fn legal_moves(&self) -> MoveList {
        let mut list = MoveList::new();
        let us = self.side_to_move();
        let ours = self.occupied_by(us);
        let theirs = self.occupied_by(!us);
        let occupied = ours | theirs;
        let king = self.king_square(us);
        let checkers = self.checkers();

        // King: may not step onto an attacked square. The king is removed from the
        // occupancy so it cannot shelter behind itself on a slider's ray.
        let without_king = occupied ^ king.bb();
        for to in king_attacks(king) & !ours {
            if (self.attackers_to(to, without_king) & theirs).is_empty() {
                list.push(Move::new(king, to, capture_flag(theirs, to)));
            }
        }
        if checkers.more_than_one() {
            return list;
        }

        // Every other move must capture the single checker or block its ray.
        let target = match checkers.lsb() {
            Some(checker) => checkers | between(king, checker),
            None => Bitboard::FULL,
        };
        let pinned = self.pinned(us, king);
        let pin_ray = |from: Square| {
            if pinned.contains(from) {
                line(king, from)
            } else {
                Bitboard::FULL
            }
        };

        for from in self.pieces_of(us, PieceKind::Knight) & !pinned {
            push_all(
                &mut list,
                from,
                knight_attacks(from) & !ours & target,
                theirs,
            );
        }
        let queens = self.pieces_of(us, PieceKind::Queen);
        for from in self.pieces_of(us, PieceKind::Bishop) | queens {
            let to = bishop_attacks(from, occupied) & !ours & target & pin_ray(from);
            push_all(&mut list, from, to, theirs);
        }
        for from in self.pieces_of(us, PieceKind::Rook) | queens {
            let to = rook_attacks(from, occupied) & !ours & target & pin_ray(from);
            push_all(&mut list, from, to, theirs);
        }

        self.pawn_moves(&mut list, target, pin_ray);

        if checkers.is_empty() {
            self.castles(&mut list);
        }
        list
    }

    /// Our pieces that are the only blocker between our king and an enemy slider.
    fn pinned(&self, us: Color, king: Square) -> Bitboard {
        let them = !us;
        let theirs = self.occupied_by(them);
        let queens = self.pieces_of(them, PieceKind::Queen);
        let snipers = (rook_attacks(king, theirs)
            & (self.pieces_of(them, PieceKind::Rook) | queens))
            | (bishop_attacks(king, theirs) & (self.pieces_of(them, PieceKind::Bishop) | queens));
        let mut pinned = Bitboard::EMPTY;
        for sniper in snipers {
            let blockers = between(king, sniper) & self.occupied();
            if blockers.count() == 1 && (blockers & self.occupied_by(us)).any() {
                pinned |= blockers;
            }
        }
        pinned
    }

    fn pawn_moves<F: Fn(Square) -> Bitboard>(
        &self,
        list: &mut MoveList,
        target: Bitboard,
        pin_ray: F,
    ) {
        let us = self.side_to_move();
        let theirs = self.occupied_by(!us);
        let occupied = self.occupied();
        let king = self.king_square(us);
        let (push, start_rank) = match us {
            Color::White => (8i8, 1u8),
            Color::Black => (-8i8, 6u8),
        };

        for from in self.pieces_of(us, PieceKind::Pawn) {
            let allowed = target & pin_ray(from);

            // Pawns never stand on the last rank, so one step forward is on the board.
            let one = from.offset(push);
            if !occupied.contains(one) {
                if allowed.contains(one) {
                    push_pawn(list, from, one, false);
                }
                if from.rank() == start_rank {
                    let two = one.offset(push);
                    if !occupied.contains(two) && allowed.contains(two) {
                        list.push(Move::new(from, two, DOUBLE_PUSH));
                    }
                }
            }

            for to in pawn_attacks(us, from) & theirs & allowed {
                push_pawn(list, from, to, true);
            }

            if let Some(ep) = self.ep_square()
                && pawn_attacks(us, from).contains(ep)
            {
                // Simulate the capture: covers pins, the rank-pin through both
                // pawns, and capturing a checking pawn.
                let captured = ep.offset(-push);
                let after = occupied ^ from.bb() ^ captured.bb() ^ ep.bb();
                let attackers = self.attackers_to(king, after) & theirs & !captured.bb();
                if attackers.is_empty() {
                    list.push(Move::new(from, ep, EN_PASSANT));
                }
            }
        }
    }

    fn castles(&self, list: &mut MoveList) {
        let us = self.side_to_move();
        let theirs = self.occupied_by(!us);
        let occupied = self.occupied();
        let relative = |sq: Square| {
            if us == Color::White {
                sq
            } else {
                sq.flip_rank()
            }
        };
        let safe = |sq: Square| (self.attackers_to(sq, occupied) & theirs).is_empty();
        let king = relative(Square::E1);

        if self.castling().has(CastleRights::king_side(us)) {
            let (f, g) = (relative(Square::F1), relative(Square::G1));
            if (occupied & (f.bb() | g.bb())).is_empty() && safe(f) && safe(g) {
                list.push(Move::new(king, g, KING_CASTLE));
            }
        }
        if self.castling().has(CastleRights::queen_side(us)) {
            let (b, c, d) = (
                relative(Square::B1),
                relative(Square::C1),
                relative(Square::D1),
            );
            if (occupied & (b.bb() | c.bb() | d.bb())).is_empty() && safe(d) && safe(c) {
                list.push(Move::new(king, c, QUEEN_CASTLE));
            }
        }
    }
}

fn capture_flag(theirs: Bitboard, to: Square) -> u16 {
    if theirs.contains(to) { CAPTURE } else { QUIET }
}

fn push_all(list: &mut MoveList, from: Square, targets: Bitboard, theirs: Bitboard) {
    for to in targets {
        list.push(Move::new(from, to, capture_flag(theirs, to)));
    }
}

fn push_pawn(list: &mut MoveList, from: Square, to: Square, capture: bool) {
    if to.rank() == 0 || to.rank() == 7 {
        for kind in [
            PieceKind::Queen,
            PieceKind::Rook,
            PieceKind::Bishop,
            PieceKind::Knight,
        ] {
            list.push(Move::promotion_move(from, to, kind, capture));
        }
    } else {
        list.push(Move::new(from, to, if capture { CAPTURE } else { QUIET }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uci_moves(fen: &str) -> Vec<String> {
        let mut moves: Vec<String> = Position::from_fen(fen)
            .unwrap()
            .legal_moves()
            .iter()
            .map(|m| m.to_uci())
            .collect();
        moves.sort();
        moves
    }

    #[test]
    fn startpos_has_twenty_moves() {
        assert_eq!(Position::startpos().legal_moves().len(), 20);
    }

    #[test]
    fn move_list_supports_iteration_and_sorting() {
        let mut list = Position::startpos().legal_moves();
        let len = list.len();

        let mut count = 0;
        for _mv in &list {
            count += 1;
        }
        assert_eq!(count, len);

        list.sort_by_key(|m| m.to_uci());
        assert_eq!(list.len(), len);
        assert!(list.windows(2).all(|w| w[0].to_uci() <= w[1].to_uci()));

        let owned_count = list.into_iter().count();
        assert_eq!(owned_count, len);
    }

    #[test]
    fn double_check_allows_only_king_moves() {
        // Rook on e8 and bishop on b4 both check the king on e1; the knight cannot help.
        let moves = uci_moves("4r2k/8/8/8/1b6/8/8/4K1N1 w - - 0 1");
        assert!(moves.iter().all(|m| m.starts_with("e1")), "{moves:?}");
    }

    #[test]
    fn pinned_piece_moves_along_pin_only() {
        // Rook on e2 pinned by rook on e8: may move along the e-file only.
        let moves = uci_moves("4r2k/8/8/8/8/8/4R3/4K3 w - - 0 1");
        let rook: Vec<&String> = moves.iter().filter(|m| m.starts_with("e2")).collect();
        assert_eq!(rook.len(), 6); // e3..e7 and capture on e8
        assert!(rook.iter().all(|m| m.as_bytes()[2] == b'e'));
    }

    #[test]
    fn en_passant_rank_pin_is_illegal() {
        // Capturing exd6 would remove both pawns from rank 5 and expose the king to the rook.
        let moves = uci_moves("8/8/8/K2pP2r/8/8/8/7k w - d6 0 1");
        assert!(!moves.contains(&"e5d6".to_string()), "{moves:?}");
    }

    #[test]
    fn en_passant_can_capture_checking_pawn() {
        // Black pawn d5 (just double-pushed) checks the king on e4; exd6 removes it.
        let moves = uci_moves("7k/8/8/3pP3/4K3/8/8/8 w - d6 0 1");
        assert!(moves.contains(&"e5d6".to_string()), "{moves:?}");
    }

    #[test]
    fn castling_blocked_by_attacked_path() {
        // Black rook on f8 attacks f1: king-side castling illegal, queen-side fine.
        let moves = uci_moves("5rk1/8/8/8/8/8/8/R3K2R w KQ - 0 1");
        assert!(!moves.contains(&"e1g1".to_string()));
        assert!(moves.contains(&"e1c1".to_string()));
    }

    #[test]
    fn castling_allowed_when_only_b_file_attacked() {
        // b1 attacked does not matter for queen-side castling; only c1/d1/e1 must be safe.
        let moves = uci_moves("1r5k/8/8/8/8/8/8/R3K3 w Q - 0 1");
        assert!(moves.contains(&"e1c1".to_string()));
    }

    #[test]
    fn promotions_generate_four_moves() {
        let moves = uci_moves("7k/P7/8/8/8/8/8/K7 w - - 0 1");
        for promo in ["a7a8q", "a7a8r", "a7a8b", "a7a8n"] {
            assert!(moves.contains(&promo.to_string()));
        }
    }

    #[test]
    fn known_maximum_of_218_moves_fits() {
        // The published maximum for a reachable position.
        let pos =
            Position::from_fen("R6R/3Q4/1Q4Q1/4Q3/2Q4Q/Q4Q2/pp1Q4/kBNN1KB1 w - - 0 1").unwrap();
        assert_eq!(pos.legal_moves().len(), 218);
    }

    #[test]
    fn checkmate_and_stalemate_have_no_moves() {
        // Fool's mate.
        let mate =
            Position::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .unwrap();
        assert!(mate.legal_moves().is_empty() && mate.is_check());
        let stalemate = Position::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert!(stalemate.legal_moves().is_empty() && !stalemate.is_check());
    }
}
