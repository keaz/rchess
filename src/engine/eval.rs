//! Static evaluation in centipawns: material plus the piece-square tables of
//! Tomasz Michniewski's "Simplified Evaluation Function".

use crate::core::{Color, PieceKind, Position, Square};

/// Material values in centipawns, indexed by `PieceKind::index()`. The king has none.
const PIECE_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 0];

/// Material value of one piece in centipawns (0 for the king).
pub fn piece_value(kind: PieceKind) -> i32 {
    PIECE_VALUES[kind.index()]
}

// Each table is written as White sees the board: the first row is rank 8, the
// last row is rank 1. `table_index` mirrors the lookup for Black.
#[rustfmt::skip]
const PAWN_TABLE: [i32; 64] = [
     0,  0,  0,  0,  0,  0,  0,  0,
    50, 50, 50, 50, 50, 50, 50, 50,
    10, 10, 20, 30, 30, 20, 10, 10,
     5,  5, 10, 25, 25, 10,  5,  5,
     0,  0,  0, 20, 20,  0,  0,  0,
     5, -5,-10,  0,  0,-10, -5,  5,
     5, 10, 10,-20,-20, 10, 10,  5,
     0,  0,  0,  0,  0,  0,  0,  0,
];

#[rustfmt::skip]
const KNIGHT_TABLE: [i32; 64] = [
    -50,-40,-30,-30,-30,-30,-40,-50,
    -40,-20,  0,  0,  0,  0,-20,-40,
    -30,  0, 10, 15, 15, 10,  0,-30,
    -30,  5, 15, 20, 20, 15,  5,-30,
    -30,  0, 15, 20, 20, 15,  0,-30,
    -30,  5, 10, 15, 15, 10,  5,-30,
    -40,-20,  0,  5,  5,  0,-20,-40,
    -50,-40,-30,-30,-30,-30,-40,-50,
];

#[rustfmt::skip]
const BISHOP_TABLE: [i32; 64] = [
    -20,-10,-10,-10,-10,-10,-10,-20,
    -10,  0,  0,  0,  0,  0,  0,-10,
    -10,  0,  5, 10, 10,  5,  0,-10,
    -10,  5,  5, 10, 10,  5,  5,-10,
    -10,  0, 10, 10, 10, 10,  0,-10,
    -10, 10, 10, 10, 10, 10, 10,-10,
    -10,  5,  0,  0,  0,  0,  5,-10,
    -20,-10,-10,-10,-10,-10,-10,-20,
];

#[rustfmt::skip]
const ROOK_TABLE: [i32; 64] = [
     0,  0,  0,  0,  0,  0,  0,  0,
     5, 10, 10, 10, 10, 10, 10,  5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
    -5,  0,  0,  0,  0,  0,  0, -5,
     0,  0,  0,  5,  5,  0,  0,  0,
];

#[rustfmt::skip]
const QUEEN_TABLE: [i32; 64] = [
    -20,-10,-10, -5, -5,-10,-10,-20,
    -10,  0,  0,  0,  0,  0,  0,-10,
    -10,  0,  5,  5,  5,  5,  0,-10,
     -5,  0,  5,  5,  5,  5,  0, -5,
      0,  0,  5,  5,  5,  5,  0, -5,
    -10,  5,  5,  5,  5,  5,  0,-10,
    -10,  0,  5,  0,  0,  0,  0,-10,
    -20,-10,-10, -5, -5,-10,-10,-20,
];

#[rustfmt::skip]
const KING_MIDDLEGAME_TABLE: [i32; 64] = [
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -30,-40,-40,-50,-50,-40,-40,-30,
    -20,-30,-30,-40,-40,-30,-30,-20,
    -10,-20,-20,-20,-20,-20,-20,-10,
     20, 20,  0,  0,  0,  0, 20, 20,
     20, 30, 10,  0,  0, 10, 30, 20,
];

#[rustfmt::skip]
const KING_ENDGAME_TABLE: [i32; 64] = [
    -50,-40,-30,-20,-20,-30,-40,-50,
    -30,-20,-10,  0,  0,-10,-20,-30,
    -30,-10, 20, 30, 30, 20,-10,-30,
    -30,-10, 30, 40, 40, 30,-10,-30,
    -30,-10, 30, 40, 40, 30,-10,-30,
    -30,-10, 20, 30, 30, 20,-10,-30,
    -30,-30,  0,  0,  0,  0,-30,-30,
    -50,-30,-30,-30,-30,-30,-30,-50,
];

fn table(kind: PieceKind, endgame: bool) -> &'static [i32; 64] {
    match kind {
        PieceKind::Pawn => &PAWN_TABLE,
        PieceKind::Knight => &KNIGHT_TABLE,
        PieceKind::Bishop => &BISHOP_TABLE,
        PieceKind::Rook => &ROOK_TABLE,
        PieceKind::Queen => &QUEEN_TABLE,
        PieceKind::King if endgame => &KING_ENDGAME_TABLE,
        PieceKind::King => &KING_MIDDLEGAME_TABLE,
    }
}

/// Index into a table for a piece of `color` on `sq`. Tables start at a8, so
/// White flips the rank and Black reads the square directly (a mirror image).
fn table_index(color: Color, sq: Square) -> usize {
    match color {
        Color::White => sq.index() ^ 56,
        Color::Black => sq.index(),
    }
}

/// True when kings should use the endgame table: both sides have no queens, or
/// every side that has a queen has no rook and at most one minor piece.
pub fn is_endgame(pos: &Position) -> bool {
    Color::ALL.into_iter().all(|color| {
        if pos.pieces_of(color, PieceKind::Queen).is_empty() {
            return true;
        }
        let rooks = pos.pieces_of(color, PieceKind::Rook).count();
        let minors = (pos.pieces_of(color, PieceKind::Knight)
            | pos.pieces_of(color, PieceKind::Bishop))
        .count();
        rooks == 0 && minors <= 1
    })
}

/// Evaluation in centipawns from the point of view of the side to move.
pub fn evaluate(pos: &Position) -> i32 {
    let endgame = is_endgame(pos);
    let mut white_minus_black = 0;
    for color in Color::ALL {
        let sign = if color == Color::White { 1 } else { -1 };
        for kind in PieceKind::ALL {
            let table = table(kind, endgame);
            for sq in pos.pieces_of(color, kind) {
                white_minus_black += sign * (piece_value(kind) + table[table_index(color, sq)]);
            }
        }
    }
    match pos.side_to_move() {
        Color::White => white_minus_black,
        Color::Black => -white_minus_black,
    }
}

/// Material only (no tables), White minus Black, in centipawns.
pub fn material_balance(pos: &Position) -> i32 {
    PieceKind::ALL
        .into_iter()
        .map(|kind| {
            let white = pos.pieces_of(Color::White, kind).count() as i32;
            let black = pos.pieces_of(Color::Black, kind).count() as i32;
            (white - black) * piece_value(kind)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(fen: &str) -> Position {
        Position::from_fen(fen).unwrap()
    }

    /// The same position with the board flipped top to bottom and the colours swapped.
    fn mirror(fen: &str) -> String {
        let fields: Vec<&str> = fen.split_whitespace().collect();
        let swap_case = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_ascii_uppercase() {
                        c.to_ascii_lowercase()
                    } else {
                        c.to_ascii_uppercase()
                    }
                })
                .collect()
        };
        let board: Vec<String> = fields[0].split('/').rev().map(swap_case).collect();
        let side = if fields[1] == "w" { "b" } else { "w" };
        let castling = if fields[2] == "-" {
            "-".to_string()
        } else {
            let swapped = swap_case(fields[2]);
            "KQkq".chars().filter(|c| swapped.contains(*c)).collect()
        };
        format!(
            "{} {side} {castling} - {} {}",
            board.join("/"),
            fields[4],
            fields[5]
        )
    }

    #[test]
    fn start_position_is_level() {
        assert_eq!(evaluate(&Position::startpos()), 0);
    }

    #[test]
    fn evaluation_is_colour_symmetric() {
        for fen in [
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "4k3/8/8/8/3b4/8/5N2/7K w - - 0 1",
        ] {
            assert_eq!(evaluate(&pos(fen)), evaluate(&pos(&mirror(fen))), "{fen}");
        }
    }

    #[test]
    fn piece_square_tables_reward_central_pawns() {
        let e2 = evaluate(&pos("4k3/8/8/8/8/8/4P3/4K3 w - - 0 1"));
        let e4 = evaluate(&pos("4k3/8/8/8/4P3/8/8/4K3 w - - 0 1"));
        assert!(e4 > e2, "e4 {e4} should beat e2 {e2}");
    }

    #[test]
    fn material_balance_counts_white_minus_black() {
        assert_eq!(material_balance(&Position::startpos()), 0);
        assert_eq!(
            material_balance(&pos("4k3/8/8/8/8/8/8/1N2K3 w - - 0 1")),
            320
        );
        assert_eq!(
            material_balance(&pos("3qk3/8/8/8/8/8/8/1N2K3 b - - 0 1")),
            -580
        );
    }

    #[test]
    fn endgame_rule() {
        assert!(!is_endgame(&Position::startpos()));
        assert!(is_endgame(&pos("4k3/8/8/8/8/8/8/R3K3 w - - 0 1")));
        assert!(is_endgame(&pos("r3k3/8/8/8/8/8/8/3QK3 w - - 0 1")));
        assert!(is_endgame(&pos("4k3/8/8/8/8/8/8/2BQK3 w - - 0 1")));
        assert!(!is_endgame(&pos("4k3/8/8/8/8/8/8/R2QK3 w - - 0 1")));
        assert!(!is_endgame(&pos("4k3/8/8/8/8/8/8/1NBQK3 w - - 0 1")));
    }
}
