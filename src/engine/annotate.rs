//! Plain-language facts about each root move, plus a bucket derived from its
//! search score. Jev sees these words, never the numbers.

use serde::Serialize;

use crate::core::{
    Bitboard, Color, Move, PieceKind, Position, Square, bishop_attacks, king_attacks,
    knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};

use super::eval::piece_value;
use super::search::{MATE_BOUND, ScoredMove};
use super::see::{capture_gain, see};

/// The engine's verdict on a move relative to the best move found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Winning,
    Good,
    Neutral,
    Bad,
    Losing,
}

/// One root move with its SAN, search score, effect words and bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub mv: Move,
    pub san: String,
    pub score: i32,
    pub effect: String,
    pub bucket: Bucket,
}

/// Annotates every scored move, keeping the order of `scored` (best first).
pub fn annotate(pos: &Position, scored: &[ScoredMove]) -> Vec<Annotation> {
    let Some(best) = scored.first().map(|s| s.score) else {
        return Vec::new();
    };
    scored
        .iter()
        .map(|s| Annotation {
            mv: s.mv,
            san: pos.to_san(s.mv),
            score: s.score,
            effect: effect(pos, s.mv, s.score),
            bucket: bucket(best, s.score),
        })
        .collect()
}

/// Bucket from `d = best - score` (spec 5.4).
pub fn bucket(best: i32, score: i32) -> Bucket {
    let d = best - score;
    if score <= -MATE_BOUND || d > 300 {
        Bucket::Losing
    } else if d > 150 {
        Bucket::Bad
    } else if d > 50 {
        Bucket::Neutral
    } else if score >= 300 {
        Bucket::Winning
    } else {
        Bucket::Good
    }
}

/// Lowercase English name of a piece kind.
pub fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
    }
}

fn effect(pos: &Position, mv: Move, score: i32) -> String {
    let us = pos.side_to_move();
    let child = pos.play(mv);
    let mut facts: Vec<String> = Vec::new();

    if mv.is_castle() {
        let side = if mv.is_king_castle() {
            "kingside"
        } else {
            "queenside"
        };
        facts.push(format!("castles {side}"));
    }
    if mv.is_en_passant() {
        facts.push("captures en passant".to_string());
    } else if mv.is_capture() {
        let victim = pos
            .piece_at(mv.to())
            .expect("capture target holds a piece")
            .kind;
        let recapture =
            (child.attackers_to(mv.to(), child.occupied()) & child.occupied_by(!us)).any();
        let exchange = see(pos, mv);
        let qualifier = if !recapture {
            "undefended"
        } else if exchange >= 50 {
            "wins material in the exchange"
        } else if exchange > -50 {
            "equal trade"
        } else {
            "loses material in the exchange"
        };
        facts.push(format!(
            "captures the {} on {}, {qualifier}",
            piece_name(victim),
            mv.to()
        ));
    }
    if let Some(kind) = mv.promotion() {
        facts.push(format!("promotes to a {}", piece_name(kind)));
    }
    let checkmate = child.is_check() && child.legal_moves().is_empty();
    if checkmate {
        facts.push("delivers checkmate".to_string());
    } else if child.is_check() {
        facts.push("gives check".to_string());
    }
    if let Some(fact) = new_attack(pos, &child, mv) {
        facts.push(fact);
    }
    if let Some(fact) = rescue(pos, &child, mv) {
        facts.push(fact);
    }
    if !checkmate && let Some(fact) = hanging(&child, us) {
        facts.push(fact);
    }
    if score <= -MATE_BOUND {
        facts.push("allows checkmate".to_string());
    }

    if facts.is_empty() {
        "quiet move".to_string()
    } else {
        facts.join("; ")
    }
}

fn attacks(kind: PieceKind, color: Color, sq: Square, occupied: Bitboard) -> Bitboard {
    match kind {
        PieceKind::Pawn => pawn_attacks(color, sq),
        PieceKind::Knight => knight_attacks(sq),
        PieceKind::Bishop => bishop_attacks(sq, occupied),
        PieceKind::Rook => rook_attacks(sq, occupied),
        PieceKind::Queen => queen_attacks(sq, occupied),
        PieceKind::King => king_attacks(sq),
    }
}

/// The most valuable enemy piece (not the king) the moved piece attacks from its new
/// square but did not attack before, when it is worth more than the mover or undefended.
fn new_attack(pos: &Position, child: &Position, mv: Move) -> Option<String> {
    let us = pos.side_to_move();
    let before_kind = pos.piece_at(mv.from())?.kind;
    let mover = child.piece_at(mv.to())?.kind;
    let before = attacks(before_kind, us, mv.from(), pos.occupied());
    let after = attacks(mover, us, mv.to(), child.occupied());
    let enemies = child.occupied_by(!us) & !child.pieces(PieceKind::King);
    let mut best: Option<(i32, Square, PieceKind)> = None;
    for sq in after & !before & enemies {
        let kind = child.piece_at(sq)?.kind;
        let defended = (child.attackers_to(sq, child.occupied()) & child.occupied_by(!us)).any();
        let value = piece_value(kind);
        if (value > piece_value(mover) || !defended) && best.is_none_or(|(v, _, _)| value > v) {
            best = Some((value, sq, kind));
        }
    }
    best.map(|(_, sq, kind)| format!("attacks the {} on {sq}", piece_name(kind)))
}

/// The moved piece could be won before the move and cannot be won after it.
fn rescue(pos: &Position, child: &Position, mv: Move) -> Option<String> {
    let kind = pos.piece_at(mv.from())?.kind;
    let saved = kind != PieceKind::King
        && capture_gain(pos, mv.from()) > 0
        && capture_gain(child, mv.to()) == 0;
    saved.then(|| format!("moves the attacked {} to safety", piece_name(kind)))
}

/// Our most valuable piece (not the king) that the opponent can win after the move.
fn hanging(child: &Position, us: Color) -> Option<String> {
    let mut best: Option<(i32, Square, PieceKind)> = None;
    for sq in child.occupied_by(us) & !child.pieces(PieceKind::King) {
        if capture_gain(child, sq) > 0 {
            let kind = child.piece_at(sq)?.kind;
            let value = piece_value(kind);
            if best.is_none_or(|(v, _, _)| value > v) {
                best = Some((value, sq, kind));
            }
        }
    }
    best.map(|(_, sq, kind)| {
        format!(
            "leaves the {} on {sq} undefended against capture",
            piece_name(kind)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Game;
    use crate::engine::search::analyse;

    fn annotations(fen: &str) -> (Position, Vec<Annotation>) {
        let game = Game::from_fen(fen).unwrap();
        let pos = *game.position();
        let scored = analyse(&game);
        (pos, annotate(&pos, &scored))
    }

    fn find<'a>(pos: &Position, list: &'a [Annotation], uci: &str) -> &'a Annotation {
        let mv = pos.parse_uci(uci).unwrap();
        list.iter().find(|a| a.mv == mv).unwrap()
    }

    #[test]
    fn bucket_thresholds() {
        assert_eq!(bucket(100, 100), Bucket::Good);
        assert_eq!(bucket(100, 50), Bucket::Good);
        assert_eq!(bucket(100, 49), Bucket::Neutral);
        assert_eq!(bucket(100, -50), Bucket::Neutral);
        assert_eq!(bucket(100, -51), Bucket::Bad);
        assert_eq!(bucket(100, -200), Bucket::Bad);
        assert_eq!(bucket(100, -201), Bucket::Losing);
        assert_eq!(bucket(400, 350), Bucket::Winning);
        assert_eq!(bucket(320, 290), Bucket::Good);
        assert_eq!(bucket(-MATE_BOUND, -MATE_BOUND), Bucket::Losing);
    }

    #[test]
    fn capturing_a_hanging_queen() {
        let (pos, list) =
            annotations("rnb1kbnr/pppp1ppp/8/4p3/4P2q/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3");
        let a = find(&pos, &list, "f3h4");
        assert_eq!(a.san, "Nxh4");
        assert_eq!(a.effect, "captures the queen on h4, undefended");
        assert_eq!(a.bucket, Bucket::Winning);
        assert_eq!(list[0].mv, a.mv);
    }

    #[test]
    fn knight_fork() {
        let (pos, list) = annotations("r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1");
        assert_eq!(
            find(&pos, &list, "b5c7").effect,
            "gives check; attacks the rook on a8"
        );
    }

    #[test]
    fn promotions() {
        let (pos, list) = annotations("8/P6k/8/8/8/8/8/K7 w - - 0 1");
        assert_eq!(find(&pos, &list, "a7a8q").effect, "promotes to a queen");
        assert_eq!(find(&pos, &list, "a7a8n").effect, "promotes to a knight");
    }

    #[test]
    fn castling() {
        let (pos, list) = annotations("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1");
        assert!(
            find(&pos, &list, "e1g1")
                .effect
                .starts_with("castles kingside")
        );
        assert!(
            find(&pos, &list, "e1c1")
                .effect
                .starts_with("castles queenside")
        );
    }

    #[test]
    fn hanging_and_rescued_pieces() {
        // Pawns keep material on the board, so losing the knight is not a draw.
        let (pos, list) = annotations("4k3/p7/8/8/3b4/8/P4N2/7K w - - 0 1");
        let king_move = find(&pos, &list, "h1h2");
        assert_eq!(
            king_move.effect,
            "leaves the knight on f2 undefended against capture"
        );
        assert_eq!(king_move.bucket, Bucket::Losing);
        assert_eq!(
            find(&pos, &list, "f2d3").effect,
            "moves the attacked knight to safety"
        );
    }

    #[test]
    fn capture_qualifiers() {
        let (pos, list) = annotations("4k3/8/4p3/3n4/8/2N5/8/4K3 w - - 0 1");
        assert!(
            find(&pos, &list, "c3d5")
                .effect
                .starts_with("captures the knight on d5, equal trade")
        );
        let (pos, list) = annotations("4k3/8/4p3/3p4/8/2N5/8/4K3 w - - 0 1");
        assert!(
            find(&pos, &list, "c3d5")
                .effect
                .starts_with("captures the pawn on d5, loses material in the exchange")
        );
        let (pos, list) = annotations("4k3/8/4p3/3n4/4P3/8/8/4K3 w - - 0 1");
        assert!(
            find(&pos, &list, "e4d5")
                .effect
                .starts_with("captures the knight on d5, wins material in the exchange")
        );
        let (pos, list) = annotations("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1");
        assert!(
            find(&pos, &list, "e5d6")
                .effect
                .starts_with("captures en passant")
        );
    }

    #[test]
    fn checkmate_given_and_allowed() {
        let (pos, list) = annotations("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1");
        assert_eq!(find(&pos, &list, "e1e8").effect, "delivers checkmate");
        let (pos, list) = annotations("r5k1/5ppp/8/8/8/8/5PPP/6K1 w - - 0 1");
        let blunder = find(&pos, &list, "g1h1");
        assert!(
            blunder.effect.contains("allows checkmate"),
            "{}",
            blunder.effect
        );
        assert_eq!(blunder.bucket, Bucket::Losing);
        assert!(
            !find(&pos, &list, "h2h3")
                .effect
                .contains("allows checkmate")
        );
    }

    #[test]
    fn quiet_moves_say_so() {
        let (pos, list) = annotations("4k3/8/8/8/8/8/8/4K3 w - - 0 1");
        assert_eq!(find(&pos, &list, "e1d1").effect, "quiet move");
    }
}
