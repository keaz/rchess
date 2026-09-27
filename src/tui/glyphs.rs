//! Piece glyph sets, board fillers and colour palettes (spec section 6.3), and
//! the width rules the rest of the UI measures text with.
//!
//! Every string handed out here is exactly one terminal cell wide under
//! ratatui's width rules, so the board grid can never shift. The tests at the
//! bottom enforce that, because ratatui-core does not pin `unicode-width` and a
//! future release could change a width. [`char_width`] is that same measure for
//! one character, and [`shorten`] caps user input echoed in a message.

use std::fmt;

use ratatui::style::Color;
use ratatui::text::Span;

use crate::core::{Color as Side, Piece, PieceKind};

/// The filled pawn, written with U+FE0E (text presentation).
///
/// U+265F is the only chess symbol with an emoji presentation. U+FE0F would
/// make it two cells wide in ratatui while several terminals still draw it one
/// cell wide, so it is never emitted. U+FE0E keeps it one cell everywhere and
/// helps terminals that honour it (kitty, ghostty, foot). Some terminals ignore
/// U+FE0E; this constant is the single place to drop it.
pub const SOLID_PAWN: &str = "\u{265F}\u{FE0E}";

/// Filler for every cell of a square that holds nothing.
pub const BLANK: &str = " ";

/// Marker drawn in the glyph cell of an empty legal-target square.
///
/// ASCII on purpose: `·`, `•`, `●` and `⭘` are East-Asian-Ambiguous and two
/// cells wide in CJK mode.
pub const TARGET_MARK: &str = ".";

/// Left edge of the keyboard-cursor outline.
pub const CURSOR_LEFT: &str = "[";

/// Right edge of the keyboard-cursor outline.
pub const CURSOR_RIGHT: &str = "]";

/// Every non-piece string the board widget writes into a square.
pub const FILLERS: [&str; 4] = [BLANK, TARGET_MARK, CURSOR_LEFT, CURSOR_RIGHT];

/// Ends text cut short: a long Jev note, or echoed input (see [`shorten`]).
pub const ELLIPSIS: &str = "…";

/// Characters of echoed user input that [`shorten`] keeps.
pub const ECHO_MAX_CHARS: usize = 24;

/// Environment variable that selects the starting glyph set.
pub const GLYPHS_ENV: &str = "RCHESS_GLYPHS";

/// How pieces are drawn. `g` cycles through the sets in [`GlyphSet::next`]
/// order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlyphSet {
    /// Filled glyphs `♚♛♜♝♞♟` for both sides; the foreground colour carries
    /// the side. Filled shapes cover far more of the cell than outlines, so the
    /// colour stays readable on coloured squares.
    #[default]
    Solid,
    /// Outline glyphs `♔♕♖♗♘♙` for White and filled glyphs for Black, so the
    /// shape alone tells the sides apart (useful without colour). Outlines are
    /// not used for Black because an outline glyph lets the square colour show
    /// through and cannot carry the side's colour.
    Outline,
    /// FEN letters from [`Piece::to_fen_char`]: upper case White, lower case
    /// Black. Works with any font.
    Ascii,
}

impl GlyphSet {
    /// All sets in cycling order.
    pub const ALL: [GlyphSet; 3] = [GlyphSet::Solid, GlyphSet::Outline, GlyphSet::Ascii];

    /// The next set in the cycle Solid → Outline → Ascii → Solid.
    #[must_use = "next() returns the new set; it does not change `self`"]
    pub const fn next(self) -> GlyphSet {
        match self {
            GlyphSet::Solid => GlyphSet::Outline,
            GlyphSet::Outline => GlyphSet::Ascii,
            GlyphSet::Ascii => GlyphSet::Solid,
        }
    }

    /// Lower-case name as used by `--glyphs` and `RCHESS_GLYPHS`.
    pub const fn name(self) -> &'static str {
        match self {
            GlyphSet::Solid => "solid",
            GlyphSet::Outline => "outline",
            GlyphSet::Ascii => "ascii",
        }
    }

    /// Parses a set name, ignoring ASCII case and surrounding whitespace.
    pub fn from_name(name: &str) -> Option<GlyphSet> {
        let name = name.trim();
        GlyphSet::ALL
            .into_iter()
            .find(|set| set.name().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for GlyphSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The one-cell string that draws `piece` in `set`. Never contains U+FE0F.
pub const fn glyph(set: GlyphSet, piece: Piece) -> &'static str {
    match set {
        GlyphSet::Solid => solid(piece.kind),
        GlyphSet::Outline => match piece.color {
            Side::White => outline(piece.kind),
            Side::Black => solid(piece.kind),
        },
        GlyphSet::Ascii => ascii(piece),
    }
}

const fn solid(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::King => "\u{265A}",
        PieceKind::Queen => "\u{265B}",
        PieceKind::Rook => "\u{265C}",
        PieceKind::Bishop => "\u{265D}",
        PieceKind::Knight => "\u{265E}",
        PieceKind::Pawn => SOLID_PAWN,
    }
}

const fn outline(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::King => "\u{2654}",
        PieceKind::Queen => "\u{2655}",
        PieceKind::Rook => "\u{2656}",
        PieceKind::Bishop => "\u{2657}",
        PieceKind::Knight => "\u{2658}",
        PieceKind::Pawn => "\u{2659}",
    }
}

/// Static strings for `Piece::to_fen_char` (a test checks they agree).
const fn ascii(piece: Piece) -> &'static str {
    match (piece.color, piece.kind) {
        (Side::White, PieceKind::King) => "K",
        (Side::White, PieceKind::Queen) => "Q",
        (Side::White, PieceKind::Rook) => "R",
        (Side::White, PieceKind::Bishop) => "B",
        (Side::White, PieceKind::Knight) => "N",
        (Side::White, PieceKind::Pawn) => "P",
        (Side::Black, PieceKind::King) => "k",
        (Side::Black, PieceKind::Queen) => "q",
        (Side::Black, PieceKind::Rook) => "r",
        (Side::Black, PieceKind::Bishop) => "b",
        (Side::Black, PieceKind::Knight) => "n",
        (Side::Black, PieceKind::Pawn) => "p",
    }
}

/// Board colours. All are explicit RGB or 256-colour indexes (never the 16
/// ANSI colours, which terminal themes remap).
///
/// Every background (both squares and all five tints) has a WCAG relative
/// luminance between 0.1 and 0.3, so both piece colours reach at least 3:1 on
/// every background in both palettes; a test recomputes this. The tints are
/// told apart from the squares by hue: olive, green, blue, magenta and red
/// against brown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Light squares (h1, a8, ...).
    pub light: Color,
    /// Dark squares (a1, h8, ...).
    pub dark: Color,
    /// Foreground of White's pieces.
    pub white_piece: Color,
    /// Foreground of Black's pieces, and of target dots and cursor brackets.
    pub black_piece: Color,
    /// Background of the last move's origin and destination (olive).
    pub last_move: Color,
    /// Background of the selected piece's square (green).
    pub selected: Color,
    /// Background of a legal capture target (blue).
    pub target: Color,
    /// Background of the keyboard cursor's outline columns (magenta).
    pub cursor: Color,
    /// Background of a king in check (red).
    pub check: Color,
}

/// Palette for terminals with 24-bit colour (`COLORTERM=truecolor|24bit`).
///
/// WCAG contrast of white / black pieces on each background:
/// light 3.15 / 6.67, dark 6.55 / 3.21, last move 3.38 / 6.22,
/// selected 3.35 / 6.26, target 3.78 / 5.56, cursor 4.15 / 5.06,
/// check 4.59 / 4.57.
pub const TRUECOLOR_PALETTE: Palette = Palette {
    light: Color::Rgb(0xB5, 0x88, 0x63),
    dark: Color::Rgb(0x7A, 0x56, 0x34),
    white_piece: Color::Rgb(0xFF, 0xFF, 0xFF),
    black_piece: Color::Rgb(0x00, 0x00, 0x00),
    last_move: Color::Rgb(0x94, 0x8F, 0x2A),
    selected: Color::Rgb(0x5E, 0x9B, 0x4A),
    target: Color::Rgb(0x5C, 0x86, 0xB8),
    cursor: Color::Rgb(0xB4, 0x5A, 0xB4),
    check: Color::Rgb(0xD0, 0x44, 0x3C),
};

/// Palette for 256-colour terminals (xterm cube indexes).
///
/// WCAG contrast of white / black pieces on each background:
/// light 137 (#AF875F) 3.25 / 6.45, dark 94 (#875F00) 5.73 / 3.67,
/// last move 100 (#878700) 3.82 / 5.50, selected 65 (#5F875F) 4.10 / 5.12,
/// target 67 (#5F87AF) 3.77 / 5.57, cursor 133 (#AF5FAF) 4.13 / 5.08,
/// check 160 (#D70000) 5.40 / 3.89.
pub const INDEXED_PALETTE: Palette = Palette {
    light: Color::Indexed(137),
    dark: Color::Indexed(94),
    white_piece: Color::Indexed(231),
    black_piece: Color::Indexed(16),
    last_move: Color::Indexed(100),
    selected: Color::Indexed(65),
    target: Color::Indexed(67),
    cursor: Color::Indexed(133),
    check: Color::Indexed(160),
};

/// The truecolor palette when `truecolor`, else the 256-colour one.
pub const fn palette(truecolor: bool) -> Palette {
    if truecolor {
        TRUECOLOR_PALETTE
    } else {
        INDEXED_PALETTE
    }
}

/// True when `COLORTERM` is `truecolor` or `24bit` (any case).
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn detect_truecolor(get: impl Fn(&str) -> Option<String>) -> bool {
    get("COLORTERM").is_some_and(|value| {
        let value = value.trim();
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    })
}

/// Chooses the starting glyph set and returns it with any warnings to show.
///
/// Order: the `--glyphs` value (`cli`), then `RCHESS_GLYPHS`, then Outline
/// when `NO_COLOR` is set and non-empty (the Solid set tells the sides apart
/// only by colour), else Solid. An unknown value adds a warning and falls
/// through to the next source; an empty `RCHESS_GLYPHS` counts as unset.
pub fn initial_glyphs(
    cli: Option<&str>,
    get: impl Fn(&str) -> Option<String>,
) -> (GlyphSet, Vec<String>) {
    let mut rejected: Vec<(&str, String)> = Vec::new();
    let mut chosen = None;

    if let Some(value) = cli {
        match GlyphSet::from_name(value) {
            Some(set) => chosen = Some(set),
            None => rejected.push(("--glyphs", value.to_owned())),
        }
    }
    if chosen.is_none()
        && let Some(value) = get(GLYPHS_ENV).filter(|v| !v.trim().is_empty())
    {
        match GlyphSet::from_name(&value) {
            Some(set) => chosen = Some(set),
            None => rejected.push((GLYPHS_ENV, value)),
        }
    }
    let set = chosen.unwrap_or_else(|| {
        if get("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            GlyphSet::Outline
        } else {
            GlyphSet::Solid
        }
    });

    let warnings = rejected
        .into_iter()
        .map(|(source, value)| {
            format!(
                "{source}: unknown glyph set {:?} (expected solid, outline or ascii); using {set}",
                shorten(&value)
            )
        })
        .collect();
    (set, warnings)
}

/// Cells `c` takes on screen by ratatui's measure: 1 for most characters, 2 for
/// wide ones such as `日`, 0 for combining marks and variation selectors.
pub fn char_width(c: char) -> usize {
    let mut buf = [0u8; 4];
    Span::raw(&*c.encode_utf8(&mut buf)).width()
}

/// User input to echo in a message: `text` itself when it has at most
/// [`ECHO_MAX_CHARS`] characters, else its first [`ECHO_MAX_CHARS`] and
/// [`ELLIPSIS`]. Command-line arguments, commands and move text all go through it,
/// so a pasted FEN or a stray paragraph never floods a one-line message.
pub fn shorten(text: &str) -> String {
    match text.char_indices().nth(ECHO_MAX_CHARS) {
        Some((end, _)) => format!("{}{ELLIPSIS}", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Span};

    use super::*;

    fn all_pieces() -> impl Iterator<Item = Piece> {
        Side::ALL
            .into_iter()
            .flat_map(|color| PieceKind::ALL.map(|kind| Piece::new(color, kind)))
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    /// Asserts `s` is one cell wide by ratatui's measure and in a real buffer.
    fn assert_one_cell(s: &str) {
        assert_eq!(Span::raw(s).width(), 1, "{s:?} is not one cell wide");
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf.set_string(0, 0, format!("{s}|"), Style::new());
        assert_eq!(buf[(0, 0)].symbol(), s, "{s:?} was split across cells");
        assert_eq!(
            buf[(1, 0)].symbol(),
            "|",
            "{s:?} spilled into the next cell"
        );
    }

    #[test]
    fn next_cycles_through_all_sets() {
        assert_eq!(GlyphSet::Solid.next(), GlyphSet::Outline);
        assert_eq!(GlyphSet::Outline.next(), GlyphSet::Ascii);
        assert_eq!(GlyphSet::Ascii.next(), GlyphSet::Solid);
        assert_eq!(GlyphSet::default(), GlyphSet::Solid);
    }

    #[test]
    fn names_round_trip_case_insensitively() {
        for set in GlyphSet::ALL {
            assert_eq!(GlyphSet::from_name(set.name()), Some(set));
            assert_eq!(GlyphSet::from_name(&set.name().to_uppercase()), Some(set));
            assert_eq!(set.to_string(), set.name());
        }
        assert_eq!(GlyphSet::from_name("Outline"), Some(GlyphSet::Outline));
        assert_eq!(GlyphSet::from_name(" ascii\n"), Some(GlyphSet::Ascii));
        assert_eq!(GlyphSet::from_name(""), None);
        assert_eq!(GlyphSet::from_name("fancy"), None);
        assert_eq!(GlyphSet::from_name("solidx"), None);
    }

    #[test]
    fn solid_uses_filled_glyphs_for_both_sides() {
        let expected = [
            (PieceKind::King, "\u{265A}"),
            (PieceKind::Queen, "\u{265B}"),
            (PieceKind::Rook, "\u{265C}"),
            (PieceKind::Bishop, "\u{265D}"),
            (PieceKind::Knight, "\u{265E}"),
            (PieceKind::Pawn, "\u{265F}\u{FE0E}"),
        ];
        for (kind, s) in expected {
            for color in Side::ALL {
                assert_eq!(glyph(GlyphSet::Solid, Piece::new(color, kind)), s);
            }
        }
        assert_eq!(SOLID_PAWN, "\u{265F}\u{FE0E}");
    }

    #[test]
    fn outline_is_hollow_for_white_and_filled_for_black() {
        let white = [
            "\u{2654}", "\u{2655}", "\u{2656}", "\u{2657}", "\u{2658}", "\u{2659}",
        ];
        let kinds = [
            PieceKind::King,
            PieceKind::Queen,
            PieceKind::Rook,
            PieceKind::Bishop,
            PieceKind::Knight,
            PieceKind::Pawn,
        ];
        for (kind, s) in kinds.into_iter().zip(white) {
            assert_eq!(glyph(GlyphSet::Outline, Piece::new(Side::White, kind)), s);
            assert_eq!(
                glyph(GlyphSet::Outline, Piece::new(Side::Black, kind)),
                glyph(GlyphSet::Solid, Piece::new(Side::Black, kind))
            );
        }
    }

    #[test]
    fn ascii_matches_fen_letters() {
        for piece in all_pieces() {
            assert_eq!(
                glyph(GlyphSet::Ascii, piece),
                piece.to_fen_char().to_string()
            );
        }
    }

    #[test]
    fn glyphs_differ_within_each_set() {
        for set in GlyphSet::ALL {
            let mut seen: Vec<&str> = all_pieces().map(|p| glyph(set, p)).collect();
            seen.sort_unstable();
            seen.dedup();
            let expected = if set == GlyphSet::Solid { 6 } else { 12 };
            assert_eq!(seen.len(), expected, "{set}");
        }
    }

    #[test]
    fn every_glyph_and_filler_is_one_cell_wide() {
        for set in GlyphSet::ALL {
            for piece in all_pieces() {
                let s = glyph(set, piece);
                assert!(!s.contains('\u{FE0F}'), "{s:?} contains U+FE0F");
                assert_one_cell(s);
            }
        }
        for filler in FILLERS {
            assert_one_cell(filler);
        }
        assert_one_cell(ELLIPSIS);
    }

    #[test]
    fn char_width_is_ratatuis_width() {
        for (c, width) in [('a', 1), ('é', 1), ('…', 1), ('日', 2), ('\u{FE0E}', 0)] {
            assert_eq!(char_width(c), width, "{c:?}");
            assert_eq!(char_width(c), Span::raw(c.to_string()).width(), "{c:?}");
        }
    }

    #[test]
    fn shorten_keeps_the_first_24_characters() {
        assert_eq!(shorten(""), "");
        assert_eq!(shorten("e4"), "e4");
        assert_eq!(shorten(&"é".repeat(24)), "é".repeat(24));
        assert_eq!(
            shorten(&"é".repeat(25)),
            format!("{}{ELLIPSIS}", "é".repeat(24))
        );
        assert_eq!(shorten(&"x".repeat(100)), format!("{}…", "x".repeat(24)));
    }

    /// sRGB of an explicit colour (xterm formula for 256-colour indexes).
    fn rgb(color: Color) -> (u8, u8, u8) {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(i @ 16..=231) => {
                let i = usize::from(i - 16);
                (LEVELS[i / 36], LEVELS[(i / 6) % 6], LEVELS[i % 6])
            }
            Color::Indexed(i @ 232..=255) => {
                let v = 8 + 10 * (i - 232);
                (v, v, v)
            }
            other => panic!("{other:?} is theme-dependent"),
        }
    }

    fn luminance(color: Color) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = rgb(color);
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn backgrounds(p: &Palette) -> [(&'static str, Color); 7] {
        [
            ("light", p.light),
            ("dark", p.dark),
            ("last_move", p.last_move),
            ("selected", p.selected),
            ("target", p.target),
            ("cursor", p.cursor),
            ("check", p.check),
        ]
    }

    #[test]
    fn palettes_use_the_spec_values() {
        let t = palette(true);
        assert_eq!(t.light, Color::Rgb(0xB5, 0x88, 0x63));
        assert_eq!(t.dark, Color::Rgb(0x7A, 0x56, 0x34));
        assert_eq!(t.white_piece, Color::Rgb(255, 255, 255));
        assert_eq!(t.black_piece, Color::Rgb(0, 0, 0));
        let i = palette(false);
        assert_eq!(i.light, Color::Indexed(137));
        assert_eq!(i.dark, Color::Indexed(94));
        assert_eq!(i.white_piece, Color::Indexed(231));
        assert_eq!(i.black_piece, Color::Indexed(16));
    }

    #[test]
    fn both_piece_colours_reach_3_to_1_on_every_background() {
        for p in [palette(true), palette(false)] {
            for (name, bg) in backgrounds(&p) {
                for piece in [p.white_piece, p.black_piece] {
                    let ratio = contrast(bg, piece);
                    assert!(ratio >= 3.0, "{name} {bg:?} vs {piece:?}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn backgrounds_are_distinct_and_explicit() {
        for p in [palette(true), palette(false)] {
            let bgs = backgrounds(&p);
            for (i, (a, ca)) in bgs.iter().enumerate() {
                for (b, cb) in &bgs[i + 1..] {
                    assert_ne!(ca, cb, "{a} and {b} share a colour");
                }
            }
            // rgb() panics on ANSI/named colours, which themes remap.
            for (_, c) in bgs {
                rgb(c);
            }
            rgb(p.white_piece);
            rgb(p.black_piece);
        }
    }

    #[test]
    fn truecolor_detection() {
        assert!(detect_truecolor(env(&[("COLORTERM", "truecolor")])));
        assert!(detect_truecolor(env(&[("COLORTERM", "24bit")])));
        assert!(detect_truecolor(env(&[("COLORTERM", "TrueColor ")])));
        assert!(!detect_truecolor(env(&[("COLORTERM", "256color")])));
        assert!(!detect_truecolor(env(&[("COLORTERM", "")])));
        assert!(!detect_truecolor(env(&[])));
    }

    #[test]
    fn initial_glyphs_defaults_to_solid() {
        assert_eq!(initial_glyphs(None, env(&[])), (GlyphSet::Solid, vec![]));
        assert_eq!(
            initial_glyphs(None, env(&[(GLYPHS_ENV, "  ")])),
            (GlyphSet::Solid, vec![])
        );
    }

    #[test]
    fn cli_wins_over_environment() {
        let get = env(&[(GLYPHS_ENV, "outline"), ("NO_COLOR", "1")]);
        assert_eq!(
            initial_glyphs(Some("ASCII"), get),
            (GlyphSet::Ascii, vec![])
        );
    }

    #[test]
    fn environment_used_without_cli() {
        let get = env(&[(GLYPHS_ENV, "Ascii")]);
        assert_eq!(initial_glyphs(None, get), (GlyphSet::Ascii, vec![]));
    }

    #[test]
    fn no_color_switches_to_outline_unless_chosen() {
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "1")])),
            (GlyphSet::Outline, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "")])),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(Some("solid"), env(&[("NO_COLOR", "1")])),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "1"), (GLYPHS_ENV, "solid")])),
            (GlyphSet::Solid, vec![])
        );
    }

    #[test]
    fn invalid_cli_value_warns_and_falls_back_to_environment() {
        let (set, warnings) = initial_glyphs(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")]));
        assert_eq!(set, GlyphSet::Ascii);
        assert_eq!(
            warnings,
            vec![
                "--glyphs: unknown glyph set \"fancy\" (expected solid, outline or ascii); \
                 using ascii"
            ]
        );
    }

    #[test]
    fn invalid_values_everywhere_fall_back_to_default() {
        let get = env(&[(GLYPHS_ENV, "bold"), ("NO_COLOR", "yes")]);
        let (set, warnings) = initial_glyphs(Some(""), get);
        assert_eq!(set, GlyphSet::Outline);
        assert_eq!(
            warnings,
            vec![
                "--glyphs: unknown glyph set \"\" (expected solid, outline or ascii); \
                 using outline",
                "RCHESS_GLYPHS: unknown glyph set \"bold\" (expected solid, outline or ascii); \
                 using outline",
            ]
        );
    }

    #[test]
    fn warnings_escape_and_shorten_echoed_input() {
        let (_, warnings) = initial_glyphs(Some("\u{1b}[2Jx"), env(&[]));
        assert!(warnings[0].contains(r#""\u{1b}[2Jx""#), "{}", warnings[0]);
        assert!(!warnings[0].contains('\u{1b}'));

        let long = "x".repeat(100);
        let (_, warnings) = initial_glyphs(Some(&long), env(&[]));
        assert!(warnings[0].contains(&format!("\"{}…\"", "x".repeat(24))));
    }
}
