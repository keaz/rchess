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

    /// Arbitrary boards: `from_fen` either rejects them or yields a position whose
    /// moves can all be generated, printed and played without panicking.
    #[test]
    fn arbitrary_positions_never_panic(
        cells in prop::collection::vec(0u8..48, 64),
        kings in (0u8..64, 0u8..64),
        white_to_move in any::<bool>(),
        castling in 0u8..64,
        ep_file in 0u8..32,
    ) {
        const PIECES: &[u8; 10] = b"PNBRQpnbrq";
        let mut board = [None::<char>; 64];
        for (i, &c) in cells.iter().enumerate() {
            if let Some(&p) = PIECES.get(c as usize) {
                board[i] = Some(p as char);
            }
        }
        board[kings.0 as usize] = Some('K');
        board[kings.1 as usize] = Some('k');

        let mut fen = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match board[rank * 8 + file] {
                    Some(c) => {
                        if empty > 0 {
                            fen.push_str(&empty.to_string());
                            empty = 0;
                        }
                        fen.push(c);
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
        fen.push_str(if white_to_move { " w " } else { " b " });
        let rights: String = "KQkq"
            .chars()
            .enumerate()
            .filter(|&(i, _)| castling < 16 && castling & (1 << i) != 0)
            .map(|(_, c)| c)
            .collect();
        fen.push_str(if rights.is_empty() { "-" } else { &rights });
        if ep_file < 8 {
            let rank = if white_to_move { '6' } else { '3' };
            fen.push_str(&format!(" {}{} 0 1", (b'a' + ep_file) as char, rank));
        } else {
            fen.push_str(" - 0 1");
        }

        if let Ok(pos) = Position::from_fen(&fen) {
            for &mv in pos.legal_moves().iter() {
                let _ = pos.to_san(mv);
                let _ = pos.play(mv).legal_moves();
            }
            prop_assert_eq!(Position::from_fen(&pos.to_fen()).unwrap(), pos);
        }
    }
}
