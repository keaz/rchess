//! Turns a game into the plain-language `state` object sent to Jev, written from
//! the point of view of the side to move (spec 5.5).

use serde::Serialize;

use crate::core::{Color, Game, PieceKind, Position};

use super::annotate::piece_name;
use super::eval::{is_endgame, material_balance};
use super::see::{capturers, least_valuable, winnable_pieces};

/// The `state` field of a Jev request. The struct's field order documents the spec 5.5
/// layout; on the wire the request JSON (see `jev.rs`) carries these keys in
/// alphabetical order, because serde_json sorts object keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct JevState {
    /// `White` or `Black`.
    pub side_to_move: String,
    /// The full-move number.
    pub move_number: u16,
    /// `opening`, `middlegame` or `endgame`.
    pub phase: String,
    /// The material balance in words, e.g. "White is ahead by about 1 pawn of material".
    pub material: String,
    /// `yes` or `no`.
    pub in_check: String,
    /// The side to move's pieces, e.g. "King g1, Rooks a1 and f1, Pawns a2 b2".
    pub our_pieces: String,
    /// The opponent's pieces, in the same form.
    pub their_pieces: String,
    /// The last six plies in SAN with move numbers; empty before the first move.
    pub recent_moves: String,
    /// Up to three of our pieces the opponent can win, most valuable first.
    pub threats_against_us: Vec<String>,
}

/// The Jev `state` for the game's current position.
pub fn describe(game: &Game) -> JevState {
    let pos = game.position();
    let us = pos.side_to_move();
    JevState {
        side_to_move: us.to_string(),
        move_number: pos.fullmove_number(),
        phase: phase(pos).to_string(),
        material: material(pos),
        in_check: if pos.is_check() { "yes" } else { "no" }.to_string(),
        our_pieces: pieces(pos, us),
        their_pieces: pieces(pos, !us),
        recent_moves: recent_moves(game),
        threats_against_us: threats(pos),
    }
}

fn phase(pos: &Position) -> &'static str {
    if is_endgame(pos) {
        "endgame"
    } else if pos.fullmove_number() <= 10 {
        "opening"
    } else {
        "middlegame"
    }
}

fn material(pos: &Position) -> String {
    let diff = material_balance(pos);
    if diff.abs() < 50 {
        return "material is equal".to_string();
    }
    let leader = if diff > 0 { Color::White } else { Color::Black };
    let pawns = (diff.abs() + 50) / 100;
    let unit = if pawns == 1 { "pawn" } else { "pawns" };
    format!("{leader} is ahead by about {pawns} {unit} of material")
}

const LISTING_ORDER: [PieceKind; 6] = [
    PieceKind::King,
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
    PieceKind::Pawn,
];

/// "King g8, Queen d8, Rooks a8 and f8, Pawns a7 b7 c7": kinds in a fixed order,
/// squares in a1..h8 order, empty kinds left out.
fn pieces(pos: &Position, color: Color) -> String {
    LISTING_ORDER
        .into_iter()
        .filter_map(|kind| {
            let squares: Vec<String> = pos
                .pieces_of(color, kind)
                .into_iter()
                .map(|sq| sq.to_string())
                .collect();
            if squares.is_empty() {
                return None;
            }
            let name = piece_name(kind);
            let mut label = name[..1].to_uppercase() + &name[1..];
            if squares.len() > 1 {
                label.push('s');
            }
            let list = if kind == PieceKind::Pawn {
                squares.join(" ")
            } else {
                join_with_and(&squares)
            };
            Some(format!("{label} {list}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// "a", "a and b", "a, b and c".
fn join_with_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The last six plies in SAN with move numbers, e.g. "11. Nxe5 Bd6 12. Qh5".
fn recent_moves(game: &Game) -> String {
    let moves = game.moves();
    let positions = game.positions();
    let start = moves.len().saturating_sub(6);
    let mut tokens = Vec::new();
    for i in start..moves.len() {
        let pos = &positions[i];
        let san = pos.to_san(moves[i]);
        match pos.side_to_move() {
            Color::White => tokens.push(format!("{}. {san}", pos.fullmove_number())),
            Color::Black if i == start => {
                tokens.push(format!("{}... {san}", pos.fullmove_number()))
            }
            Color::Black => tokens.push(san),
        }
    }
    tokens.join(" ")
}

/// Our pieces (not the king) the opponent can win by SEE, most valuable first, at most three.
fn threats(pos: &Position) -> Vec<String> {
    let us = pos.side_to_move();
    winnable_pieces(pos, us)
        .into_iter()
        .take(3)
        .filter_map(|(_, sq, kind)| {
            let attackers = capturers(pos, sq, pos.occupied(), !us);
            let (from, attacker) = least_valuable(pos, attackers)?;
            Some(format!(
                "Our {} on {sq} is attacked by the {} on {from}",
                piece_name(kind),
                piece_name(attacker)
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn play(game: &mut Game, moves: &[&str]) {
        for uci in moves {
            let mv = game.position().parse_uci(uci).unwrap();
            game.play(mv).unwrap();
        }
    }

    #[test]
    fn opening_state_golden() {
        let mut game = Game::new();
        play(&mut game, &["e2e4", "e7e5", "g1f3", "b8c6", "f1c4"]);
        let state = serde_json::to_value(describe(&game)).unwrap();
        assert_eq!(
            state,
            json!({
                "side_to_move": "Black",
                "move_number": 3,
                "phase": "opening",
                "material": "material is equal",
                "in_check": "no",
                "our_pieces": "King e8, Queen d8, Rooks a8 and h8, Bishops c8 and f8, Knights c6 and g8, Pawns e5 a7 b7 c7 d7 f7 g7 h7",
                "their_pieces": "King e1, Queen d1, Rooks a1 and h1, Bishops c1 and c4, Knights b1 and f3, Pawns a2 b2 c2 d2 f2 g2 h2 e4",
                "recent_moves": "1. e4 e5 2. Nf3 Nc6 3. Bc4",
                "threats_against_us": []
            })
        );
    }

    #[test]
    fn recent_moves_from_a_black_start() {
        let mut game = Game::from_fen("4k3/8/8/8/8/8/4P3/4K3 b - - 0 12").unwrap();
        play(&mut game, &["e8d7", "e2e4"]);
        let state = describe(&game);
        assert_eq!(state.recent_moves, "12... Kd7 13. e4");
        assert_eq!(state.material, "White is ahead by about 1 pawn of material");
        assert_eq!(state.phase, "endgame");
    }

    #[test]
    fn recent_moves_keep_only_six_plies() {
        let mut game = Game::new();
        play(
            &mut game,
            &["g1f3", "g8f6", "b1c3", "b8c6", "e2e4", "e7e5", "d2d4"],
        );
        assert_eq!(
            describe(&game).recent_moves,
            "1... Nf6 2. Nc3 Nc6 3. e4 e5 4. d4"
        );
    }

    #[test]
    fn material_wording() {
        let game = Game::from_fen("4k3/8/8/8/8/8/8/Q3K3 b - - 0 1").unwrap();
        assert_eq!(
            describe(&game).material,
            "White is ahead by about 9 pawns of material"
        );
        let game = Game::from_fen("3qk3/8/8/8/8/8/8/1N2K3 w - - 0 30").unwrap();
        let state = describe(&game);
        assert_eq!(
            state.material,
            "Black is ahead by about 6 pawns of material"
        );
        assert_eq!(state.phase, "endgame");
        assert_eq!(state.our_pieces, "King e1, Knight b1");
        assert_eq!(state.their_pieces, "King e8, Queen d8");
    }

    #[test]
    fn threats_most_valuable_first_and_capped() {
        // Four undefended white pieces are attacked; the knight on g8 is fourth and dropped.
        let game = Game::from_fen("r4kNr/4b3/2B5/4n3/7R/8/8/Q6K w - - 0 1").unwrap();
        assert_eq!(
            describe(&game).threats_against_us,
            vec![
                "Our queen on a1 is attacked by the rook on a8",
                "Our rook on h4 is attacked by the bishop on e7",
                "Our bishop on c6 is attacked by the knight on e5",
            ]
        );
    }

    #[test]
    fn defended_pieces_are_not_threats() {
        // The knight on f2 is defended by the queen on h4 (and the rook on d1 by the knight),
        // so only the queen and the bishop can be won.
        let game = Game::from_fen("4k3/8/8/8/P2q3Q/8/1B3N2/3R3K w - - 0 1").unwrap();
        assert_eq!(
            describe(&game).threats_against_us,
            vec![
                "Our queen on h4 is attacked by the queen on d4",
                "Our bishop on b2 is attacked by the queen on d4",
            ]
        );
    }

    #[test]
    fn a_pinned_attacker_is_not_a_threat() {
        // The knight on e5 is pinned by Re1 and cannot take the queen on c4.
        let game = Game::from_fen("4k3/8/8/4n3/2Q5/8/8/4RK2 w - - 0 1").unwrap();
        assert_eq!(describe(&game).threats_against_us, Vec::<String>::new());
    }

    #[test]
    fn a_capture_that_releases_a_pin_is_not_a_threat() {
        // Qxc3 would leave the e-file and free the pinned bishop on e5 to take the queen.
        let game = Game::from_fen("4k3/8/8/4b3/8/2p5/8/4QK2 b - - 0 1").unwrap();
        let threats = describe(&game).threats_against_us;
        assert!(!threats.iter().any(|t| t.contains("c3")), "{threats:?}");
    }

    #[test]
    fn check_and_middlegame() {
        let game =
            Game::from_fen("rnbqkbnr/ppp2ppp/8/3pp3/4P3/5Q2/PPPP1PPP/RNB1KBNR b KQkq - 0 12")
                .unwrap();
        let state = describe(&game);
        assert_eq!(state.phase, "middlegame");
        assert_eq!(state.in_check, "no");
        let game = Game::from_fen("4k3/8/8/8/8/8/8/4K2r w - - 0 1").unwrap();
        assert_eq!(describe(&game).in_check, "yes");
    }
}
