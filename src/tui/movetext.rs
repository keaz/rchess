//! Lenient move-text parsing for the command box.
//!
//! [`parse_move`] first drops a trailing `e.p.`, then tries, in order:
//!
//! 1. strict SAN ([`ChessPosition::parse_san`]): `e4`, `Nbd7`, `exd6`, `e8=Q`, `O-O`, `Qh4#`;
//! 2. UCI on the ASCII-lowercased text ([`ChessPosition::parse_uci`]): `e2e4`, `E7E8Q`;
//! 3. long algebraic: the input is normalised (case folded, `x - + # = ! ?` dropped, `0` read
//!    as `O`) and read as UCI, optionally after a piece letter that must name the moving
//!    piece: `e2-e4`, `e5xd6`, `e7e8=q`, `e2e4+`, `Ng1-f3`;
//! 4. loose SAN: the normalised input is compared with the normalised SAN of every legal
//!    move, so `nf3`, `Ne5` for `Nxe5`, `e8q`, `o-o` and `OO` are understood;
//! 5. if loose SAN finds nothing, the input is read as a piece move with any level of
//!    disambiguation (`Nd2`, `Nbd2`, `N1d2`, `Nb1d2`), so an under-disambiguated move is
//!    reported as ambiguous rather than illegal, and an over-disambiguated one is accepted;
//! 6. a pawn move to the last rank without its piece (`e8`, `e7e8`, `dxe8`) is reported as
//!    [`MoveTextError::NeedsPromotion`], so the caller can ask which piece.
//!
//! Case matters in one place: an uppercase `N`, `B`, `R`, `Q` or `K` followed by a square
//! names the piece, so steps 3 to 6 only consider that piece's moves. `Bc4` is never read as
//! the b-pawn capture `bxc4`; a lowercase `bc4` may be either, and is ambiguous when both
//! are legal. `B4` (a digit after the letter) is still the pawn move `b4`.
//!
//! A move is returned only when exactly one legal move matches. Errors are short enough for
//! a one-line status bar: they echo at most the start of the input ([`shorten`]) and never
//! the FEN that [`crate::core::ChessError`] embeds.

use thiserror::Error;

use super::glyphs::shorten;
use crate::core::{Move, PieceKind, Position as ChessPosition, Square};

/// Why move text could not be turned into exactly one legal move.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MoveTextError {
    /// The text was empty or only whitespace.
    #[error("type a move")]
    Empty,
    /// No legal move matches. Carries the trimmed input; the message shows at most
    /// its first [`ECHO_MAX_CHARS`](crate::tui::glyphs::ECHO_MAX_CHARS) characters.
    #[error("not a legal move: {}", shorten(.0))]
    NotLegal(String),
    /// Several legal moves match. Carries their SAN (with check suffixes), sorted.
    #[error("ambiguous: {}", .0.join(", "))]
    Ambiguous(Vec<String>),
    /// A legal promotion typed without the piece: the caller should ask for it.
    #[error("choose the promotion piece for {from}{to}")]
    NeedsPromotion {
        /// The pawn's square.
        from: Square,
        /// The promotion square.
        to: Square,
    },
}

/// Parses move text typed by the user into a legal move of `pos`.
///
/// See the [module documentation](self) for the accepted spellings and the order in which
/// they are tried.
///
/// # Errors
///
/// [`MoveTextError::Empty`] for blank text, [`MoveTextError::Ambiguous`] when several legal
/// moves match the loose spelling, [`MoveTextError::NeedsPromotion`] for a promotion without
/// its piece, and [`MoveTextError::NotLegal`] when nothing matches.
pub fn parse_move(pos: &ChessPosition, text: &str) -> Result<Move, MoveTextError> {
    let text = strip_en_passant_mark(text.trim());
    if text.is_empty() {
        return Err(MoveTextError::Empty);
    }
    if let Ok(mv) = pos.parse_san(text) {
        return Ok(mv);
    }
    if let Ok(mv) = pos.parse_uci(&text.to_ascii_lowercase()) {
        return Ok(mv);
    }
    loose_match(pos, text)
}

/// Steps 3 to 6 of [`parse_move`]: matching on normalised text.
fn loose_match(pos: &ChessPosition, text: &str) -> Result<Move, MoveTextError> {
    let wanted = normalise(text);
    let named = named_piece(text);
    if let Some(mv) = long_algebraic(pos, &wanted, named) {
        return Ok(mv);
    }
    let kind_of = |mv: Move| pos.piece_at(mv.from()).map(|piece| piece.kind);
    let candidates: Vec<(Move, String)> = pos
        .legal_moves()
        .iter()
        .copied()
        .filter(|&mv| named.is_none_or(|kind| kind_of(mv) == Some(kind)))
        .map(|mv| (mv, pos.to_san(mv)))
        .collect();

    let mut matches: Vec<&(Move, String)> = candidates
        .iter()
        .filter(|(_, san)| normalise(san) == wanted)
        .collect();
    if matches.is_empty() {
        matches = candidates
            .iter()
            .filter(|(mv, _)| {
                piece_move_spellings(pos, *mv).is_some_and(|forms| forms.contains(&wanted))
            })
            .collect();
    }
    if matches.is_empty() {
        return missing_promotion(&candidates, &wanted, text);
    }

    match matches.as_slice() {
        [(mv, _)] => Ok(*mv),
        many => Err(ambiguous(many.iter().map(|(_, san)| san.clone()))),
    }
}

/// Step 6: the promotion moves `wanted` names once their piece is left out. One pawn move
/// is [`MoveTextError::NeedsPromotion`]; none is [`MoveTextError::NotLegal`].
fn missing_promotion(
    candidates: &[(Move, String)],
    wanted: &str,
    text: &str,
) -> Result<Move, MoveTextError> {
    let without_piece = |spelling: &str| {
        let mut spelling = spelling.to_string();
        spelling.pop();
        spelling
    };
    let matches: Vec<&(Move, String)> = candidates
        .iter()
        .filter(|(mv, san)| {
            mv.promotion().is_some()
                && (without_piece(&normalise(san)) == wanted
                    || without_piece(&mv.to_uci()) == wanted)
        })
        .collect();
    let Some((first, _)) = matches.first() else {
        return Err(MoveTextError::NotLegal(text.to_string()));
    };
    let (from, to) = (first.from(), first.to());
    if matches
        .iter()
        .all(|(mv, _)| mv.from() == from && mv.to() == to)
    {
        Err(MoveTextError::NeedsPromotion { from, to })
    } else {
        Err(ambiguous(matches.iter().map(|(_, san)| san.clone())))
    }
}

/// [`MoveTextError::Ambiguous`] with the SANs sorted.
fn ambiguous(sans: impl Iterator<Item = String>) -> MoveTextError {
    let mut sans: Vec<String> = sans.collect();
    sans.sort();
    MoveTextError::Ambiguous(sans)
}

/// Step 3: normalised text read as UCI, optionally after a piece letter that must name the
/// piece on the origin square (`ng1f3`), and that must match `named` when there is one.
fn long_algebraic(pos: &ChessPosition, wanted: &str, named: Option<PieceKind>) -> Option<Move> {
    let kind_of = |mv: Move| pos.piece_at(mv.from()).map(|piece| piece.kind);
    if let Ok(mv) = pos.parse_uci(wanted)
        && named.is_none_or(|kind| kind_of(mv) == Some(kind))
    {
        return Some(mv);
    }
    let mut chars = wanted.chars();
    let letter = PieceKind::from_char(chars.next()?)?;
    let mv = pos.parse_uci(chars.as_str()).ok()?;
    (kind_of(mv) == Some(letter) && named.is_none_or(|kind| kind == letter)).then_some(mv)
}

/// The piece an uppercase leading letter names: `N`, `R`, `Q` and `K` always, `B` only when
/// a letter follows (after any `x` or `-`), because `B4` is the b-pawn typed in capitals.
fn named_piece(text: &str) -> Option<PieceKind> {
    let mut chars = text.chars();
    let first = chars.next()?;
    let kind = match first {
        'N' => PieceKind::Knight,
        'B' => PieceKind::Bishop,
        'R' => PieceKind::Rook,
        'Q' => PieceKind::Queen,
        'K' => PieceKind::King,
        _ => return None,
    };
    if kind == PieceKind::Bishop {
        let next = chars.find(|c| !matches!(c, 'x' | 'X' | '-'))?;
        if !next.is_ascii_alphabetic() {
            return None;
        }
    }
    Some(kind)
}

/// `text` without a trailing en passant mark (`exd6 e.p.`, any case), unless that is all
/// there is.
fn strip_en_passant_mark(text: &str) -> &str {
    const MARK: &str = "e.p.";
    let lower = text.to_ascii_lowercase();
    match lower.strip_suffix(MARK) {
        // ASCII lowercasing keeps byte offsets, so `rest.len()` indexes `text` too.
        Some(rest) if !rest.trim_end().is_empty() => text[..rest.len()].trim_end(),
        _ => text,
    }
}

/// Loose comparison key: ASCII-lowercased, `x - + # = ! ?` dropped, `0` read as `o`
/// (castling).
fn normalise(text: &str) -> String {
    text.chars()
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| !matches!(c, 'x' | '-' | '+' | '#' | '=' | '!' | '?'))
        .map(|c| if c == '0' { 'o' } else { c })
        .collect()
}

/// Every normalised spelling of a non-pawn, non-castling move, from no disambiguation to
/// full: `nd2`, `nbd2`, `n1d2`, `nb1d2`. `None` for pawn moves and castling.
fn piece_move_spellings(pos: &ChessPosition, mv: Move) -> Option<[String; 4]> {
    if mv.is_castle() {
        return None;
    }
    let piece = pos.piece_at(mv.from())?;
    if piece.kind == PieceKind::Pawn {
        return None;
    }
    let letter = piece.kind.to_char();
    let from = mv.from().to_string();
    let (file, rank) = from.split_at(1);
    let to = mv.to();
    Some([
        format!("{letter}{to}"),
        format!("{letter}{file}{to}"),
        format!("{letter}{rank}{to}"),
        format!("{letter}{from}{to}"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::START_FEN;

    const KIWIPETE: &str = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
    /// White pawn b2 and bishop e1 can both take the knight on c3.
    const PAWN_OR_BISHOP_TAKES: &str = "6k1/8/8/8/8/2n5/1P6/4B1K1 w - - 0 1";
    /// Black pawn b4 can take c4 en passant on c3 and the bishop e5 can go to c3.
    const PAWN_EP_OR_BISHOP: &str = "4k3/8/8/4b3/1pP5/8/8/7K b - c3 0 1";
    /// Knights on b1 and f1 can both reach d2.
    const TWO_KNIGHTS: &str = "7k/8/8/8/8/8/8/1N1K1N2 w - - 0 1";
    const CASTLING: &str = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
    const PROMOTION: &str = "8/4P3/8/8/8/8/8/k6K w - - 0 1";
    /// Italian-style position where the knight on f3 can take the pawn on e5.
    const KNIGHT_TAKES: &str = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    /// Black to move; `Qh4` is mate.
    const FOOLS_MATE: &str = "rnbqkbnr/pppp1ppp/8/4p3/6P1/5P2/PPPPP2P/RNBQKBNR b KQkq - 0 2";

    fn uci(fen: &str, text: &str) -> Result<String, MoveTextError> {
        let pos = ChessPosition::from_fen(fen).expect("test FEN is valid");
        parse_move(&pos, text).map(Move::to_uci)
    }

    fn ok(s: &str) -> Result<String, MoveTextError> {
        Ok(s.to_string())
    }

    fn ambiguous(sans: &[&str]) -> Result<String, MoveTextError> {
        Err(MoveTextError::Ambiguous(
            sans.iter().map(|s| s.to_string()).collect(),
        ))
    }

    #[test]
    fn strict_san() {
        assert_eq!(uci(START_FEN, "e4"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "Nf3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "  Nc3 \n"), ok("b1c3"));
    }

    #[test]
    fn loose_san_is_case_insensitive() {
        assert_eq!(uci(START_FEN, "nf3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "NF3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "E4"), ok("e2e4"));
    }

    #[test]
    fn uci_any_case() {
        assert_eq!(uci(START_FEN, "e2e4"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "E2E4"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "G1f3"), ok("g1f3"));
    }

    #[test]
    fn promotions() {
        assert_eq!(uci(PROMOTION, "e7e8q"), ok("e7e8q"));
        assert_eq!(uci(PROMOTION, "E7E8N"), ok("e7e8n"));
        assert_eq!(uci(PROMOTION, "e8=Q"), ok("e7e8q"));
        assert_eq!(uci(PROMOTION, "e8=R"), ok("e7e8r"));
        assert_eq!(uci(PROMOTION, "e8q"), ok("e7e8q"));
        assert_eq!(uci(PROMOTION, "e8N"), ok("e7e8n"));
        assert_eq!(uci(PROMOTION, "e8=b"), ok("e7e8b"));
        assert_eq!(uci(PROMOTION, "e7e8=q"), ok("e7e8q"));
    }

    #[test]
    fn a_promotion_without_its_piece_asks_for_one() {
        let needs = |from: &str, to: &str| {
            Err(MoveTextError::NeedsPromotion {
                from: from.parse().expect("square"),
                to: to.parse().expect("square"),
            })
        };
        for text in ["e8", "E8", "e7e8", "E7E8", "e7-e8", "e8+", "e8="] {
            assert_eq!(uci(PROMOTION, text), needs("e7", "e8"), "{text}");
        }
        // A capture that promotes, next to the push.
        const TAKE_OR_PUSH: &str = "3r3k/4P3/8/8/8/8/8/K7 w - - 0 1";
        assert_eq!(uci(TAKE_OR_PUSH, "exd8"), needs("e7", "d8"));
        assert_eq!(uci(TAKE_OR_PUSH, "e7d8"), needs("e7", "d8"));
        assert_eq!(uci(TAKE_OR_PUSH, "e8"), needs("e7", "e8"));
        assert_eq!(uci(TAKE_OR_PUSH, "exd8=R"), ok("e7d8r"));
        // A named piece is never a pawn, and pawns off the seventh rank do not promote.
        assert_eq!(
            uci(PROMOTION, "Ne8"),
            Err(MoveTextError::NotLegal("Ne8".to_string()))
        );
        assert_eq!(
            uci(START_FEN, "e8"),
            Err(MoveTextError::NotLegal("e8".to_string()))
        );
    }

    #[test]
    fn castling_spellings() {
        for text in ["O-O", "0-0", "o-o", "O-O+", "OO", "oo", "00"] {
            assert_eq!(uci(CASTLING, text), ok("e1g1"), "{text}");
        }
        for text in ["O-O-O", "0-0-0", "o-o-o", "OOO", "ooo", "000"] {
            assert_eq!(uci(CASTLING, text), ok("e1c1"), "{text}");
        }
        assert_eq!(uci(CASTLING, "e1g1"), ok("e1g1"));
        // Castling is not a king move to g1 in SAN.
        assert_eq!(
            uci(CASTLING, "Kg1"),
            Err(MoveTextError::NotLegal("Kg1".to_string()))
        );
    }

    #[test]
    fn capture_marker_is_optional() {
        assert_eq!(uci(KNIGHT_TAKES, "Nxe5"), ok("f3e5"));
        assert_eq!(uci(KNIGHT_TAKES, "Ne5"), ok("f3e5"));
        assert_eq!(uci(KNIGHT_TAKES, "nxe5"), ok("f3e5"));
        assert_eq!(uci(KNIGHT_TAKES, "NXE5"), ok("f3e5"));
    }

    #[test]
    fn a_capital_b_names_the_bishop_and_a_small_b_may_be_either() {
        assert_eq!(uci(PAWN_OR_BISHOP_TAKES, "bxc3"), ok("b2c3"));
        assert_eq!(uci(PAWN_OR_BISHOP_TAKES, "Bxc3"), ok("e1c3"));
        for text in ["Bc3", "BC3", "BXC3", "B-c3"] {
            assert_eq!(uci(PAWN_OR_BISHOP_TAKES, text), ok("e1c3"), "{text}");
        }
        assert_eq!(
            uci(PAWN_OR_BISHOP_TAKES, "bc3"),
            ambiguous(&["Bxc3", "bxc3"])
        );
    }

    #[test]
    fn a_named_piece_is_never_another_piece() {
        // The f1 bishop is blocked, so only the b3 pawn can reach c4.
        const BLOCKED_BISHOP: &str = "rnbqkbnr/pppp1ppp/8/8/2p5/1P6/P1PPPPPP/RNBQKBNR w KQkq - 0 3";
        for text in ["Bc4", "Bxc4", "BXC4", "Bfc4", "Bf1c4"] {
            assert_eq!(
                uci(BLOCKED_BISHOP, text),
                Err(MoveTextError::NotLegal(text.to_string())),
                "{text}"
            );
        }
        for text in ["bxc4", "bc4", "b3c4", "b3xc4"] {
            assert_eq!(uci(BLOCKED_BISHOP, text), ok("b3c4"), "{text}");
        }
        // A capital B before a digit is the b-pawn typed in capitals.
        assert_eq!(uci(START_FEN, "B4"), ok("b2b4"));
        assert_eq!(uci(START_FEN, "B2B4"), ok("b2b4"));
        // Other capitals only ever named pieces.
        assert_eq!(
            uci(START_FEN, "Qh5"),
            Err(MoveTextError::NotLegal("Qh5".to_string()))
        );
    }

    #[test]
    fn long_algebraic_with_separators_and_suffixes() {
        assert_eq!(uci(START_FEN, "e2-e4"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "E2-E4"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "e2e4+"), ok("e2e4"));
        assert_eq!(uci(START_FEN, "Ng1-f3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "ng1f3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "Ng1xf3"), ok("g1f3"));
        assert_eq!(uci(KNIGHT_TAKES, "Nf3xe5"), ok("f3e5"));
        assert_eq!(uci(KNIGHT_TAKES, "f3xe5"), ok("f3e5"));
        assert_eq!(uci(PROMOTION, "e7-e8=Q"), ok("e7e8q"));
        // The letter must name the piece that moves.
        for text in ["Bg1-f3", "Qg1f3", "Ne2-e4"] {
            assert_eq!(
                uci(START_FEN, text),
                Err(MoveTextError::NotLegal(text.to_string())),
                "{text}"
            );
        }
    }

    #[test]
    fn en_passant_spellings() {
        const EN_PASSANT: &str = "4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1";
        for text in [
            "exd6",
            "exd6 e.p.",
            "exd6e.p.",
            "EXD6 E.P.",
            "e5xd6",
            "e5d6",
            "ed6",
        ] {
            assert_eq!(uci(EN_PASSANT, text), ok("e5d6"), "{text}");
        }
        assert_eq!(
            uci(EN_PASSANT, "e.p."),
            Err(MoveTextError::NotLegal("e.p.".to_string()))
        );
    }

    #[test]
    fn en_passant_or_bishop_matches_the_spec_message() {
        assert_eq!(uci(PAWN_EP_OR_BISHOP, "bxc3"), ok("b4c3"));
        assert_eq!(uci(PAWN_EP_OR_BISHOP, "Bc3"), ok("e5c3"));
        let pos = ChessPosition::from_fen(PAWN_EP_OR_BISHOP).expect("valid FEN");
        let err = parse_move(&pos, "bc3").expect_err("both moves match");
        assert_eq!(err.to_string(), "ambiguous: Bc3, bxc3");
    }

    #[test]
    fn under_disambiguated_piece_move_is_ambiguous() {
        assert_eq!(uci(TWO_KNIGHTS, "Nd2"), ambiguous(&["Nbd2", "Nfd2"]));
        assert_eq!(uci(TWO_KNIGHTS, "nd2"), ambiguous(&["Nbd2", "Nfd2"]));
        assert_eq!(uci(TWO_KNIGHTS, "N1d2"), ambiguous(&["Nbd2", "Nfd2"]));
        assert_eq!(uci(TWO_KNIGHTS, "Nbd2"), ok("b1d2"));
        assert_eq!(uci(TWO_KNIGHTS, "nfd2"), ok("f1d2"));
    }

    #[test]
    fn over_disambiguated_piece_move_is_accepted() {
        assert_eq!(uci(TWO_KNIGHTS, "Nb1d2"), ok("b1d2"));
        assert_eq!(uci(TWO_KNIGHTS, "Nf1xd2"), ok("f1d2"));
        assert_eq!(uci(START_FEN, "Ngf3"), ok("g1f3"));
        assert_eq!(uci(START_FEN, "Ng1f3"), ok("g1f3"));
        // Disambiguation that names the wrong origin is still illegal.
        assert_eq!(
            uci(START_FEN, "Nbf3"),
            Err(MoveTextError::NotLegal("Nbf3".to_string()))
        );
    }

    #[test]
    fn check_and_annotation_suffixes() {
        for text in ["Qh4#", "Qh4+", "Qh4", "qh4", "Qh4#!", "Qh4?!", "d8h4"] {
            assert_eq!(uci(FOOLS_MATE, text), ok("d8h4"), "{text}");
        }
        assert_eq!(uci(START_FEN, "e4!"), ok("e2e4"));
    }

    #[test]
    fn garbage_is_not_legal() {
        for text in [
            "hello", "Ke3", "e5", "e2e5", "e4 e5", "é4", "♞f3", "z9", "e2e4q",
        ] {
            assert_eq!(
                uci(START_FEN, text),
                Err(MoveTextError::NotLegal(text.to_string())),
                "{text}"
            );
        }
    }

    #[test]
    fn empty_input() {
        assert_eq!(uci(START_FEN, ""), Err(MoveTextError::Empty));
        assert_eq!(uci(START_FEN, " \t\n "), Err(MoveTextError::Empty));
    }

    #[test]
    fn error_messages() {
        assert_eq!(MoveTextError::Empty.to_string(), "type a move");
        assert_eq!(
            MoveTextError::NotLegal("Ke3".to_string()).to_string(),
            "not a legal move: Ke3"
        );
        assert_eq!(
            MoveTextError::Ambiguous(vec!["Bxc3".to_string(), "bxc3".to_string()]).to_string(),
            "ambiguous: Bxc3, bxc3"
        );
        let needs = MoveTextError::NeedsPromotion {
            from: "e7".parse().expect("square"),
            to: "e8".parse().expect("square"),
        };
        assert_eq!(needs.to_string(), "choose the promotion piece for e7e8");
        // The error echoes the trimmed input and never the position's FEN.
        let pos = ChessPosition::startpos();
        let err = parse_move(&pos, "  nxe5  ").expect_err("no capture on e5");
        assert_eq!(err.to_string(), "not a legal move: nxe5");
        assert!(!err.to_string().contains('/'));
    }

    #[test]
    fn long_input_is_echoed_only_in_part() {
        // A FEN pasted as a move: the error keeps it on one line.
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        let err = parse_move(&ChessPosition::startpos(), fen).expect_err("not a move");
        assert_eq!(err, MoveTextError::NotLegal(fen.to_string()));
        assert_eq!(
            err.to_string(),
            "not a legal move: rnbqkbnr/pppppppp/8/8/4P…"
        );
    }

    #[test]
    fn every_legal_move_round_trips() {
        for fen in [
            START_FEN,
            KIWIPETE,
            PAWN_OR_BISHOP_TAKES,
            PAWN_EP_OR_BISHOP,
            TWO_KNIGHTS,
            CASTLING,
            PROMOTION,
        ] {
            let pos = ChessPosition::from_fen(fen).expect("valid FEN");
            for &mv in pos.legal_moves().iter() {
                let san = pos.to_san(mv);
                assert_eq!(parse_move(&pos, &san), Ok(mv), "{fen}: {san}");
                assert_eq!(parse_move(&pos, &mv.to_uci()), Ok(mv), "{fen}: {mv}");
                let upper = mv.to_uci().to_ascii_uppercase();
                assert_eq!(parse_move(&pos, &upper), Ok(mv), "{fen}: {upper}");
                // Lower-case SAN finds the move, lists it among the candidates, or is
                // itself the exact SAN of another move (`Bxc3` lower-cases to the pawn
                // capture `bxc3`, which strict SAN takes first).
                let lower = san.to_ascii_lowercase();
                match parse_move(&pos, &lower) {
                    Ok(found) if found == mv => {}
                    Ok(found) => assert_eq!(pos.to_san(found), lower, "{fen}: {san}"),
                    Err(MoveTextError::Ambiguous(sans)) => {
                        assert!(sans.contains(&san), "{fen}: {lower} -> {sans:?}");
                        assert!(sans.is_sorted(), "{fen}: {sans:?}");
                    }
                    Err(other) => panic!("{fen}: {lower} -> {other}"),
                }
            }
        }
    }
}
