//! Static exchange evaluation (SEE): the material outcome of a series of captures
//! on one square when both sides always recapture with their least valuable piece
//! and may stop whenever continuing would lose material.

use crate::core::{Bitboard, Color, Move, PieceKind, Position, Square};

use super::eval::piece_value;

/// Stand-in value for a king taking part in an exchange: losing it ends the game.
const KING_VALUE: i32 = 20_000;

fn exchange_value(kind: PieceKind) -> i32 {
    if kind == PieceKind::King {
        KING_VALUE
    } else {
        piece_value(kind)
    }
}

/// Material gain in centipawns for the side to move from the capture `mv`,
/// assuming best recaptures. Negative when the capture loses material.
/// `mv` must be a legal capture in `pos`.
pub fn see(pos: &Position, mv: Move) -> i32 {
    let (from, to) = (mv.from(), mv.to());
    let mover = pos
        .piece_at(from)
        .expect("capture source holds a piece")
        .kind;
    let mut occupied = pos.occupied() ^ from.bb();
    let captured = if mv.is_en_passant() {
        let captured_sq = Square::from_file_rank(to.file(), from.rank()).expect("on board");
        occupied ^= captured_sq.bb();
        piece_value(PieceKind::Pawn)
    } else {
        pos.piece_at(to).map_or(0, |piece| piece_value(piece.kind))
    };
    let (first_gain, on_square) = match mv.promotion() {
        Some(kind) => (
            captured + piece_value(kind) - piece_value(PieceKind::Pawn),
            exchange_value(kind),
        ),
        None => (captured, exchange_value(mover)),
    };
    swap(
        pos,
        to,
        occupied,
        !pos.side_to_move(),
        first_gain,
        on_square,
    )
}

/// Material the opponent of the piece on `sq` can win by capturing it, assuming
/// best recaptures. 0 when the piece is safe or the square is empty.
pub fn capture_gain(pos: &Position, sq: Square) -> i32 {
    let Some(target) = pos.piece_at(sq) else {
        return 0;
    };
    let attackers = pos.attackers_to(sq, pos.occupied()) & pos.occupied_by(!target.color);
    let Some((from, kind)) = least_valuable(pos, attackers) else {
        return 0;
    };
    let occupied = pos.occupied() ^ from.bb();
    let gain = swap(
        pos,
        sq,
        occupied,
        target.color,
        piece_value(target.kind),
        exchange_value(kind),
    );
    gain.max(0)
}

/// The least valuable piece in `attackers`, with its square.
pub fn least_valuable(pos: &Position, attackers: Bitboard) -> Option<(Square, PieceKind)> {
    PieceKind::ALL
        .into_iter()
        .find_map(|kind| (attackers & pos.pieces(kind)).lsb().map(|sq| (sq, kind)))
}

/// Finishes an exchange on `target` after a first capture worth `first_gain` that
/// left a piece worth `on_square` there. `side` recaptures next; `occupied` already
/// excludes every piece that has moved. Returns the first capturer's net gain.
fn swap(
    pos: &Position,
    target: Square,
    mut occupied: Bitboard,
    mut side: Color,
    first_gain: i32,
    mut on_square: i32,
) -> i32 {
    let mut gain = [0i32; 32];
    gain[0] = first_gain;
    let mut depth = 0;
    loop {
        // Removing pieces from `occupied` uncovers x-ray attackers behind them.
        let attackers = pos.attackers_to(target, occupied) & occupied & pos.occupied_by(side);
        let Some((from, kind)) = least_valuable(pos, attackers) else {
            break;
        };
        depth += 1;
        gain[depth] = on_square - gain[depth - 1];
        on_square = exchange_value(kind);
        occupied ^= from.bb();
        side = !side;
        if depth == gain.len() - 1 {
            break;
        }
    }
    // Walk back: each side may decline to continue a losing exchange.
    while depth > 0 {
        gain[depth - 1] = -(-gain[depth - 1]).max(gain[depth]);
        depth -= 1;
    }
    gain[0]
}

#[cfg(test)]
mod tests {
    use crate::core::Position;

    use super::*;

    fn see_of(fen: &str, uci: &str) -> i32 {
        let pos = Position::from_fen(fen).unwrap();
        see(&pos, pos.parse_uci(uci).unwrap())
    }

    fn gain_on(fen: &str, square: &str) -> i32 {
        let pos = Position::from_fen(fen).unwrap();
        capture_gain(&pos, square.parse().unwrap())
    }

    #[test]
    fn undefended_capture_wins_the_piece() {
        assert_eq!(see_of("4k3/8/8/3r4/8/8/3Q4/4K3 w - - 0 1", "d2d5"), 500);
    }

    #[test]
    fn pawn_takes_defended_knight() {
        assert_eq!(see_of("4k3/8/4p3/3n4/4P3/8/8/4K3 w - - 0 1", "e4d5"), 220);
    }

    #[test]
    fn knight_takes_defended_pawn_loses() {
        assert_eq!(see_of("4k3/8/4p3/3p4/8/2N5/8/4K3 w - - 0 1", "c3d5"), -220);
    }

    #[test]
    fn equal_trade_is_zero() {
        assert_eq!(see_of("4k3/8/4p3/3n4/8/2N5/8/4K3 w - - 0 1", "c3d5"), 0);
    }

    #[test]
    fn x_ray_recapture_counts() {
        // Rxe5 Rxe5 Rxe5: the rook on e1 recaptures through the vacated e2.
        assert_eq!(see_of("4k3/4r3/8/4p3/8/8/4R3/4R1K1 w - - 0 1", "e2e5"), 100);
    }

    #[test]
    fn en_passant_capture() {
        assert_eq!(see_of("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"), 100);
    }

    #[test]
    fn capture_gain_finds_hanging_pieces() {
        // The bishop wins the undefended knight.
        assert_eq!(gain_on("4k3/8/8/8/3b4/8/5N2/7K w - - 0 1", "f2"), 320);
        // Defended by the king: BxN KxB loses 10 for Black, so the knight is safe.
        assert_eq!(gain_on("4k3/8/8/8/3b4/8/5N2/4K3 w - - 0 1", "f2"), 0);
        // Empty square.
        assert_eq!(gain_on("4k3/8/8/8/3b4/8/5N2/4K3 w - - 0 1", "a4"), 0);
    }
}
