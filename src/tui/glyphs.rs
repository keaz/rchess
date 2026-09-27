//! Piece glyph sets, board fillers and colour palettes (spec sections 6.3 and
//! 9.3), the start-up choice of a set, and the width rules the rest of the UI
//! measures text with.
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

/// Left of a legal capture target's glyph when the terminal shows no colour.
pub const CAPTURE_LEFT: &str = "(";

/// Right of a legal capture target's glyph when the terminal shows no colour.
pub const CAPTURE_RIGHT: &str = ")";

/// Either side of a king in check when the terminal shows no colour.
pub const CHECK_MARK: &str = "+";

/// Every non-piece string the board widget writes into a square.
pub const FILLERS: [&str; 7] = [
    BLANK,
    TARGET_MARK,
    CURSOR_LEFT,
    CURSOR_RIGHT,
    CAPTURE_LEFT,
    CAPTURE_RIGHT,
    CHECK_MARK,
];

/// Ends text cut short: a long Jev note, or echoed input (see [`shorten`]).
pub const ELLIPSIS: &str = "…";

/// Characters of echoed user input that [`shorten`] keeps.
pub const ECHO_MAX_CHARS: usize = 24;

/// Environment variable that selects the starting glyph set.
pub const GLYPHS_ENV: &str = "RCHESS_GLYPHS";

/// Environment variable that turns piece images off: `off` skips the graphics
/// query and leaves [`GlyphSet::Image`] out of the cycle.
pub const IMAGES_ENV: &str = "RCHESS_IMAGES";

/// How pieces are drawn. `g` cycles through the sets in [`GlyphSet::next`]
/// order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GlyphSet {
    /// Pictures of the pieces, drawn with the terminal's graphics protocol (or
    /// half-blocks without one). Offered only when the graphics query ran; where
    /// text is drawn instead (the Captured panel, squares too small for a
    /// picture) it uses the [`GlyphSet::Solid`] glyphs.
    Image,
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
    pub const ALL: [GlyphSet; 4] = [
        GlyphSet::Image,
        GlyphSet::Solid,
        GlyphSet::Outline,
        GlyphSet::Ascii,
    ];

    /// The next set in the cycle Image → Solid → Outline → Ascii → Image, or in
    /// Solid → Outline → Ascii → Solid when `images` is false (the graphics query
    /// was skipped, so there is nothing to draw pictures with).
    #[must_use = "next() returns the new set; it does not change `self`"]
    pub const fn next(self, images: bool) -> GlyphSet {
        match self {
            GlyphSet::Image => GlyphSet::Solid,
            GlyphSet::Solid => GlyphSet::Outline,
            GlyphSet::Outline => GlyphSet::Ascii,
            GlyphSet::Ascii if images => GlyphSet::Image,
            GlyphSet::Ascii => GlyphSet::Solid,
        }
    }

    /// Lower-case name as used by `--glyphs` and `RCHESS_GLYPHS`.
    pub const fn name(self) -> &'static str {
        match self {
            GlyphSet::Image => "image",
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

/// The one-cell string that draws `piece` in `set` (the Solid glyph for
/// [`GlyphSet::Image`], which draws text only where a picture does not fit).
/// Never contains U+FE0F.
pub const fn glyph(set: GlyphSet, piece: Piece) -> &'static str {
    match set {
        GlyphSet::Image | GlyphSet::Solid => solid(piece.kind),
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

/// The RGB value of a palette colour, which piece pictures are composited onto (spec
/// 9.3): an RGB colour as it is, a 256-colour index as xterm shows it by default (the 16
/// system colours, the 6×6×6 cube, the grey ramp). `None` for the named ANSI colours
/// and [`Color::Reset`], which the terminal's theme decides; the palettes never use
/// them.
pub const fn xterm_rgb(color: Color) -> Option<[u8; 3]> {
    match color {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        Color::Indexed(index) => Some(indexed_rgb(index)),
        _ => None,
    }
}

/// xterm's default RGB for colour `index` of the 256-colour palette.
const fn indexed_rgb(index: u8) -> [u8; 3] {
    /// Colours 0 to 15.
    const SYSTEM: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    /// One channel of the cube: 0, then 95 to 255 in steps of 40.
    const fn level(step: u8) -> u8 {
        if step == 0 { 0 } else { 55 + 40 * step }
    }
    match index {
        0..=15 => SYSTEM[index as usize],
        16..=231 => {
            let cube = index - 16;
            [level(cube / 36), level(cube / 6 % 6), level(cube % 6)]
        }
        232..=255 => {
            let grey = 8 + 10 * (index - 232);
            [grey, grey, grey]
        }
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

/// What the terminal can do with pictures, as far as choosing a glyph set cares
/// (see [`initial_glyphs`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageSupport {
    /// Images are off (`NO_COLOR` or `RCHESS_IMAGES=off`, or a text set was
    /// named, see [`images_wanted`]): the graphics query was skipped and
    /// [`GlyphSet::Image`] is not offered.
    Off,
    /// The query found no graphics protocol: [`GlyphSet::Image`] draws with
    /// half-blocks and is offered, but not the default.
    Halfblocks,
    /// The query found Kitty, iTerm2 or Sixel: [`GlyphSet::Image`] is the default.
    Protocol,
}

/// Chooses the starting glyph set and returns it with any warnings to show.
///
/// Order: the `--glyphs` value (`cli`), then `RCHESS_GLYPHS`, then the default:
/// Outline when `NO_COLOR` is set and non-empty (the Solid set tells the sides
/// apart only by colour), else Image when `images` found a graphics protocol,
/// else Solid. An unknown value adds a warning and falls through to the next
/// source; so does `image` when `images` is [`ImageSupport::Off`]. An empty
/// `RCHESS_GLYPHS` counts as unset.
pub fn initial_glyphs(
    cli: Option<&str>,
    get: impl Fn(&str) -> Option<String>,
    images: ImageSupport,
) -> (GlyphSet, Vec<String>) {
    /// Why a named set was not used.
    enum Refused {
        Unknown(String),
        ImagesOff,
    }
    let mut refused: Vec<(&str, Refused)> = Vec::new();
    let mut chosen = None;

    let named = [
        ("--glyphs", cli.map(str::to_owned)),
        (GLYPHS_ENV, get(GLYPHS_ENV).filter(|v| !v.trim().is_empty())),
    ];
    for (source, value) in named {
        let Some(value) = value else { continue };
        match GlyphSet::from_name(&value) {
            Some(GlyphSet::Image) if images == ImageSupport::Off => {
                refused.push((source, Refused::ImagesOff));
            }
            Some(set) => {
                chosen = Some(set);
                break;
            }
            None => refused.push((source, Refused::Unknown(value))),
        }
    }
    let set = chosen.unwrap_or_else(|| {
        if no_color(&get) {
            GlyphSet::Outline
        } else if images == ImageSupport::Protocol {
            GlyphSet::Image
        } else {
            GlyphSet::Solid
        }
    });

    // `Off` with `image` named first means one of the two switches is on.
    let off_because = if no_color(&get) {
        "NO_COLOR is set"
    } else {
        "RCHESS_IMAGES=off"
    };
    let warnings = refused
        .into_iter()
        .map(|(source, why)| match why {
            Refused::Unknown(value) => format!(
                "{source}: unknown glyph set {:?} (expected image, solid, outline or ascii); \
                 using {set}",
                shorten(&value)
            ),
            Refused::ImagesOff => {
                format!("{source}: images are off ({off_because}); using {set}")
            }
        })
        .collect();
    (set, warnings)
}

/// Whether start-up asks the terminal about graphics (spec 9.3), which also puts
/// [`GlyphSet::Image`] in the cycle. Not when `NO_COLOR` is set, not with
/// `RCHESS_IMAGES=off` ([`images_off`]), and not when the first valid set named by
/// `cli` (`--glyphs`) or `RCHESS_GLYPHS` is a text set, as [`initial_glyphs`] would
/// choose it.
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn images_wanted(cli: Option<&str>, get: impl Fn(&str) -> Option<String>) -> bool {
    if no_color(&get) || images_off(&get) {
        return false;
    }
    let named = cli
        .and_then(GlyphSet::from_name)
        .or_else(|| get(GLYPHS_ENV).and_then(|value| GlyphSet::from_name(&value)));
    named.is_none_or(|set| set == GlyphSet::Image)
}

/// True when `RCHESS_IMAGES` is `off` (any case, surrounding whitespace ignored).
/// Any other value leaves images on.
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn images_off(get: impl Fn(&str) -> Option<String>) -> bool {
    get(IMAGES_ENV).is_some_and(|value| value.trim().eq_ignore_ascii_case("off"))
}

/// True when `NO_COLOR` is set and non-empty (<https://no-color.org/>): the terminal
/// then shows no colour at all, because crossterm drops every colour it would write under
/// that same test. Attributes such as reversed and underlined still show.
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn no_color(get: impl Fn(&str) -> Option<String>) -> bool {
    get("NO_COLOR").is_some_and(|value| !value.is_empty())
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

    /// No graphics query ran: the text styles only.
    const OFF: ImageSupport = ImageSupport::Off;

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
    fn next_cycles_through_all_sets_with_images() {
        assert_eq!(GlyphSet::Image.next(true), GlyphSet::Solid);
        assert_eq!(GlyphSet::Solid.next(true), GlyphSet::Outline);
        assert_eq!(GlyphSet::Outline.next(true), GlyphSet::Ascii);
        assert_eq!(GlyphSet::Ascii.next(true), GlyphSet::Image);
        assert_eq!(GlyphSet::default(), GlyphSet::Solid);
        assert_eq!(GlyphSet::ALL[0], GlyphSet::Image, "Image comes first");
    }

    #[test]
    fn next_skips_the_image_style_without_images() {
        assert_eq!(GlyphSet::Solid.next(false), GlyphSet::Outline);
        assert_eq!(GlyphSet::Outline.next(false), GlyphSet::Ascii);
        assert_eq!(GlyphSet::Ascii.next(false), GlyphSet::Solid);
        assert_eq!(GlyphSet::Image.next(false), GlyphSet::Solid);
        // Starting anywhere, a full turn visits each text set once and never Image.
        let mut set = GlyphSet::Solid;
        let mut seen = Vec::new();
        for _ in 0..3 {
            set = set.next(false);
            seen.push(set);
        }
        assert_eq!(seen, [GlyphSet::Outline, GlyphSet::Ascii, GlyphSet::Solid]);
    }

    #[test]
    fn names_round_trip_case_insensitively() {
        for set in GlyphSet::ALL {
            assert_eq!(GlyphSet::from_name(set.name()), Some(set));
            assert_eq!(GlyphSet::from_name(&set.name().to_uppercase()), Some(set));
            assert_eq!(set.to_string(), set.name());
        }
        assert_eq!(GlyphSet::from_name("Outline"), Some(GlyphSet::Outline));
        assert_eq!(GlyphSet::from_name("image"), Some(GlyphSet::Image));
        assert_eq!(GlyphSet::Image.to_string(), "image");
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
    fn the_image_style_writes_solid_glyphs_where_it_draws_text() {
        // The Captured panel, and squares too small for a picture.
        for piece in all_pieces() {
            assert_eq!(glyph(GlyphSet::Image, piece), glyph(GlyphSet::Solid, piece));
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
            let expected = if matches!(set, GlyphSet::Solid | GlyphSet::Image) {
                6
            } else {
                12
            };
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
    fn pictures_are_composited_onto_the_xterm_value_of_each_colour() {
        // Every colour a square can have, in both palettes, has a known RGB value.
        for p in [palette(true), palette(false)] {
            for (name, bg) in backgrounds(&p) {
                let (r, g, b) = rgb(bg);
                assert_eq!(xterm_rgb(bg), Some([r, g, b]), "{name} {bg:?}");
            }
        }
        // RGB passes through; indexes get xterm's defaults, as the palette's comments give them.
        let cases = [
            (Color::Rgb(0xB5, 0x88, 0x63), [0xB5, 0x88, 0x63]),
            (Color::Indexed(137), [0xAF, 0x87, 0x5F]),
            (Color::Indexed(94), [0x87, 0x5F, 0x00]),
            (Color::Indexed(100), [0x87, 0x87, 0x00]),
            (Color::Indexed(65), [0x5F, 0x87, 0x5F]),
            (Color::Indexed(67), [0x5F, 0x87, 0xAF]),
            (Color::Indexed(133), [0xAF, 0x5F, 0xAF]),
            (Color::Indexed(160), [0xD7, 0x00, 0x00]),
            (Color::Indexed(16), [0, 0, 0]),
            (Color::Indexed(231), [255, 255, 255]),
            // The grey ramp.
            (Color::Indexed(232), [8, 8, 8]),
            (Color::Indexed(255), [238, 238, 238]),
            // The 16 system colours.
            (Color::Indexed(0), [0, 0, 0]),
            (Color::Indexed(1), [205, 0, 0]),
            (Color::Indexed(4), [0, 0, 238]),
            (Color::Indexed(7), [229, 229, 229]),
            (Color::Indexed(8), [127, 127, 127]),
            (Color::Indexed(12), [92, 92, 255]),
            (Color::Indexed(15), [255, 255, 255]),
        ];
        for (color, expected) in cases {
            assert_eq!(xterm_rgb(color), Some(expected), "{color:?}");
        }
        // Named colours and the default are the terminal theme's to decide.
        for color in [Color::Reset, Color::Red, Color::White, Color::DarkGray] {
            assert_eq!(xterm_rgb(color), None, "{color:?}");
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
        assert_eq!(
            initial_glyphs(None, env(&[]), OFF),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[(GLYPHS_ENV, "  ")]), OFF),
            (GlyphSet::Solid, vec![])
        );
    }

    #[test]
    fn cli_wins_over_environment() {
        let get = env(&[(GLYPHS_ENV, "outline"), ("NO_COLOR", "1")]);
        assert_eq!(
            initial_glyphs(Some("ASCII"), get, OFF),
            (GlyphSet::Ascii, vec![])
        );
    }

    #[test]
    fn environment_used_without_cli() {
        let get = env(&[(GLYPHS_ENV, "Ascii")]);
        assert_eq!(initial_glyphs(None, get, OFF), (GlyphSet::Ascii, vec![]));
    }

    #[test]
    fn no_color_switches_to_outline_unless_chosen() {
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "1")]), OFF),
            (GlyphSet::Outline, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "")]), OFF),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(Some("solid"), env(&[("NO_COLOR", "1")]), OFF),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[("NO_COLOR", "1"), (GLYPHS_ENV, "solid")]), OFF),
            (GlyphSet::Solid, vec![])
        );
    }

    #[test]
    fn no_color_needs_a_non_empty_value() {
        assert!(no_color(env(&[("NO_COLOR", "1")])));
        assert!(no_color(env(&[("NO_COLOR", "0")])), "any value counts");
        assert!(!no_color(env(&[("NO_COLOR", "")])));
        assert!(!no_color(env(&[])));
    }

    #[test]
    fn invalid_cli_value_warns_and_falls_back_to_environment() {
        let (set, warnings) = initial_glyphs(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")]), OFF);
        assert_eq!(set, GlyphSet::Ascii);
        assert_eq!(
            warnings,
            vec![
                "--glyphs: unknown glyph set \"fancy\" (expected image, solid, outline or ascii); \
                 using ascii"
            ]
        );
    }

    #[test]
    fn invalid_values_everywhere_fall_back_to_default() {
        let get = env(&[(GLYPHS_ENV, "bold"), ("NO_COLOR", "yes")]);
        let (set, warnings) = initial_glyphs(Some(""), get, OFF);
        assert_eq!(set, GlyphSet::Outline);
        assert_eq!(
            warnings,
            vec![
                "--glyphs: unknown glyph set \"\" (expected image, solid, outline or ascii); \
                 using outline",
                "RCHESS_GLYPHS: unknown glyph set \"bold\" (expected image, solid, outline or ascii); \
                 using outline",
            ]
        );
    }

    #[test]
    fn the_image_style_is_the_default_only_with_a_graphics_protocol() {
        assert_eq!(
            initial_glyphs(None, env(&[]), ImageSupport::Protocol),
            (GlyphSet::Image, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[]), ImageSupport::Halfblocks),
            (GlyphSet::Solid, vec![])
        );
        assert_eq!(
            initial_glyphs(None, env(&[]), OFF),
            (GlyphSet::Solid, vec![])
        );
    }

    #[test]
    fn a_named_style_wins_over_the_image_default() {
        let get = env(&[(GLYPHS_ENV, "outline")]);
        assert_eq!(
            initial_glyphs(Some("ascii"), &get, ImageSupport::Protocol),
            (GlyphSet::Ascii, vec![])
        );
        assert_eq!(
            initial_glyphs(None, &get, ImageSupport::Protocol),
            (GlyphSet::Outline, vec![])
        );
    }

    #[test]
    fn the_image_style_can_be_named_whenever_the_query_ran() {
        for images in [ImageSupport::Halfblocks, ImageSupport::Protocol] {
            assert_eq!(
                initial_glyphs(Some("image"), env(&[]), images),
                (GlyphSet::Image, vec![])
            );
            assert_eq!(
                initial_glyphs(None, env(&[(GLYPHS_ENV, "IMAGE")]), images),
                (GlyphSet::Image, vec![])
            );
        }
    }

    #[test]
    fn a_named_image_style_without_images_warns_and_falls_through() {
        assert_eq!(
            initial_glyphs(Some("image"), env(&[("NO_COLOR", "1")]), OFF),
            (
                GlyphSet::Outline,
                vec!["--glyphs: images are off (NO_COLOR is set); using outline".to_string()]
            )
        );
        assert_eq!(
            initial_glyphs(
                None,
                env(&[(GLYPHS_ENV, "image"), (IMAGES_ENV, "off")]),
                OFF
            ),
            (
                GlyphSet::Solid,
                vec!["RCHESS_GLYPHS: images are off (RCHESS_IMAGES=off); using solid".to_string()]
            )
        );
        // The environment still gets its turn after a refused --glyphs.
        let get = env(&[(GLYPHS_ENV, "ascii"), (IMAGES_ENV, "off")]);
        let (set, warnings) = initial_glyphs(Some("image"), get, OFF);
        assert_eq!(set, GlyphSet::Ascii);
        assert_eq!(
            warnings,
            ["--glyphs: images are off (RCHESS_IMAGES=off); using ascii"]
        );
    }

    #[test]
    fn images_are_wanted_unless_a_text_style_is_named() {
        assert!(images_wanted(None, env(&[])));
        assert!(images_wanted(Some("image"), env(&[])));
        assert!(images_wanted(None, env(&[(GLYPHS_ENV, "Image")])));
        assert!(
            images_wanted(None, env(&[(GLYPHS_ENV, " ")])),
            "empty is unset"
        );
        for set in ["solid", "outline", "ascii"] {
            assert!(!images_wanted(Some(set), env(&[])), "{set}");
            assert!(!images_wanted(None, env(&[(GLYPHS_ENV, set)])), "{set}");
        }
        // The first valid name counts, as in `initial_glyphs`.
        assert!(images_wanted(Some("image"), env(&[(GLYPHS_ENV, "ascii")])));
        assert!(!images_wanted(Some("ascii"), env(&[(GLYPHS_ENV, "image")])));
        assert!(!images_wanted(Some("fancy"), env(&[(GLYPHS_ENV, "ascii")])));
        assert!(images_wanted(Some("fancy"), env(&[(GLYPHS_ENV, "bold")])));
    }

    #[test]
    fn no_color_and_rchess_images_off_turn_images_off() {
        assert!(!images_wanted(None, env(&[("NO_COLOR", "1")])));
        assert!(!images_wanted(Some("image"), env(&[("NO_COLOR", "1")])));
        assert!(images_wanted(None, env(&[("NO_COLOR", "")])));
        for off in ["off", "OFF", " Off\n"] {
            let pairs = [(IMAGES_ENV, off)];
            let get = env(&pairs);
            assert!(images_off(&get), "{off:?}");
            assert!(!images_wanted(Some("image"), get), "{off:?}");
        }
        for on in ["", "on", "1", "offline"] {
            let pairs = [(IMAGES_ENV, on)];
            let get = env(&pairs);
            assert!(!images_off(&get), "{on:?}");
            assert!(images_wanted(None, get), "{on:?}");
        }
        assert!(!images_off(env(&[])));
    }

    #[test]
    fn warnings_escape_and_shorten_echoed_input() {
        let (_, warnings) = initial_glyphs(Some("\u{1b}[2Jx"), env(&[]), OFF);
        assert!(warnings[0].contains(r#""\u{1b}[2Jx""#), "{}", warnings[0]);
        assert!(!warnings[0].contains('\u{1b}'));

        let long = "x".repeat(100);
        let (_, warnings) = initial_glyphs(Some(&long), env(&[]), OFF);
        assert!(warnings[0].contains(&format!("\"{}…\"", "x".repeat(24))));
    }
}
