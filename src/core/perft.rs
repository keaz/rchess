use super::Position;

/// Counts leaf nodes of the legal move tree to `depth`. The standard
/// correctness test for move generators: compare against published counts.
pub fn perft(pos: &Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let moves = pos.legal_moves();
    if depth == 1 {
        return moves.len() as u64;
    }
    moves
        .iter()
        .map(|&mv| perft(&pos.play(mv), depth - 1))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Positions and node counts from https://www.chessprogramming.org/Perft_Results
    const SUITE: &[(&str, &[u64])] = &[
        (
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            &[20, 400, 8_902, 197_281, 4_865_609, 119_060_324],
        ),
        (
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            &[48, 2_039, 97_862, 4_085_603, 193_690_690],
        ),
        (
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            &[14, 191, 2_812, 43_238, 674_624, 11_030_083],
        ),
        (
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            &[6, 264, 9_467, 422_333, 15_833_292],
        ),
        (
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            &[44, 1_486, 62_379, 2_103_487, 89_941_194],
        ),
        (
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            &[46, 2_079, 89_890, 3_894_594, 164_075_551],
        ),
    ];

    fn run_suite(max_nodes: u64) {
        for (fen, counts) in SUITE {
            let pos = Position::from_fen(fen).unwrap();
            for (depth, &expected) in counts.iter().enumerate() {
                if expected > max_nodes {
                    break;
                }
                assert_eq!(
                    perft(&pos, depth as u32 + 1),
                    expected,
                    "{fen} depth {}",
                    depth + 1
                );
            }
        }
    }

    #[test]
    fn perft_suite_shallow() {
        run_suite(5_000_000);
    }

    /// Full published depths. Run with: cargo test --release -- --ignored perft_suite_deep
    #[test]
    #[ignore]
    fn perft_suite_deep() {
        run_suite(u64::MAX);
    }
}
