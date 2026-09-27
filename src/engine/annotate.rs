//! Plain-language facts about each root move, plus a bucket derived from its
//! search score. Jev sees these words, never the numbers.

use serde::Serialize;

use crate::core::{
    Bitboard, Color, Move, PieceKind, Position, Square, bishop_attacks, king_attacks,
    knight_attacks, pawn_attacks, queen_attacks, rook_attacks,
};

use super::eval::piece_value;
use super::search::{MATE_BOUND, ScoredMove};
use super::see::{capture_gain, capturers, exchange_value, see, winnable_pieces};

/// The engine's verdict on a move relative to the best move found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    /// At most 50 centipawns below the best move, and at least +300 or a mate for us.
    Winning,
    /// At most 50 centipawns below the best move.
    Good,
    /// 51 to 150 centipawns below the best move.
    Neutral,
    /// 151 to 300 centipawns below the best move.
    Bad,
    /// More than 300 centipawns below the best move, or allows checkmate.
    Losing,
}

/// One root move with its SAN, search score, effect words and bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    /// The move.
    pub mv: Move,
    /// The move in SAN; the option key sent to Jev.
    pub san: String,
    /// Search score in centipawns for the side to move.
    pub score: i32,
    /// Plain-language facts joined with "; ", or "quiet move".
    pub effect: String,
    /// The verdict relative to the best move.
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
        let recapture = capturers(&child, mv.to(), child.occupied(), !us).any();
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
    if !checkmate && let Some(fact) = hanging(&child, us, mv) {
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
/// A king counts as worth more than anything, so it only "attacks" undefended pieces.
/// A defender pinned to its king off the line to the attacked piece does not count.
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
        // A defender pinned to its king off the line to `sq` does not defend it.
        let defended = capturers(child, sq, child.occupied(), !us).any();
        let value = piece_value(kind);
        if (value > exchange_value(mover) || !defended) && best.is_none_or(|(v, _, _)| value > v) {
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
/// After a capture the destination square is left out: the capture's exchange
/// qualifier already says what happens there.
fn hanging(child: &Position, us: Color, mv: Move) -> Option<String> {
    let skipped = mv.is_capture().then(|| mv.to());
    winnable_pieces(child, us)
        .into_iter()
        .find(|&(_, sq, _)| Some(sq) != skipped)
        .map(|(_, sq, kind)| format!("leaves the {} on {sq} exposed to capture", piece_name(kind)))
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
            "leaves the knight on f2 exposed to capture"
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
    fn a_capture_does_not_call_the_capturing_piece_exposed() {
        // Nxd5 exd5 Rxd5: the recapture is part of the exchange, not a hanging piece.
        let (pos, list) = annotations("4k3/8/4p3/3n4/8/2N5/8/3RK3 w - - 0 1");
        assert_eq!(
            find(&pos, &list, "c3d5").effect,
            "captures the knight on d5, wins material in the exchange"
        );
        // Scotch Game: 3...exd4 4. Nxd4 is a normal trade.
        let (pos, list) =
            annotations("r1bqkbnr/pppp1ppp/2n5/4p3/3PP3/5N2/PPP2PPP/RNBQKB1R b KQkq d3 0 3");
        assert_eq!(
            find(&pos, &list, "e5d4").effect,
            "captures the pawn on d4, equal trade"
        );
    }

    #[test]
    fn a_pinned_defender_does_not_defend() {
        // Nc6 is pinned by Bb5, so nothing can take back on e5.
        let (pos, list) =
            annotations("r1bqkbnr/ppp2ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 0 4");
        assert_eq!(
            find(&pos, &list, "f3e5").effect,
            "captures the pawn on e5, undefended"
        );
    }

    #[test]
    fn a_capture_that_releases_a_pin_loses_the_capturer() {
        // Qe1 pins Be5 to the king on e8; Qxc3 leaves the e-file and Bxc3 wins the queen.
        let fen = "4k3/8/8/4b3/8/2p5/8/4QK2 w - - 0 1";
        let (pos, list) = annotations(fen);
        let effect = &find(&pos, &list, "e1c3").effect;
        assert!(
            effect.starts_with("captures the pawn on c3, loses material in the exchange"),
            "{effect}"
        );
        // With Black to move the pawn on c3 is not really attacked, so c2 rescues nothing.
        let (pos, list) = annotations(&fen.replace(" w ", " b "));
        let effect = &find(&pos, &list, "c3c2").effect;
        assert!(!effect.contains("to safety"), "{effect}");
    }

    #[test]
    fn a_pinned_defender_does_not_defend_an_attacked_piece() {
        // Re1 pins Be6 to the king on e8, so the bishop does not defend the knight
        // on d5: Rd1 attacks it although the knight is worth less than the rook.
        let (pos, list) = annotations("4k3/8/4b3/3n4/8/8/8/R3R2K w - - 0 1");
        assert_eq!(find(&pos, &list, "a1d1").effect, "attacks the knight on d5");
    }

    #[test]
    fn a_king_only_attacks_undefended_pieces() {
        // The pawn on c5 defends the rook on d4: the king can never take it.
        let (pos, list) = annotations("4k3/8/8/2p5/3r4/8/4K3/8 w - - 0 1");
        assert_eq!(find(&pos, &list, "e2e3").effect, "quiet move");
        let (pos, list) = annotations("4k3/8/8/8/3r4/8/4K3/8 w - - 0 1");
        assert_eq!(find(&pos, &list, "e2e3").effect, "attacks the rook on d4");
    }

    #[test]
    fn a_defended_cheaper_piece_is_not_attacked() {
        // Nc3 hits the pawn on d5, which e6 defends and which is worth less than the knight.
        let (pos, list) = annotations("4k3/8/4p3/3p4/8/8/8/1N2K3 w - - 0 1");
        assert_eq!(find(&pos, &list, "b1c3").effect, "quiet move");
        let (pos, list) = annotations("4k3/8/8/3p4/8/8/8/1N2K3 w - - 0 1");
        assert_eq!(find(&pos, &list, "b1c3").effect, "attacks the pawn on d5");
    }

    #[test]
    fn checkmate_skips_the_exposed_fact() {
        // After Re8# the knight on a4 could be taken by b5, but the game is over.
        let (pos, list) = annotations("6k1/5ppp/8/1p6/N7/8/5PPP/4R1K1 w - - 0 1");
        assert_eq!(find(&pos, &list, "e1e8").effect, "delivers checkmate");
        assert_eq!(
            find(&pos, &list, "g1f1").effect,
            "leaves the knight on a4 exposed to capture"
        );
    }

    #[test]
    fn moving_to_another_attacked_square_is_not_a_rescue() {
        // The knight on c3 is attacked by b4; on d5 it is attacked by e6 instead.
        let (pos, list) = annotations("4k3/8/4p3/8/1p6/2N5/8/7K w - - 0 1");
        assert_eq!(
            find(&pos, &list, "c3d5").effect,
            "attacks the pawn on b4; leaves the knight on d5 exposed to capture"
        );
        assert_eq!(
            find(&pos, &list, "c3e2").effect,
            "moves the attacked knight to safety"
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
