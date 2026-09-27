//! Static exchange evaluation (SEE): the material outcome of a series of captures
//! on one square when both sides always recapture with their least valuable piece
//! and may stop whenever continuing would lose material.
//!
//! A piece absolutely pinned to its own king takes part only when the target square
//! lies on the line through its king and itself. The first capturer is checked
//! against the pins of the position before it moves. The recaptures are checked
//! against the pins of the position after the first capture, so a pin that capture
//! releases (the pinning piece itself captures and leaves the line) or creates (the
//! capturer uncovers a line to the enemy king) is honoured. Pins created or released
//! by later captures in the exchange are not modelled. A pinned piece still guards
//! the square against a king: a king never captures onto a square an enemy piece
//! attacks, pinned or not.

use crate::core::{
    Bitboard, Color, Move, PieceKind, Position, Square, between, bishop_attacks, line, rook_attacks,
};

use super::eval::piece_value;

/// Stand-in value for a king taking part in an exchange: losing it ends the game.
const KING_VALUE: i32 = 20_000;

/// A piece's value in an exchange: its material value, or effectively infinite
/// for the king.
pub fn exchange_value(kind: PieceKind) -> i32 {
    if kind == PieceKind::King {
        KING_VALUE
    } else {
        piece_value(kind)
    }
}

/// Absolutely pinned pieces and the king squares they are pinned to, both indexed
/// by `Color::index`.
struct Pins {
    pinned: [Bitboard; 2],
    kings: [Square; 2],
}

/// One side's pieces as the pin scan sees them.
#[derive(Clone, Copy)]
struct Side {
    king: Square,
    occupied: Bitboard,
    /// Rooks and queens.
    orthogonal: Bitboard,
    /// Bishops and queens.
    diagonal: Bitboard,
}

impl Side {
    fn of(pos: &Position, color: Color) -> Side {
        let queens = pos.pieces_of(color, PieceKind::Queen);
        Side {
            king: pos.king_square(color),
            occupied: pos.occupied_by(color),
            orthogonal: pos.pieces_of(color, PieceKind::Rook) | queens,
            diagonal: pos.pieces_of(color, PieceKind::Bishop) | queens,
        }
    }
}

/// Pins of both colors in `pos` as it stands.
fn pins(pos: &Position) -> Pins {
    let sides = [Side::of(pos, Color::White), Side::of(pos, Color::Black)];
    pins_of(&sides, pos.occupied())
}

/// Pins of both colors in the position that results when the piece on `from`
/// captures on `to`, becoming `kind_after` (differs from its kind only for a
/// promotion). `ep_victim` is the square of a pawn taken en passant.
fn pins_after_capture(
    pos: &Position,
    from: Square,
    to: Square,
    kind_after: PieceKind,
    ep_victim: Option<Square>,
) -> Pins {
    let mover = pos.piece_at(from).expect("capture source holds a piece");
    let removed = to.bb() | ep_victim.map_or(Bitboard::EMPTY, Square::bb);
    let mut sides = [Side::of(pos, Color::White), Side::of(pos, Color::Black)];

    let theirs = &mut sides[(!mover.color).index()];
    theirs.occupied &= !removed;
    theirs.orthogonal &= !removed;
    theirs.diagonal &= !removed;

    let ours = &mut sides[mover.color.index()];
    ours.occupied = (ours.occupied & !from.bb()) | to.bb();
    ours.orthogonal &= !from.bb();
    ours.diagonal &= !from.bb();
    match kind_after {
        PieceKind::King => ours.king = to,
        PieceKind::Rook => ours.orthogonal |= to.bb(),
        PieceKind::Bishop => ours.diagonal |= to.bb(),
        PieceKind::Queen => {
            ours.orthogonal |= to.bb();
            ours.diagonal |= to.bb();
        }
        PieceKind::Pawn | PieceKind::Knight => {}
    }

    let occupied = sides[0].occupied | sides[1].occupied;
    pins_of(&sides, occupied)
}

fn pins_of(sides: &[Side; 2], occupied: Bitboard) -> Pins {
    Pins {
        pinned: [
            pinned(&sides[0], &sides[1], occupied),
            pinned(&sides[1], &sides[0], occupied),
        ],
        kings: [sides[0].king, sides[1].king],
    }
}

/// Pieces of `ours` that are the only piece between their king and an enemy
/// rook, bishop or queen on the same line.
fn pinned(ours: &Side, theirs: &Side, occupied: Bitboard) -> Bitboard {
    let king = ours.king;
    let snipers = (rook_attacks(king, theirs.occupied) & theirs.orthogonal)
        | (bishop_attacks(king, theirs.occupied) & theirs.diagonal);
    let mut pinned = Bitboard::EMPTY;
    for sniper in snipers {
        let blockers = between(king, sniper) & occupied;
        if blockers.count() == 1 && (blockers & ours.occupied).any() {
            pinned |= blockers;
        }
    }
    pinned
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
    let ep_victim = mv
        .is_en_passant()
        .then(|| Square::from_file_rank(to.file(), from.rank()).expect("on board"));
    let captured = if let Some(victim) = ep_victim {
        occupied ^= victim.bb();
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
    let pins = pins_after_capture(pos, from, to, mv.promotion().unwrap_or(mover), ep_victim);
    swap(
        pos,
        &pins,
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
    // The first capturer must be free to move in the position as it stands; the
    // recaptures are judged in the position after its capture.
    let Some((from, kind)) = next_capturer(pos, &pins(pos), sq, pos.occupied(), !target.color)
    else {
        return 0;
    };
    let pins = pins_after_capture(pos, from, sq, kind, None);
    let occupied = pos.occupied() ^ from.bb();
    let gain = swap(
        pos,
        &pins,
        sq,
        occupied,
        target.color,
        piece_value(target.kind),
        exchange_value(kind),
    );
    gain.max(0)
}

/// Pieces of `color` that can capture on `target` given `occupied`, excluding pieces
/// absolutely pinned to their king off the target's line.
pub fn capturers(pos: &Position, target: Square, occupied: Bitboard, color: Color) -> Bitboard {
    pin_aware_capturers(pos, &pins(pos), target, occupied, color)
}

fn pin_aware_capturers(
    pos: &Position,
    pins: &Pins,
    target: Square,
    occupied: Bitboard,
    color: Color,
) -> Bitboard {
    let mut attackers = pos.attackers_to(target, occupied) & occupied & pos.occupied_by(color);
    let pinned = attackers & pins.pinned[color.index()];
    if pinned.any() {
        let king = pins.kings[color.index()];
        for sq in pinned {
            if !line(king, sq).contains(target) {
                attackers ^= sq.bb();
            }
        }
    }
    attackers
}

/// The least valuable piece of `side` that can capture on `target` given
/// `occupied`, with its square. A pinned piece does not capture off its pin line,
/// but it still guards the square against a king: a king captures only when no
/// enemy piece, pinned or not, attacks `target` once the king has left its square.
fn next_capturer(
    pos: &Position,
    pins: &Pins,
    target: Square,
    occupied: Bitboard,
    side: Color,
) -> Option<(Square, PieceKind)> {
    let attackers = pin_aware_capturers(pos, pins, target, occupied, side);
    let (from, kind) = least_valuable(pos, attackers)?;
    if kind == PieceKind::King {
        let after = occupied ^ from.bb();
        let guards = pos.attackers_to(target, after) & after & pos.occupied_by(!side);
        if guards.any() {
            return None;
        }
    }
    Some((from, kind))
}

/// Pieces of `color` (not the king) the opponent can win by SEE, as
/// `(value, square, kind)`: most valuable first, ties in a1..h8 order.
pub fn winnable_pieces(pos: &Position, color: Color) -> Vec<(i32, Square, PieceKind)> {
    let mut pieces: Vec<(i32, Square, PieceKind)> = (pos.occupied_by(color)
        & !pos.pieces(PieceKind::King))
    .into_iter()
    .filter(|&sq| capture_gain(pos, sq) > 0)
    .filter_map(|sq| pos.piece_at(sq).map(|p| (piece_value(p.kind), sq, p.kind)))
    .collect();
    // A stable sort keeps the a1..h8 iteration order among equal values.
    pieces.sort_by_key(|&(value, _, _)| std::cmp::Reverse(value));
    pieces
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
    pins: &Pins,
    target: Square,
    mut occupied: Bitboard,
    mut side: Color,
    first_gain: i32,
    mut on_square: i32,
) -> i32 {
    let mut gain = [0i32; 32];
    gain[0] = first_gain;
    let mut depth = 0;
    // Removing pieces from `occupied` uncovers x-ray attackers behind them.
    while let Some((from, kind)) = next_capturer(pos, pins, target, occupied, side) {
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

    /// Nc6 is pinned to the king by Bb5, so it cannot recapture on e5.
    const PINNED_DEFENDER: &str =
        "r1bqkbnr/ppp2ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4";
    /// Ne5 is pinned to the king by Re1, so it cannot take the queen on c4.
    const PINNED_ATTACKER: &str = "4k3/8/8/4n3/2Q5/8/8/4RK2 w - - 0 1";

    #[test]
    fn pinned_defender_does_not_recapture() {
        assert_eq!(see_of(PINNED_DEFENDER, "f3e5"), 100);
    }

    #[test]
    fn pinned_attacker_cannot_win_a_piece() {
        assert_eq!(gain_on(PINNED_ATTACKER, "c4"), 0);
    }

    #[test]
    fn pinned_piece_captures_along_its_pin_line() {
        // The rook on e2 is pinned on the e-file by the rook on e7: it may take
        // the pinning rook but not the bishop on a2.
        let fen = "k7/4r3/8/8/8/8/b3R3/4K3 w - - 0 1";
        assert_eq!(see_of(fen, "e2e7"), 500);
        assert_eq!(gain_on(fen, "e7"), 500);
        assert_eq!(gain_on(fen, "a2"), 0);
        let pos = Position::from_fen(fen).unwrap();
        let e2: Square = "e2".parse().unwrap();
        let e7: Square = "e7".parse().unwrap();
        let a2: Square = "a2".parse().unwrap();
        assert_eq!(capturers(&pos, e7, pos.occupied(), Color::White), e2.bb());
        assert!(capturers(&pos, a2, pos.occupied(), Color::White).is_empty());
    }

    /// The queen on e1 pins the bishop on e5 to the king on e8.
    const PIN_RELEASED: &str = "4k3/8/8/4b3/8/2p5/8/4QK2 w - - 0 1";

    #[test]
    fn pin_released_by_the_first_capture() {
        // Qxc3 leaves the e-file, so the bishop is free to take back: 100 - 900.
        assert_eq!(see_of(PIN_RELEASED, "e1c3"), -800);
    }

    #[test]
    fn capture_gain_sees_the_released_pin() {
        let fen = PIN_RELEASED.replace(" w ", " b ");
        assert_eq!(gain_on(&fen, "c3"), 0);
    }

    #[test]
    fn pin_created_by_the_first_capture() {
        // Nxd5 opens the a1-h8 diagonal: the knight on f6 is now pinned to the
        // king on h8 by the bishop on a1 and cannot take back on d5.
        assert_eq!(see_of("7k/8/5n2/3p4/8/2N5/8/B3K3 w - - 0 1", "c3d5"), 100);
    }

    #[test]
    fn a_king_does_not_capture_onto_a_square_a_pinned_piece_guards() {
        // Re1 pins Be6 to the king on e8, but the bishop still guards d5: Kxd5 is illegal.
        assert_eq!(gain_on("4k3/8/4b3/3n4/2K5/8/8/4R3 b - - 0 1", "d5"), 0);
    }

    #[test]
    fn a_king_does_not_recapture_onto_a_square_a_pinned_piece_guards() {
        // Qe1 pins Be5 to the king on e8, but the bishop still guards d4: after
        // Rxd4 Rxd4 the king cannot take back, so White only trades rooks.
        assert_eq!(see_of("3rk3/8/8/4b3/3r4/2K5/8/3RQ3 w - - 0 1", "d1d4"), 0);
    }

    #[test]
    fn winnable_pieces_most_valuable_first() {
        // exd4 wins the rook for a pawn; the knights on b3 and f3 hang and tie on
        // value, so they keep a1..h8 order.
        let pos = Position::from_fen("4k3/8/8/4p3/2pR2p1/1N3N2/8/7K w - - 0 1").unwrap();
        let found: Vec<(i32, String)> = winnable_pieces(&pos, Color::White)
            .into_iter()
            .map(|(value, sq, _)| (value, sq.to_string()))
            .collect();
        assert_eq!(
            found,
            vec![
                (500, "d4".to_string()),
                (320, "b3".to_string()),
                (320, "f3".to_string())
            ]
        );
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
