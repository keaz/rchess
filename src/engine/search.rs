//! Fixed-depth negamax alpha-beta search with quiescence. `analyse` gives every
//! legal move of the current position an exact score, which the player uses for
//! annotation buckets, the shortlist and the veto.

use crate::core::{Game, Move, PieceKind, Position};

use super::eval::{evaluate, piece_value};

/// Score of delivering checkmate on the next move; a mate `n` plies away scores `MATE - n`.
pub const MATE: i32 = 30_000;
/// Any score beyond this (in absolute value) is a forced mate.
pub const MATE_BOUND: i32 = MATE - 1_000;

const INFINITY: i32 = MATE + 1;
/// Plies searched in total, counting the root move itself.
const ROOT_DEPTH: u32 = 3;
/// Maximum plies of quiescence below the main search.
const QUIESCENCE_PLIES: u32 = 8;

/// A legal root move and its score in centipawns for the side to move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScoredMove {
    /// A legal move of the analysed position.
    pub mv: Move,
    /// Its score in centipawns for the side to move; a mate `n` plies away is `±(MATE - n)`.
    pub score: i32,
}

/// Scores every legal move of the game's current position, best first (ties keep
/// move-generation order). Empty when the side to move has no legal moves.
pub fn analyse(game: &Game) -> Vec<ScoredMove> {
    let pos = *game.position();
    // Only positions since the last irreversible move can repeat.
    let window = pos.halfmove_clock() as usize + 1;
    let history: Vec<u64> = game
        .positions()
        .iter()
        .rev()
        .take(window)
        .map(|p| p.hash())
        .collect();
    let mut searcher = Searcher {
        history,
        path: Vec::new(),
    };
    let mut scored: Vec<ScoredMove> = pos
        .legal_moves()
        .iter()
        .map(|&mv| {
            let score = -searcher.negamax(&pos.play(mv), ROOT_DEPTH - 1, 1, -INFINITY, INFINITY);
            ScoredMove { mv, score }
        })
        .collect();
    scored.sort_by_key(|s| std::cmp::Reverse(s.score));
    scored
}

struct Searcher {
    /// Hashes of game positions that can still repeat, including the root.
    history: Vec<u64>,
    /// Hashes of positions on the current search path below the root.
    path: Vec<u64>,
}

impl Searcher {
    /// Draws that hold whatever the legal moves are: a repeated position or
    /// insufficient material. The fifty-move rule is checked separately, after
    /// mate and stalemate, because a checkmate on the hundredth half-move wins
    /// (the same precedence as `Game::outcome`).
    fn is_forced_draw(&self, pos: &Position) -> bool {
        let hash = pos.hash();
        pos.is_insufficient_material() || self.history.contains(&hash) || self.path.contains(&hash)
    }

    fn negamax(&mut self, pos: &Position, depth: u32, ply: i32, mut alpha: i32, beta: i32) -> i32 {
        if self.is_forced_draw(pos) {
            return 0;
        }
        if depth == 0 {
            return self.quiesce(pos, ply, alpha, beta, 0);
        }
        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            return if pos.is_check() { -(MATE - ply) } else { 0 };
        }
        if pos.halfmove_clock() >= 100 {
            return 0;
        }
        moves.sort_unstable_by_key(|&mv| -order_key(pos, mv));
        self.path.push(pos.hash());
        let mut best = -INFINITY;
        for &mv in moves.iter() {
            let score = -self.negamax(&pos.play(mv), depth - 1, ply + 1, -beta, -alpha);
            best = best.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        self.path.pop();
        best
    }

    /// Resolves captures (and all evasions when in check) so the static
    /// evaluation is only trusted in quiet positions.
    fn quiesce(&mut self, pos: &Position, ply: i32, mut alpha: i32, beta: i32, qply: u32) -> i32 {
        if self.is_forced_draw(pos) {
            return 0;
        }
        let in_check = pos.is_check();
        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            return if in_check { -(MATE - ply) } else { 0 };
        }
        if pos.halfmove_clock() >= 100 {
            return 0;
        }
        if qply >= QUIESCENCE_PLIES {
            return evaluate(pos);
        }
        let mut best = -INFINITY;
        if !in_check {
            let stand_pat = evaluate(pos);
            if stand_pat >= beta {
                return stand_pat;
            }
            alpha = alpha.max(stand_pat);
            best = stand_pat;
        }
        moves.sort_unstable_by_key(|&mv| -order_key(pos, mv));
        self.path.push(pos.hash());
        for &mv in moves.iter() {
            let tactical = mv.is_capture() || mv.promotion() == Some(PieceKind::Queen);
            if !in_check && !tactical {
                continue;
            }
            let score = -self.quiesce(&pos.play(mv), ply + 1, -beta, -alpha, qply + 1);
            best = best.max(score);
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        self.path.pop();
        best
    }
}

/// Move ordering: captures by most valuable victim, least valuable attacker (MVV-LVA),
/// then promotions, then quiet moves.
fn order_key(pos: &Position, mv: Move) -> i32 {
    let promotion = mv.promotion().map_or(0, piece_value);
    if mv.is_capture() {
        let victim = if mv.is_en_passant() {
            piece_value(PieceKind::Pawn)
        } else {
            pos.piece_at(mv.to()).map_or(0, |p| piece_value(p.kind))
        };
        let attacker = pos.piece_at(mv.from()).map_or(0, |p| piece_value(p.kind));
        20_000 + 10 * victim - attacker + promotion
    } else if promotion > 0 {
        10_000 + promotion
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyse_fen(fen: &str) -> Vec<ScoredMove> {
        analyse(&Game::from_fen(fen).unwrap())
    }

    fn score_of(scored: &[ScoredMove], pos: &Position, uci: &str) -> i32 {
        let mv = pos.parse_uci(uci).unwrap();
        scored.iter().find(|s| s.mv == mv).unwrap().score
    }

    #[test]
    fn finds_mate_in_one() {
        let fen = "6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1";
        let scored = analyse_fen(fen);
        assert_eq!(scored[0].mv.to_uci(), "e1e8");
        assert_eq!(scored[0].score, MATE - 1);
    }

    #[test]
    fn finds_mate_in_two() {
        // Legal's mate: 1. Nf6+ gxf6 2. Bxf7#.
        let fen = "r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 1";
        let scored = analyse_fen(fen);
        assert_eq!(scored[0].mv.to_uci(), "d5f6");
        assert_eq!(scored[0].score, MATE - 3);
    }

    #[test]
    fn refuses_to_hang_the_queen() {
        // Qxd5 wins a pawn but cxd5 wins the queen.
        let fen = "4k3/8/2p5/3p4/8/8/8/3QK3 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let scored = analyse_fen(fen);
        assert_ne!(scored[0].mv.to_uci(), "d1d5");
        assert!(scored[0].score - score_of(&scored, &pos, "d1d5") > 300);
    }

    #[test]
    fn stalemating_move_scores_zero() {
        // Qg6 stalemates; Qg7 is mate.
        let fen = "7k/8/5K2/8/8/8/8/6Q1 w - - 0 1";
        let pos = Position::from_fen(fen).unwrap();
        let scored = analyse_fen(fen);
        assert_eq!(score_of(&scored, &pos, "g1g6"), 0);
        assert_eq!(scored[0].score, MATE - 1);
    }

    #[test]
    fn fifty_move_rule_scores_zero() {
        // A queen up, but every quiet move reaches the fifty-move limit.
        let scored = analyse_fen("4k3/8/8/8/8/8/8/Q3K3 w - - 99 80");
        assert!(!scored.is_empty());
        assert!(scored.iter().all(|s| s.score == 0), "{scored:?}");
    }

    #[test]
    fn mate_on_the_hundredth_halfmove_is_mate() {
        // Re8# makes the clock 100; checkmate outranks the fifty-move rule.
        let scored = analyse_fen("6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 99 80");
        assert_eq!(scored[0].mv.to_uci(), "e1e8");
        assert_eq!(scored[0].score, MATE - 1);
    }

    #[test]
    fn quiet_mate_on_the_hundredth_halfmove_is_mate() {
        // Qg7# is a quiet move that makes the clock 100.
        let scored = analyse_fen("7k/8/5K2/8/8/8/8/6Q1 w - - 99 80");
        assert_eq!(scored[0].mv.to_uci(), "g1g7");
        assert_eq!(scored[0].score, MATE - 1);
    }

    #[test]
    fn repetition_scores_zero() {
        let mut game = Game::from_fen("4k3/8/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
        for uci in ["a1a2", "e8d8", "a2a1", "d8e8"] {
            let mv = game.position().parse_uci(uci).unwrap();
            game.play(mv).unwrap();
        }
        let pos = *game.position();
        let scored = analyse(&game);
        assert_eq!(
            score_of(&scored, &pos, "a1a2"),
            0,
            "repeats the position after 1. Qa2"
        );
        assert!(scored[0].score > 800);
    }

    #[test]
    fn insufficient_material_scores_zero() {
        let scored = analyse_fen("4k3/8/8/8/8/8/8/2B1K3 w - - 0 1");
        assert!(scored.iter().all(|s| s.score == 0));
    }

    #[test]
    fn no_moves_means_empty() {
        assert!(analyse_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").is_empty());
    }

    /// Search budget from spec 5.3. Run with:
    /// cargo test --release --lib engine::search -- --ignored
    #[test]
    #[ignore]
    fn kiwipete_within_budget() {
        let game =
            Game::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        let start = std::time::Instant::now();
        let scored = analyse(&game);
        let elapsed = start.elapsed();
        assert_eq!(scored.len(), 48);
        assert!(elapsed.as_millis() < 250, "took {elapsed:?}");
    }
}
