//! Evaluates Jev as a chess player on fixed positions (spec 5.8): Jev's pick
//! against the local search best, vetoes, fallbacks, latency and token cost.
//!
//! Run with: cargo run --release --example jev_eval   (needs JEV_API_KEY)

use std::time::Duration;

use chess::core::Game;
use chess::engine::{ComputerPlayer, EngineConfig, MoveSource, analyse};

const POSITIONS: [(&str, &str); 20] = [
    // Openings
    (
        "Start position",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    ),
    (
        "After 1. e4",
        "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1",
    ),
    (
        "Italian Game",
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3",
    ),
    (
        "Sicilian, 2. Nf3",
        "rnbqkbnr/pp1ppppp/8/2p5/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2",
    ),
    (
        "Queen's Gambit",
        "rnbqkbnr/ppp1pppp/8/3p4/2PP4/8/PP2PPPP/RNBQKBNR b KQkq - 0 2",
    ),
    // Tactics
    (
        "Hanging queen",
        "rnb1kbnr/pppp1ppp/8/4p3/4P2q/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3",
    ),
    ("Knight fork", "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 1"),
    ("Undefended rook", "4k3/8/8/3r4/8/8/3Q4/4K3 w - - 0 1"),
    ("Pawn fork", "4k3/8/8/2n1b3/8/2PP4/8/4K3 w - - 0 1"),
    ("Promotion", "8/P6k/8/8/8/8/8/K7 w - - 0 1"),
    ("Mate in one", "6k1/5ppp/8/8/8/8/5PPP/4R1K1 w - - 0 1"),
    // Endgames
    ("King and pawn", "8/8/8/4k3/8/8/4P3/4K3 w - - 0 1"),
    ("King and rook", "8/8/8/4k3/8/8/8/R3K3 w - - 0 1"),
    ("King and queen", "8/8/8/4k3/8/8/8/3QK3 w - - 0 1"),
    ("Rook endgame", "8/5pk1/8/8/8/8/R4PK1/r7 w - - 0 1"),
    ("Pawn race", "8/p7/8/8/8/8/7P/k6K w - - 0 1"),
    // Defence
    (
        "Queen attacked by pawn",
        "4k3/8/8/8/8/2p5/1Q6/4K3 w - - 0 1",
    ),
    ("Back-rank threat", "3r2k1/5ppp/8/8/8/8/5PPP/6K1 w - - 0 1"),
    ("In check", "4k3/8/8/8/8/8/3q4/4K3 w - - 0 1"),
    ("Hanging knight", "4k3/p7/8/8/3b4/8/P4N2/7K w - - 0 1"),
];

/// Jev input price in US dollars per million tokens (spec 5.1).
const DOLLARS_PER_MILLION_TOKENS: f64 = 0.042;

fn main() {
    // Validate every position before spending anything.
    let games: Vec<(&str, Game)> = POSITIONS
        .iter()
        .map(|(name, fen)| {
            let game = Game::from_fen(fen).unwrap_or_else(|e| panic!("{name}: {e}"));
            (*name, game)
        })
        .collect();

    let config = EngineConfig::from_env();
    for warning in &config.warnings {
        eprintln!("warning: {warning}");
    }
    if config.api_key.is_none() {
        eprintln!(
            "JEV_API_KEY (or TYPESAFE_API_KEY) is not set; {} positions checked, nothing sent to Jev.",
            games.len()
        );
        std::process::exit(2);
    }
    let player = ComputerPlayer::from_config(config);

    println!(
        "{:<24} {:<8} {:<8} {:<10} {:>7} {:>7}",
        "position", "played", "search", "source", "ms", "tokens"
    );
    let (mut answered, mut agreed, mut vetoes, mut fallbacks) = (0, 0, 0, 0);
    let mut total_latency = Duration::ZERO;
    let mut total_tokens: u64 = 0;
    for (name, game) in &games {
        let search_best = analyse(game)
            .first()
            .map(|s| game.position().to_san(s.mv))
            .unwrap_or_default();
        let Some(result) = player.choose_move(game) else {
            println!("{name:<24} game over");
            continue;
        };
        let (source, jev_pick) = match &result.source {
            MoveSource::Jev => ("jev", Some(result.san.clone())),
            MoveSource::Vetoed { jev_pick } => ("vetoed", Some(jev_pick.clone())),
            MoveSource::OnlyMove => ("only-move", None),
            MoveSource::MateInOne => ("mate-in-1", None),
            MoveSource::Fallback => ("fallback", None),
        };
        if let Some(pick) = &jev_pick {
            answered += 1;
            if *pick == search_best {
                agreed += 1;
            }
        }
        if matches!(result.source, MoveSource::Vetoed { .. }) {
            vetoes += 1;
        }
        if result.source == MoveSource::Fallback {
            fallbacks += 1;
        }
        total_latency += result.latency;
        total_tokens += u64::from(result.input_tokens.unwrap_or(0));
        let tokens = result
            .input_tokens
            .map_or("-".to_string(), |t| t.to_string());
        println!(
            "{name:<24} {:<8} {search_best:<8} {source:<10} {:>7} {tokens:>7}",
            result.san,
            result.latency.as_millis()
        );
        if let Some(note) = &result.note {
            println!("    note: {note}");
        }
    }

    let percent = |part: usize, whole: usize| {
        if whole == 0 {
            0.0
        } else {
            100.0 * part as f64 / whole as f64
        }
    };
    println!();
    println!("positions: {}", games.len());
    println!(
        "answered by Jev: {answered}; agreement with search best: {agreed}/{answered} ({:.0}%)",
        percent(agreed, answered)
    );
    println!(
        "vetoes: {vetoes} ({:.0}% of answers); fallbacks: {fallbacks}",
        percent(vetoes, answered)
    );
    println!(
        "mean latency: {} ms",
        total_latency.as_millis() / games.len() as u128
    );
    println!(
        "input tokens: {total_tokens} (about ${:.6})",
        total_tokens as f64 * DOLLARS_PER_MILLION_TOKENS / 1_000_000.0
    );
}
