//! Property tests: random legal games must keep every Position invariant.

use chess::core::{Game, Position};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Walk a random legal game. At every ply:
    /// - FEN round-trips exactly (so the parser and printer agree),
    /// - the incrementally updated hash equals the hash computed from scratch
    ///   by `from_fen`,
    /// - every generated move is accepted by `Game::play` and SAN round-trips.
    #[test]
    fn random_games_keep_invariants(choices in prop::collection::vec(any::<u8>(), 0..120)) {
        let mut game = Game::new();
        for choice in choices {
            let pos = *game.position();
            let moves = pos.legal_moves();
            if moves.is_empty() {
                break;
            }
            let mv = moves[choice as usize % moves.len()];
            prop_assert_eq!(pos.parse_san(&pos.to_san(mv)), Ok(mv));
            prop_assert_eq!(pos.parse_uci(&mv.to_uci()), Ok(mv));
            game.play(mv).unwrap();

            let next = game.position();
            let reparsed = Position::from_fen(&next.to_fen()).unwrap();
            prop_assert_eq!(reparsed, *next);
            prop_assert_eq!(reparsed.hash(), next.hash());
        }
    }
}
