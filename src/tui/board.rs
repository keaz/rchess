//! Board widget and flip-aware hit-testing (spec sections 6.3, 9.2 and 9.3).
//!
//! [`layout_board`] picks a square size and places the board inside an area;
//! the resulting [`BoardGeometry`] is used both to draw ([`BoardView`]) and to
//! map mouse cells back to squares ([`square_at`]), so the App must keep the
//! geometry from its most recent draw for hit-testing.
//!
//! Squares are as tall as the area allows, and as wide as the font makes them
//! look square: [`square_width`] turns a height in rows into a width in columns
//! from the terminal's [`CellSize`].
//!
//! The widget draws only the 8×8 grid plus a rank-label column on the left and
//! a file-label row underneath; any enclosing block is the caller's.
//!
//! In the [`GlyphSet::Image`] style a piece is a picture: the Cburnett image
//! composited onto the square's colour
//! ([`composite`](super::pieces::composite)) and drawn by the terminal's
//! graphics protocol through a ratatui-image [`Picker`], in the square's
//! [`image_area`]. Squares smaller than [`min_picture_square`] for the protocol
//! ([`MIN_IMAGE_SQUARE`], or [`MIN_HALFBLOCK_SQUARE`] for half blocks) show the
//! Solid glyph instead. [`PieceImages`] keeps the encoded pictures between frames.

use std::fmt;

use image::DynamicImage;
use ratatui::{
    buffer::{Buffer, Cell},
    layout::{Position as CellPosition, Rect, Size},
    style::{Color, Modifier},
    widgets::{self, Widget},
};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::protocol::kitty::Kitty;
use ratatui_image::{Image, Resize};

use super::glyphs::{self, GlyphSet, Palette};
use super::pieces::{ImageCache, ImageKey};
use super::terminal;
use crate::core::{Color as Side, Piece, PieceKind, Position as ChessPosition, Square};

/// The smallest square `(width, height)` in cells: one row, with room for the
/// glyph and the cursor's `[` `]` on either side of it. A font so narrow that
/// even one-row squares come out too wide for the area still gets this size.
pub const MIN_SQUARE: (u16, u16) = (3, 1);

/// The smallest square `(width, height)` in cells that shows a piece as a picture
/// in the [`GlyphSet::Image`] style with a pixel protocol (Kitty, iTerm2, Sixel);
/// smaller squares show the Solid glyph. Its [`image_area`] is 3×2 cells.
pub const MIN_IMAGE_SQUARE: (u16, u16) = (5, 2);

/// The smallest square `(width, height)` in cells that shows a piece as a picture
/// drawn in half blocks, which have two pixels per cell: below an [`image_area`] of
/// 9×5 cells the pieces cannot be told apart, so smaller squares show the Solid
/// glyph.
pub const MIN_HALFBLOCK_SQUARE: (u16, u16) = (11, 5);

/// The smallest square that shows a picture drawn with `protocol`:
/// [`MIN_HALFBLOCK_SQUARE`] for half blocks, else [`MIN_IMAGE_SQUARE`].
pub fn min_picture_square(protocol: ProtocolType) -> (u16, u16) {
    match protocol {
        ProtocolType::Halfblocks => MIN_HALFBLOCK_SQUARE,
        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => MIN_IMAGE_SQUARE,
    }
}

/// A terminal cell's size in pixels, which is the font size. It makes squares
/// look square: see [`square_width`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellSize {
    width: u16,
    height: u16,
}

impl CellSize {
    /// 10×20 pixels, assumed when the terminal does not report its font size.
    pub const DEFAULT: CellSize = CellSize {
        width: 10,
        height: 20,
    };

    /// A cell `width` × `height` pixels, or [`CellSize::DEFAULT`] when either is
    /// zero (a terminal that does not know its size reports zero).
    pub const fn new(width: u16, height: u16) -> CellSize {
        if width == 0 || height == 0 {
            CellSize::DEFAULT
        } else {
            CellSize { width, height }
        }
    }

    /// Width in pixels (never zero).
    pub const fn width(self) -> u16 {
        self.width
    }

    /// Height in pixels (never zero).
    pub const fn height(self) -> u16 {
        self.height
    }
}

impl Default for CellSize {
    fn default() -> CellSize {
        CellSize::DEFAULT
    }
}

/// The width in cells of a square `square_h` rows high, so that it looks square
/// in pixels: `square_h × cell height / cell width`, rounded, at least 3, and
/// bumped up to the next odd number so a one-cell glyph sits in the middle
/// column. With the default 10×20 font that is `2 × square_h + 1`.
pub fn square_width(square_h: u16, cell: CellSize) -> u16 {
    let (cell_w, cell_h) = (u64::from(cell.width), u64::from(cell.height));
    // Rounded to the nearest whole column, halves up; u64 cannot overflow here.
    let columns = (2 * u64::from(square_h) * cell_h + cell_w) / (2 * cell_w);
    u16::try_from(columns.max(3) | 1).unwrap_or(u16::MAX)
}

/// Width of the rank-label column left of the grid.
const LABEL_COLUMNS: u16 = 1;
/// Height of the file-label row below the grid.
const LABEL_ROWS: u16 = 1;

const RANK_LABELS: [&str; 8] = ["1", "2", "3", "4", "5", "6", "7", "8"];
const FILE_LABELS: [&str; 8] = ["a", "b", "c", "d", "e", "f", "g", "h"];

/// Where the board sits on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardGeometry {
    /// Grid plus the rank-label column (left) and file-label row (bottom).
    pub outer: Rect,
    /// The 8×8 squares only: `8 * square_w` by `8 * square_h` cells.
    pub grid: Rect,
    /// Width of one square in cells: odd, at least 3 (see [`square_width`]).
    pub square_w: u16,
    /// Height of one square in cells: at least 1.
    pub square_h: u16,
    /// Black at the bottom when true; White at the bottom otherwise.
    pub flipped: bool,
}

/// Lays the board out in `area` with the tallest squares that fit (labels
/// included), each [`square_width`] wide for the font `cell`, centred in both
/// directions: a board limited by the area's width is centred vertically.
///
/// When even one-row squares are too wide for the area (a very narrow font),
/// the board uses [`MIN_SQUARE`]. Returns `None` when that does not fit either
/// (25×9 cells).
pub fn layout_board(area: Rect, flipped: bool, cell: CellSize) -> Option<BoardGeometry> {
    // Grid plus labels, in u32 so wide squares cannot overflow.
    let fits = |square_w: u16, square_h: u16| {
        8 * u32::from(square_w) + u32::from(LABEL_COLUMNS) <= u32::from(area.width)
            && 8 * u32::from(square_h) + u32::from(LABEL_ROWS) <= u32::from(area.height)
    };
    // The width grows with the height, so the first height that fits is the tallest.
    let tallest = area.height.saturating_sub(LABEL_ROWS) / 8;
    let (square_w, square_h) = (1..=tallest)
        .rev()
        .map(|square_h| (square_width(square_h, cell), square_h))
        .chain([MIN_SQUARE])
        .find(|&(square_w, square_h)| fits(square_w, square_h))?;
    // `fits` bounds both below the area's size, so these cannot overflow.
    let (grid_w, grid_h) = (8 * square_w, 8 * square_h);
    let (outer_w, outer_h) = (grid_w + LABEL_COLUMNS, grid_h + LABEL_ROWS);
    let x = area.x + (area.width - outer_w) / 2;
    let y = area.y + (area.height - outer_h) / 2;
    Some(BoardGeometry {
        outer: Rect::new(x, y, outer_w, outer_h),
        grid: Rect::new(x + LABEL_COLUMNS, y, grid_w, grid_h),
        square_w,
        square_h,
        flipped,
    })
}

/// The cells covered by `sq`.
pub fn square_rect(g: &BoardGeometry, sq: Square) -> Rect {
    let (column, row) = if g.flipped {
        (7 - sq.file(), sq.rank())
    } else {
        (sq.file(), 7 - sq.rank())
    };
    // Saturating so a hand-built geometry can never panic a draw.
    let dx = u16::from(column).saturating_mul(g.square_w);
    let dy = u16::from(row).saturating_mul(g.square_h);
    Rect::new(
        g.grid.x.saturating_add(dx),
        g.grid.y.saturating_add(dy),
        g.square_w,
        g.square_h,
    )
}

/// The square under terminal cell (`column`, `row`), or `None` outside the
/// grid (labels included). Exact inverse of [`square_rect`].
pub fn square_at(g: &BoardGeometry, column: u16, row: u16) -> Option<Square> {
    if g.square_w == 0 || g.square_h == 0 || !g.grid.contains(CellPosition::new(column, row)) {
        return None;
    }
    let grid_column = u8::try_from((column - g.grid.x) / g.square_w).ok()?;
    let grid_row = u8::try_from((row - g.grid.y) / g.square_h).ok()?;
    let (file, rank) = if g.flipped {
        (7u8.checked_sub(grid_column)?, grid_row)
    } else {
        (grid_column, 7u8.checked_sub(grid_row)?)
    };
    Square::from_file_rank(file, rank)
}

/// True for light squares (h1, a8, ...); a1 is dark.
pub const fn is_light(sq: Square) -> bool {
    (sq.file() + sq.rank()) % 2 == 1
}

/// Where a picture of the piece on `square` goes: the square without its leftmost
/// and rightmost columns, which stay text for the cursor's `[` `]` and the marks
/// drawn without colour. `None` for a square smaller than [`MIN_IMAGE_SQUARE`].
pub fn image_area(square: Rect) -> Option<Rect> {
    let (min_width, min_height) = MIN_IMAGE_SQUARE;
    (square.width >= min_width && square.height >= min_height)
        .then(|| Rect::new(square.x + 1, square.y, square.width - 2, square.height))
}

/// The largest picture built, in pixels per side. Real image areas are a few hundred
/// pixels; a bigger one could only come from a bogus font size, and would cost
/// seconds and hundreds of megabytes on the UI thread.
const MAX_PICTURE_PX: u32 = 4096;

/// Piece pictures kept between frames for the [`GlyphSet::Image`] style: the
/// ratatui-image [`Protocol`] a picker made of each
/// [`composite`](super::pieces::composite), one per [`ImageKey`], so a picture is
/// scaled and encoded once and a piece that moves to a square of the same colour
/// reuses it. A picture the picker could not encode is kept as `None`, and its
/// square shows the glyph.
///
/// Every picture on a board has the same image area size. When a frame needs
/// another size, or the picker another font or protocol, the pictures are dropped
/// first (the squares or the font changed), so the cache holds at most one entry
/// per piece and square colour in use.
#[derive(Default)]
pub struct PieceImages {
    cache: ImageCache<Option<Protocol>>,
    /// What the pictures in `cache` were made for.
    made_for: Option<PictureFormat>,
}

/// Everything a board's pictures share, besides the piece and the colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PictureFormat {
    /// The image area in cells.
    cells: (u16, u16),
    /// The picker's font size in pixels.
    font: (u16, u16),
    protocol: ProtocolType,
}

impl PieceImages {
    /// No pictures yet.
    pub fn new() -> PieceImages {
        PieceImages::default()
    }

    /// Number of pictures kept.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// True when no picture is kept.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Drops every picture, as when the font changed.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.made_for = None;
    }

    /// The picture of `piece` on `background` (RGB) for an image area of `cells`,
    /// drawn by `picker`: made on first use, `None` when the picker cannot encode it
    /// or it would be over [`MAX_PICTURE_PX`] on a side.
    fn picture(
        &mut self,
        picker: &Picker,
        piece: Piece,
        background: [u8; 3],
        cells: Size,
    ) -> Option<&Protocol> {
        let font = picker.font_size();
        let format = PictureFormat {
            cells: (cells.width, cells.height),
            font: (font.width, font.height),
            protocol: picker.protocol_type(),
        };
        if self.made_for != Some(format) {
            self.cache.clear();
            self.made_for = Some(format);
        }
        // The composite is exactly the area's size in pixels, so the picker encodes it
        // as it is, without scaling it again.
        let key = ImageKey::new(
            piece,
            background,
            u32::from(cells.width) * u32::from(font.width),
            u32::from(cells.height) * u32::from(font.height),
        );
        if key.width_px > MAX_PICTURE_PX || key.height_px > MAX_PICTURE_PX {
            return None;
        }
        self.cache
            .get_or_insert_with(key, |key| {
                let image = DynamicImage::ImageRgba8(key.composite());
                if picker.protocol_type() == ProtocolType::Kitty {
                    kitty_picture(image, cells, picker.tmux_detected())
                } else {
                    picker.new_protocol(image, cells, Resize::Fit(None)).ok()
                }
            })
            .as_ref()
    }
}

/// A Kitty picture of `image`, which is exactly the pixel size of `cells`, sent
/// through tmux when `tmux`. It is what `Picker::new_protocol` builds, except that
/// the image id comes from [`terminal::next_kitty_id`] instead of a random number:
/// Kitty pictures are virtual placements, which the terminal deletes only by id, so
/// the restore must know every id sent. The transmission is not compressed, as the
/// graphics query never asks whether the terminal can inflate it.
fn kitty_picture(image: DynamicImage, cells: Size, tmux: bool) -> Option<Protocol> {
    let id = terminal::next_kitty_id(tmux);
    Kitty::new(image, cells, id, tmux, false)
        .ok()
        .map(Protocol::Kitty)
}

impl fmt::Debug for PieceImages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Protocols hold encoded image data, and are not `Debug`.
        f.debug_struct("PieceImages")
            .field("len", &self.len())
            .field("made_for", &self.made_for)
            .finish_non_exhaustive()
    }
}

/// The cell that holds a square's glyph or target mark: the middle column
/// and, for even heights, the upper of the two middle rows.
fn glyph_cell(rect: Rect) -> CellPosition {
    CellPosition::new(
        rect.x + rect.width / 2,
        rect.y + rect.height.saturating_sub(1) / 2,
    )
}

/// What to mark on the board.
///
/// Square background precedence: selected, then check, then capture target,
/// then last move, then the plain square colour. Selection wins over check so
/// that picking up a king in check still shows it picked up (the Status panel
/// says "Check" too). The cursor outline is drawn on top of all of them.
///
/// Without colour ([`BoardView::no_color`]) the tints show nothing, so the
/// selected square is reversed, the last move's squares are underlined, a
/// capture target gets `(` `)` and a king in check `+` `+` beside its glyph
/// (the cursor's `[` `]` still win there).
///
/// A picture ([`GlyphSet::Image`]) is composited onto the square's background,
/// highlight tints included.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlights {
    /// Origin and destination of the last move played (tinted).
    pub last_move: Option<(Square, Square)>,
    /// Square of the piece picked up by click or Enter (tinted).
    pub selected: Option<Square>,
    /// Legal destinations of the selected piece: a centred `.` on empty
    /// squares, a tinted background on captures (en passant included, whose
    /// destination is empty).
    pub targets: Vec<Square>,
    /// Keyboard cursor, drawn as a bracket outline `[ ]`.
    pub cursor: Option<Square>,
    /// King in check (red).
    pub check: Option<Square>,
}

/// The board widget. Render it with any area: it draws into
/// `geometry.outer` and clips to the area it is given.
///
/// In the [`GlyphSet::Image`] style with a [`picker`](Self::picker), pieces are
/// pictures. Render it as a [`StatefulWidget`](widgets::StatefulWidget) with
/// [`PieceImages`] to keep them between frames; as a plain [`Widget`] every
/// picture is made afresh. A picture is drawn only when its whole image area is
/// inside the area rendered to and clear of the [`overlays`](Self::overlays), so
/// nothing is drawn over it.
#[derive(Clone, Copy, Debug)]
pub struct BoardView<'a> {
    /// Position to draw.
    pub position: &'a ChessPosition,
    /// Layout from [`layout_board`] (also used for hit-testing).
    pub geometry: BoardGeometry,
    /// Glyph set for the pieces.
    pub glyphs: GlyphSet,
    /// Square, piece and highlight colours.
    pub palette: &'a Palette,
    /// Marks to draw.
    pub highlights: &'a Highlights,
    /// The terminal shows no colour (`NO_COLOR`, see [`glyphs::no_color`]): mark
    /// the highlights with text and attributes as well as tints.
    pub no_color: bool,
    /// Draws the pieces as pictures in the [`GlyphSet::Image`] style; without one
    /// that style shows the Solid glyphs.
    pub picker: Option<&'a Picker>,
    /// Boxes drawn over the board after it (a dialog, the game-over box). A piece
    /// whose image area meets one shows its glyph instead of its picture while the
    /// box is up: a box would wipe the kitty picture's one-time transmission, or
    /// leave its text on an iTerm2 or Sixel picture whose first cell it missed
    /// (only that cell is ever sent). When the box closes, the picture is drawn
    /// and sent whole.
    pub overlays: &'a [Rect],
}

impl Widget for BoardView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &BoardView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        widgets::StatefulWidget::render(self, area, buf, &mut PieceImages::new());
    }
}

impl widgets::StatefulWidget for BoardView<'_> {
    type State = PieceImages;

    fn render(self, area: Rect, buf: &mut Buffer, images: &mut PieceImages) {
        widgets::StatefulWidget::render(&self, area, buf, images);
    }
}

impl widgets::StatefulWidget for &BoardView<'_> {
    type State = PieceImages;

    fn render(self, area: Rect, buf: &mut Buffer, images: &mut PieceImages) {
        let clip = area.intersection(*buf.area());
        if clip.is_empty() {
            return;
        }
        let grid = self.geometry.grid;
        for pos in self.geometry.outer.positions() {
            if !grid.contains(pos)
                && let Some(cell) = cell_in(buf, clip, pos)
            {
                cell.set_symbol(glyphs::BLANK);
            }
        }
        for sq in Square::all() {
            self.render_square(sq, clip, buf, images);
        }
        self.render_labels(clip, buf);
    }
}

impl BoardView<'_> {
    fn render_square(&self, sq: Square, clip: Rect, buf: &mut Buffer, images: &mut PieceImages) {
        let rect = square_rect(&self.geometry, sq);
        let piece = self.position.piece_at(sq);
        let is_target = self.highlights.targets.contains(&sq);
        let is_capture = is_target && (piece.is_some() || self.is_en_passant_target(sq));
        let bg = self.background(sq, is_capture);
        let modifier = self.modifier(sq);
        for pos in rect.positions() {
            if let Some(cell) = cell_in(buf, clip, pos) {
                cell.reset();
                cell.set_bg(bg);
                cell.modifier = modifier;
            }
        }
        let picture = piece.and_then(|piece| self.picture(piece, rect, bg, clip, images));
        let pictured = picture.is_some();
        if let Some((area, protocol)) = picture {
            Image::new(protocol).render(area, buf);
        } else if let Some(cell) = cell_in(buf, clip, glyph_cell(rect)) {
            if let Some(piece) = piece {
                let fg = match piece.color {
                    Side::White => self.palette.white_piece,
                    Side::Black => self.palette.black_piece,
                };
                cell.set_symbol(glyphs::glyph(self.glyphs, piece))
                    .set_fg(fg);
            } else if is_target && !is_capture {
                cell.set_symbol(glyphs::TARGET_MARK)
                    .set_fg(self.palette.black_piece);
            }
        }
        let cursor = self.highlights.cursor == Some(sq);
        if cursor {
            self.render_cursor(rect, clip, buf);
        }
        if let Some((left, right)) = self.side_marks(sq, is_capture) {
            // A picture leaves no room between the cursor's brackets.
            let inside = rect.width >= 5 && !pictured;
            self.render_side_marks(rect, (left, right), cursor, inside, clip, buf);
        }
    }

    /// The picture of `piece` on the square at `rect`, whose background is `bg`, and
    /// the area it goes in. Only in the Image style with a picker, on a square of at
    /// least [`min_picture_square`] for its protocol whose image area lies wholly inside `clip` and meets
    /// none of the [`overlays`](Self::overlays), and on a background with a known RGB
    /// value ([`glyphs::xterm_rgb`]); `None` otherwise, and the square shows the glyph.
    fn picture<'i>(
        &self,
        piece: Piece,
        rect: Rect,
        bg: Color,
        clip: Rect,
        images: &'i mut PieceImages,
    ) -> Option<(Rect, &'i Protocol)> {
        if self.glyphs != GlyphSet::Image {
            return None;
        }
        let picker = self.picker?;
        let (min_width, min_height) = min_picture_square(picker.protocol_type());
        if rect.width < min_width || rect.height < min_height {
            return None;
        }
        let area = image_area(rect).filter(|&area| {
            clip.intersection(area) == area
                && !self.overlays.iter().any(|overlay| overlay.intersects(area))
        })?;
        let background = glyphs::xterm_rgb(bg)?;
        let protocol = images.picture(picker, piece, background, area.as_size())?;
        Some((area, protocol))
    }

    /// True when `sq` is the en passant square and the selected piece is a
    /// pawn, so moving there captures the pawn beside it.
    fn is_en_passant_target(&self, sq: Square) -> bool {
        self.position.ep_square() == Some(sq)
            && self
                .highlights
                .selected
                .and_then(|from| self.position.piece_at(from))
                .is_some_and(|piece| piece.kind == PieceKind::Pawn)
    }

    fn background(&self, sq: Square, capture_target: bool) -> Color {
        let (h, p) = (self.highlights, self.palette);
        if h.selected == Some(sq) {
            p.selected
        } else if h.check == Some(sq) {
            p.check
        } else if capture_target {
            p.target
        } else if h.last_move.is_some_and(|(from, to)| sq == from || sq == to) {
            p.last_move
        } else if is_light(sq) {
            p.light
        } else {
            p.dark
        }
    }

    /// Without colour: reversed for the selected square, underlined for the
    /// last move's squares. With colour the tints are enough.
    fn modifier(&self, sq: Square) -> Modifier {
        let h = self.highlights;
        if !self.no_color {
            Modifier::empty()
        } else if h.selected == Some(sq) {
            Modifier::REVERSED
        } else if h.last_move.is_some_and(|(from, to)| sq == from || sq == to) {
            Modifier::UNDERLINED
        } else {
            Modifier::empty()
        }
    }

    /// Without colour: the marks beside the glyph of a capture target or a king
    /// in check (a king is never a capture target).
    fn side_marks(&self, sq: Square, capture_target: bool) -> Option<(&'static str, &'static str)> {
        if !self.no_color {
            None
        } else if capture_target {
            Some((glyphs::CAPTURE_LEFT, glyphs::CAPTURE_RIGHT))
        } else if self.highlights.check == Some(sq) {
            Some((glyphs::CHECK_MARK, glyphs::CHECK_MARK))
        } else {
            None
        }
    }

    /// Writes the `(left, right)` marks in the square's outer columns on the glyph row. Under
    /// the cursor, whose `[` `]` hold the outer columns, they go just inside them (`[(p)]`)
    /// when `inside` says there is room; a square too narrow for both, or with a picture
    /// in its middle, keeps the cursor's `[` and the mark's right side (`[p)`), so the
    /// cursor never hides the mark.
    fn render_side_marks(
        &self,
        rect: Rect,
        (left, right): (&'static str, &'static str),
        cursor: bool,
        inside: bool,
        clip: Rect,
        buf: &mut Buffer,
    ) {
        if rect.width < 3 {
            return;
        }
        let (first, last) = (rect.left(), rect.right() - 1);
        let columns = match (cursor, inside) {
            (false, _) => [Some((first, left)), Some((last, right))],
            (true, true) => [Some((first + 1, left)), Some((last - 1, right))],
            (true, false) => [None, Some((last, right))],
        };
        let y = glyph_cell(rect).y;
        for (x, mark) in columns.into_iter().flatten() {
            if let Some(cell) = cell_in(buf, clip, CellPosition::new(x, y)) {
                cell.set_symbol(mark).set_fg(self.palette.black_piece);
            }
        }
    }

    /// Tints the square's left and right columns and puts `[` `]` on the
    /// glyph row, leaving the middle (and its highlight) visible.
    fn render_cursor(&self, rect: Rect, clip: Rect, buf: &mut Buffer) {
        if rect.width < 3 {
            return;
        }
        let glyph_row = glyph_cell(rect).y;
        let edges = [
            (rect.left(), glyphs::CURSOR_LEFT),
            (rect.right() - 1, glyphs::CURSOR_RIGHT),
        ];
        for y in rect.top()..rect.bottom() {
            for (x, mark) in edges {
                if let Some(cell) = cell_in(buf, clip, CellPosition::new(x, y)) {
                    cell.set_bg(self.palette.cursor);
                    if y == glyph_row {
                        cell.set_symbol(mark).set_fg(self.palette.black_piece);
                    }
                }
            }
        }
    }

    /// Rank numbers left of each rank's glyph row, file letters under each
    /// file's glyph column; both follow the flip.
    fn render_labels(&self, clip: Rect, buf: &mut Buffer) {
        let g = &self.geometry;
        if let Some(label_x) = g.grid.x.checked_sub(LABEL_COLUMNS) {
            // a1, a2, ..., a8: one square per rank.
            for sq in Square::all().step_by(8) {
                let y = glyph_cell(square_rect(g, sq)).y;
                if let Some(cell) = cell_in(buf, clip, CellPosition::new(label_x, y)) {
                    cell.set_symbol(RANK_LABELS[usize::from(sq.rank())]);
                }
            }
        }
        let label_y = g.grid.bottom();
        // a1, b1, ..., h1: one square per file.
        for sq in Square::all().take(8) {
            let x = glyph_cell(square_rect(g, sq)).x;
            if let Some(cell) = cell_in(buf, clip, CellPosition::new(x, label_y)) {
                cell.set_symbol(FILE_LABELS[usize::from(sq.file())]);
            }
        }
    }
}

/// The buffer cell at `pos` if it lies inside `clip`.
fn cell_in(buf: &mut Buffer, clip: Rect, pos: CellPosition) -> Option<&mut Cell> {
    if clip.contains(pos) {
        buf.cell_mut(pos)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use image::DynamicImage;
    use ratatui::{Terminal, backend::TestBackend, layout::Size, style::Modifier, text::Span};
    use ratatui_image::picker::{Picker, ProtocolType};
    use ratatui_image::{Image, Resize};

    use super::*;
    use crate::core::Piece;
    use crate::tui::glyphs::{SOLID_PAWN, palette, xterm_rgb};
    use crate::tui::graphics::picker_for;
    use crate::tui::pieces::composite;
    use crate::tui::test_support::sq;

    /// Square sizes the default font gives for heights 1 to 4.
    const SIZES: [(u16, u16); 4] = [(3, 1), (5, 2), (7, 3), (9, 4)];

    fn geometry(square_w: u16, square_h: u16, x: u16, y: u16, flipped: bool) -> BoardGeometry {
        let area = Rect::new(x, y, 8 * square_w + 1, 8 * square_h + 1);
        let g = layout_board(area, flipped, CellSize::DEFAULT).expect("exact-fit area");
        assert_eq!((g.square_w, g.square_h), (square_w, square_h));
        g
    }

    /// Renders `position` into a fresh TestBackend of `width`×`height`.
    fn draw(
        width: u16,
        height: u16,
        position: &ChessPosition,
        glyph_set: GlyphSet,
        flipped: bool,
        highlights: &Highlights,
    ) -> (Terminal<TestBackend>, BoardGeometry) {
        draw_marked(
            width, height, position, glyph_set, flipped, highlights, false,
        )
    }

    /// [`draw`] with [`BoardView::no_color`] set as given.
    fn draw_marked(
        width: u16,
        height: u16,
        position: &ChessPosition,
        glyph_set: GlyphSet,
        flipped: bool,
        highlights: &Highlights,
        no_color: bool,
    ) -> (Terminal<TestBackend>, BoardGeometry) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
        let pal = palette(true);
        let mut saved = None;
        terminal
            .draw(|frame| {
                let area = frame.area();
                let geometry = layout_board(area, flipped, CellSize::DEFAULT).expect("board fits");
                saved = Some(geometry);
                frame.render_widget(
                    BoardView {
                        position,
                        geometry,
                        glyphs: glyph_set,
                        palette: &pal,
                        highlights,
                        no_color,
                        picker: None,
                        overlays: &[],
                    },
                    area,
                );
            })
            .expect("draw");
        (terminal, saved.expect("drawn"))
    }

    fn cells(buf: &Buffer, rect: Rect) -> Vec<&Cell> {
        rect.positions().map(|p| &buf[p]).collect()
    }

    #[test]
    fn cell_size_defaults_to_ten_by_twenty() {
        assert_eq!(CellSize::default(), CellSize::DEFAULT);
        assert_eq!(
            (CellSize::DEFAULT.width(), CellSize::DEFAULT.height()),
            (10, 20)
        );
        let font = CellSize::new(9, 19);
        assert_eq!((font.width(), font.height()), (9, 19));
        // A terminal that does not know its font size reports zero.
        for (width, height) in [(0, 0), (0, 20), (10, 0)] {
            assert_eq!(CellSize::new(width, height), CellSize::DEFAULT);
        }
    }

    #[test]
    fn square_width_makes_squares_look_square() {
        // (font, widths for square heights 1 to 8)
        let cases = [
            // Twice as tall as wide: 2h, bumped to odd.
            ((10, 20), [3, 5, 7, 9, 11, 13, 15, 17]),
            ((8, 16), [3, 5, 7, 9, 11, 13, 15, 17]),
            ((16, 32), [3, 5, 7, 9, 11, 13, 15, 17]),
            // 2.5: 2.5 rounds to 3, 7.5 to 8 and then 9.
            ((10, 25), [3, 5, 9, 11, 13, 15, 19, 21]),
            // 15/7 = 2.14: 4.29 rounds to 4 then 5, 8.57 to 9.
            ((7, 15), [3, 5, 7, 9, 11, 13, 15, 17]),
            // 1.8: 3.6 rounds to 4 then 5, 5.4 to 5, 7.2 to 7.
            ((10, 18), [3, 5, 5, 7, 9, 11, 13, 15]),
            // Square cells: at least 3 columns.
            ((12, 12), [3, 3, 3, 5, 5, 7, 7, 9]),
            // Very tall cells: one row already needs 5 columns.
            ((5, 20), [5, 9, 13, 17, 21, 25, 29, 33]),
        ];
        for ((cell_w, cell_h), widths) in cases {
            let cell = CellSize::new(cell_w, cell_h);
            for (square_h, width) in (1..=8).zip(widths) {
                assert_eq!(
                    square_width(square_h, cell),
                    width,
                    "{cell_w}x{cell_h} font, {square_h} rows"
                );
            }
        }
        // Huge values saturate instead of overflowing.
        assert_eq!(square_width(u16::MAX, CellSize::new(1, u16::MAX)), u16::MAX);
        assert_eq!(square_width(0, CellSize::DEFAULT), 3);
    }

    #[test]
    fn layout_picks_the_tallest_square_that_fits_and_centres_it() {
        // (area, font, square size, outer rect)
        let cases = [
            ((80, 24), (10, 20), (5, 2), Rect::new(19, 3, 41, 17)),
            ((120, 40), (10, 20), (9, 4), Rect::new(23, 3, 73, 33)),
            ((200, 60), (10, 20), (15, 7), Rect::new(39, 1, 121, 57)),
            ((60, 20), (10, 20), (5, 2), Rect::new(9, 1, 41, 17)),
            ((40, 16), (10, 20), (3, 1), Rect::new(7, 3, 25, 9)),
            ((25, 9), (10, 20), (3, 1), Rect::new(0, 0, 25, 9)),
            // Limited by width: shorter squares, centred vertically.
            ((73, 60), (10, 20), (9, 4), Rect::new(0, 13, 73, 33)),
            ((74, 60), (10, 20), (9, 4), Rect::new(0, 13, 73, 33)),
            ((80, 40), (10, 25), (9, 3), Rect::new(3, 7, 73, 25)),
            // A taller font needs more columns per square, a wider one fewer.
            ((120, 40), (10, 25), (11, 4), Rect::new(15, 3, 89, 33)),
            ((80, 40), (12, 12), (5, 4), Rect::new(19, 3, 41, 33)),
        ];
        for ((width, height), (cell_w, cell_h), size, outer) in cases {
            let at = format!("{width}x{height} with a {cell_w}x{cell_h} font");
            let g = layout_board(
                Rect::new(0, 0, width, height),
                false,
                CellSize::new(cell_w, cell_h),
            )
            .unwrap_or_else(|| panic!("{at} should fit"));
            assert_eq!((g.square_w, g.square_h), size, "{at}");
            assert_eq!(g.outer, outer, "{at}");
            assert_eq!(
                g.grid,
                Rect::new(outer.x + 1, outer.y, 8 * size.0, 8 * size.1),
                "{at}"
            );
        }
    }

    #[test]
    fn a_very_narrow_font_still_gets_the_smallest_squares() {
        // One-row squares would be 5 columns wide (41 with labels): too wide for 30.
        let tall = CellSize::new(5, 20);
        let g = layout_board(Rect::new(0, 0, 30, 20), false, tall).expect("fits");
        assert_eq!((g.square_w, g.square_h), MIN_SQUARE);
        assert_eq!(g.outer, Rect::new(2, 5, 25, 9));
        // With the room, the font's own width wins.
        let g = layout_board(Rect::new(0, 0, 41, 20), false, tall).expect("fits");
        assert_eq!((g.square_w, g.square_h), (5, 1));
    }

    #[test]
    fn layout_is_none_when_too_small() {
        for cell in [
            CellSize::DEFAULT,
            CellSize::new(5, 20),
            CellSize::new(20, 10),
        ] {
            for (width, height) in [(24, 9), (25, 8), (0, 0), (100, 8), (24, 100)] {
                assert_eq!(
                    layout_board(Rect::new(0, 0, width, height), false, cell),
                    None
                );
            }
        }
    }

    #[test]
    fn layout_respects_the_area_origin_and_flip() {
        let g = layout_board(Rect::new(10, 5, 26, 10), true, CellSize::DEFAULT).expect("fits");
        assert_eq!(g.outer, Rect::new(10, 5, 25, 9));
        assert_eq!(g.grid, Rect::new(11, 5, 24, 8));
        assert!(g.flipped);
    }

    #[test]
    fn square_rect_and_square_at_round_trip() {
        for (w, h) in SIZES {
            for flipped in [false, true] {
                for (x, y) in [(0, 0), (7, 4)] {
                    let g = geometry(w, h, x, y, flipped);
                    let mut covered = 0;
                    for sq in Square::all() {
                        let rect = square_rect(&g, sq);
                        assert_eq!((rect.width, rect.height), (w, h));
                        assert_eq!(rect.intersection(g.grid), rect, "{sq} outside grid");
                        for pos in rect.positions() {
                            assert_eq!(
                                square_at(&g, pos.x, pos.y),
                                Some(sq),
                                "{w}x{h} flipped={flipped} at {pos:?}"
                            );
                            covered += 1;
                        }
                    }
                    assert_eq!(covered, g.grid.area(), "squares tile the grid");
                }
            }
        }
    }

    #[test]
    fn white_is_at_the_bottom_unless_flipped() {
        for (w, h) in SIZES {
            let g = geometry(w, h, 0, 0, false);
            let bottom = g.grid.bottom() - 1;
            let right = g.grid.right() - 1;
            assert_eq!(square_at(&g, g.grid.x, bottom), Some(Square::A1));
            assert_eq!(square_at(&g, right, g.grid.y), Some(Square::H8));
            assert_eq!(square_at(&g, g.grid.x, g.grid.y), Some(Square::A8));

            let g = geometry(w, h, 0, 0, true);
            assert_eq!(square_at(&g, g.grid.x, g.grid.y), Some(Square::H1));
            assert_eq!(square_at(&g, right, g.grid.y), Some(Square::A1));
            assert_eq!(square_at(&g, g.grid.x, bottom), Some(Square::H8));
            assert_eq!(square_at(&g, right, bottom), Some(Square::A8));
        }
    }

    #[test]
    fn clicks_outside_the_grid_miss() {
        for (w, h) in SIZES {
            for flipped in [false, true] {
                let g = geometry(w, h, 3, 2, flipped);
                let grid = g.grid;
                let misses = [
                    (grid.x - 1, grid.y),          // rank-label column
                    (grid.x, grid.bottom()),       // file-label row
                    (grid.right(), grid.y),        // right of the grid
                    (grid.x, grid.y - 1),          // above the grid
                    (grid.right(), grid.bottom()), // diagonal corner
                    (0, 0),                        // top-left of the screen
                    (u16::MAX, u16::MAX),          // crossterm's wrapped 0;0 report
                ];
                for (column, row) in misses {
                    assert_eq!(square_at(&g, column, row), None, "({column}, {row})");
                }
            }
        }
    }

    #[test]
    fn degenerate_geometry_never_panics() {
        let zero = BoardGeometry {
            outer: Rect::ZERO,
            grid: Rect::ZERO,
            square_w: 0,
            square_h: 0,
            flipped: false,
        };
        assert_eq!(square_at(&zero, 0, 0), None);
        // A grid larger than 8×8 squares maps its extra cells to nothing.
        let oversized = BoardGeometry {
            grid: Rect::new(0, 0, 40, 10),
            square_w: 3,
            square_h: 1,
            ..zero
        };
        assert_eq!(square_at(&oversized, 30, 0), None);
        assert_eq!(square_at(&oversized, 0, 9), None);
    }

    #[test]
    fn square_colours_follow_the_board_pattern() {
        assert!(!is_light(Square::A1));
        assert!(is_light(Square::H1));
        assert!(is_light(Square::A8));
        assert!(!is_light(Square::H8));
        assert!(is_light(sq("e2")));
        assert!(!is_light(sq("d2")));
    }

    #[test]
    fn labels_are_one_cell_wide() {
        for label in RANK_LABELS.iter().chain(&FILE_LABELS) {
            assert_eq!(Span::raw(*label).width(), 1);
        }
    }

    #[test]
    fn start_position_3x1_snapshot() {
        let (terminal, g) = draw(
            31,
            11,
            &ChessPosition::startpos(),
            GlyphSet::Solid,
            false,
            &Highlights::default(),
        );
        assert_eq!((g.square_w, g.square_h), (3, 1));
        insta::assert_snapshot!("start_3x1", terminal.backend());
    }

    #[test]
    fn start_position_3x1_flipped_snapshot() {
        let (terminal, _) = draw(
            31,
            11,
            &ChessPosition::startpos(),
            GlyphSet::Solid,
            true,
            &Highlights::default(),
        );
        insta::assert_snapshot!("start_3x1_flipped", terminal.backend());
    }

    #[test]
    fn start_position_5x2_ascii_snapshot() {
        let (terminal, g) = draw(
            41,
            17,
            &ChessPosition::startpos(),
            GlyphSet::Ascii,
            false,
            &Highlights::default(),
        );
        assert_eq!((g.square_w, g.square_h), (5, 2));
        insta::assert_snapshot!("start_5x2_ascii", terminal.backend());
    }

    #[test]
    fn e2_is_a_light_square_with_a_white_pawn() {
        let pal = palette(true);
        let (terminal, g) = draw(
            31,
            11,
            &ChessPosition::startpos(),
            GlyphSet::Solid,
            false,
            &Highlights::default(),
        );
        let buf = terminal.backend().buffer();
        let e2 = square_rect(&g, sq("e2"));
        assert_eq!(e2, Rect::new(16, 7, 3, 1));
        for cell in cells(buf, e2) {
            assert_eq!(cell.bg, pal.light);
        }
        let pawn = &buf[(17, 7)];
        assert_eq!(pawn.symbol(), SOLID_PAWN);
        assert_eq!(pawn.fg, pal.white_piece);
        assert_eq!(buf[(16, 7)].symbol(), " ");
        assert_eq!(buf[(18, 7)].symbol(), " ");

        // Neighbours alternate, and a black piece is drawn in black.
        let d2 = square_rect(&g, sq("d2"));
        assert!(cells(buf, d2).iter().all(|c| c.bg == pal.dark));
        let e7 = glyph_cell(square_rect(&g, sq("e7")));
        assert_eq!(buf[e7].symbol(), SOLID_PAWN);
        assert_eq!(buf[e7].fg, pal.black_piece);
    }

    #[test]
    fn labels_follow_the_flip() {
        for flipped in [false, true] {
            let (terminal, g) = draw(
                25,
                9,
                &ChessPosition::startpos(),
                GlyphSet::Solid,
                flipped,
                &Highlights::default(),
            );
            let buf = terminal.backend().buffer();
            let ranks: String = (0..8).map(|y| buf[(0, y)].symbol().to_owned()).collect();
            let files: String = (0..8)
                .map(|i| buf[(g.grid.x + 1 + 3 * i, 8)].symbol().to_owned())
                .collect();
            if flipped {
                assert_eq!((ranks.as_str(), files.as_str()), ("12345678", "hgfedcba"));
            } else {
                assert_eq!((ranks.as_str(), files.as_str()), ("87654321", "abcdefgh"));
            }
        }
    }

    #[test]
    fn highlights_mark_the_right_cells() {
        let pal = palette(true);
        // 1.e4 e5 2.Nf3 d5, White's knight on f3 picked up.
        let position = ChessPosition::from_fen(
            "rnbqkbnr/ppp2ppp/8/3pp3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 0 3",
        )
        .expect("valid FEN");
        let targets: Vec<Square> = position
            .legal_moves()
            .iter()
            .filter(|m| m.from() == sq("f3"))
            .map(|m| m.to())
            .collect();
        assert!(targets.contains(&sq("e5")) && targets.contains(&sq("g5")));
        let highlights = Highlights {
            last_move: Some((sq("d7"), sq("d5"))),
            selected: Some(sq("f3")),
            targets,
            cursor: Some(sq("c3")),
            check: None,
        };
        let (terminal, g) = draw(31, 11, &position, GlyphSet::Solid, false, &highlights);
        let buf = terminal.backend().buffer();
        let rect = |name: &str| square_rect(&g, sq(name));
        let all_bg = |name: &str, bg: Color| cells(buf, rect(name)).iter().all(|c| c.bg == bg);

        // Selected knight: tinted, glyph still white.
        assert!(all_bg("f3", pal.selected));
        let f3 = &buf[glyph_cell(rect("f3"))];
        assert_eq!((f3.symbol(), f3.fg), ("\u{265E}", pal.white_piece));

        // Capture target: tinted background, black pawn kept.
        assert!(all_bg("e5", pal.target));
        let e5 = &buf[glyph_cell(rect("e5"))];
        assert_eq!((e5.symbol(), e5.fg), (SOLID_PAWN, pal.black_piece));

        // Empty target: plain square with a centred dot.
        assert!(all_bg("g5", pal.dark));
        let g5 = &buf[glyph_cell(rect("g5"))];
        assert_eq!((g5.symbol(), g5.fg), (".", pal.black_piece));
        assert!(all_bg("h4", pal.dark));
        assert_eq!(buf[glyph_cell(rect("h4"))].symbol(), ".");

        // Last move: both ends tinted, the piece on the destination kept.
        assert!(all_bg("d7", pal.last_move));
        assert!(all_bg("d5", pal.last_move));
        assert_eq!(buf[glyph_cell(rect("d5"))].symbol(), SOLID_PAWN);

        // Cursor on empty dark c3: bracket outline, middle keeps the square.
        let c3 = rect("c3");
        let [left, middle, right] = [0, 1, 2].map(|dx| &buf[(c3.x + dx, c3.y)]);
        assert_eq!(
            (left.symbol(), left.bg, left.fg),
            ("[", pal.cursor, pal.black_piece)
        );
        assert_eq!((right.symbol(), right.bg), ("]", pal.cursor));
        assert_eq!((middle.symbol(), middle.bg), (" ", pal.dark));

        // Untouched squares keep their colour and no stray marks appear.
        assert!(all_bg("h3", pal.light));
        assert_eq!(buf[glyph_cell(rect("h3"))].symbol(), " ");
        assert!(all_bg("e4", pal.light));
    }

    #[test]
    fn selection_wins_over_check_and_the_cursor_outlines_tall_squares() {
        let pal = palette(true);
        // Fool's mate: 1.f3 e5 2.g4 Qh4#.
        let position = ChessPosition::from_fen(
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
        )
        .expect("valid FEN");
        let mut highlights = Highlights {
            last_move: Some((sq("d8"), sq("h4"))),
            selected: Some(sq("e1")),
            targets: vec![sq("h4")],
            cursor: Some(sq("e1")),
            check: Some(sq("e1")),
        };
        let (terminal, g) = draw(41, 17, &position, GlyphSet::Solid, false, &highlights);
        assert_eq!((g.square_w, g.square_h), (5, 2));
        let buf = terminal.backend().buffer();

        // The king in check is picked up: the selection tint shows, inside the cursor.
        let e1 = square_rect(&g, sq("e1"));
        for pos in e1.positions() {
            let cell = &buf[pos];
            let edge = pos.x == e1.left() || pos.x == e1.right() - 1;
            assert_eq!(
                cell.bg,
                if edge { pal.cursor } else { pal.selected },
                "{pos:?}"
            );
        }
        // Glyph and brackets on the upper row; the lower row is blank.
        let top: String = (0..5).map(|dx| buf[(e1.x + dx, e1.y)].symbol()).collect();
        let bottom: String = (0..5)
            .map(|dx| buf[(e1.x + dx, e1.y + 1)].symbol())
            .collect();
        assert_eq!(top, "[ \u{265A} ]");
        assert_eq!(bottom, "     ");
        assert_eq!(buf[(e1.x + 2, e1.y)].fg, pal.white_piece);

        // The capture tint beats the last-move tint on h4.
        let h4 = square_rect(&g, sq("h4"));
        assert!(cells(buf, h4).iter().all(|c| c.bg == pal.target));
        // The other end of the last move keeps its tint.
        let d8 = square_rect(&g, sq("d8"));
        assert!(cells(buf, d8).iter().all(|c| c.bg == pal.last_move));

        // Put down again, the king shows the check colour.
        highlights.selected = None;
        highlights.targets.clear();
        highlights.cursor = None;
        let (terminal, g) = draw(41, 17, &position, GlyphSet::Solid, false, &highlights);
        let e1 = square_rect(&g, sq("e1"));
        let buf = terminal.backend().buffer();
        assert!(cells(buf, e1).iter().all(|c| c.bg == pal.check));
    }

    #[test]
    fn without_colour_the_highlights_are_marked_with_text_and_attributes() {
        // NO_COLOR: the terminal drops every colour, so tints alone would show nothing.
        // Fool's mate with the checking queen as the last move; White's king picked up.
        let position = ChessPosition::from_fen(
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
        )
        .expect("valid FEN");
        let highlights = Highlights {
            last_move: Some((sq("d8"), sq("h4"))),
            selected: Some(sq("g1")),
            targets: vec![sq("h3"), sq("h4")],
            cursor: None,
            check: Some(sq("e1")),
        };
        let (terminal, g) = draw_marked(
            31,
            11,
            &position,
            GlyphSet::Outline,
            false,
            &highlights,
            true,
        );
        let buf = terminal.backend().buffer();
        let row = |name: &str| -> String {
            let rect = square_rect(&g, sq(name));
            (0..rect.width)
                .map(|dx| buf[(rect.x + dx, rect.y)].symbol())
                .collect()
        };
        let all = |name: &str, modifier: Modifier| {
            cells(buf, square_rect(&g, sq(name)))
                .iter()
                .all(|c| c.modifier.contains(modifier))
        };

        // The selected piece is drawn reversed.
        assert!(all("g1", Modifier::REVERSED));
        assert_eq!(row("g1"), " \u{2658} ");
        // A capture target is bracketed in round brackets, not the cursor's square ones.
        assert_eq!(row("h4"), "(\u{265B})");
        // A quiet target keeps its dot.
        assert_eq!(row("h3"), " . ");
        // Both ends of the last move are underlined.
        assert!(all("d8", Modifier::UNDERLINED));
        assert!(all("h4", Modifier::UNDERLINED));
        // The king in check is flanked by `+`, as in SAN.
        assert_eq!(row("e1"), "+\u{2654}+");
        // Nothing else is marked.
        assert_eq!(row("a2"), " \u{2659} ");
        assert!(
            cells(buf, square_rect(&g, sq("a2")))
                .iter()
                .all(|c| c.modifier.is_empty())
        );

        // With colours the marks are the tints alone, as before.
        let (terminal, g) = draw_marked(
            31,
            11,
            &position,
            GlyphSet::Outline,
            false,
            &highlights,
            false,
        );
        let buf = terminal.backend().buffer();
        let rect = square_rect(&g, sq("h4"));
        let symbols: String = (0..3)
            .map(|dx| buf[(rect.x + dx, rect.y)].symbol())
            .collect();
        assert_eq!(symbols, " \u{265B} ");
        assert!(buf.content().iter().all(|c| c.modifier.is_empty()));
    }

    #[test]
    fn without_colour_the_cursor_never_hides_a_capture_or_check_mark() {
        // Black has just played d7-d5 beside White's e5 pawn: d6 is an empty capture
        // target (en passant), so its marks are all that says "capture".
        let position =
            ChessPosition::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").expect("valid FEN");
        let at = |cursor: &str, selected: Option<&str>, check: Option<&str>| Highlights {
            selected: selected.map(sq),
            targets: if selected.is_some() {
                vec![sq("d6"), sq("e6")]
            } else {
                Vec::new()
            },
            cursor: Some(sq(cursor)),
            check: check.map(sq),
            ..Highlights::default()
        };
        // (board width, height, capture under the cursor, king in check under it,
        // plain cursor, quiet target under the cursor), for each square size.
        let cases = [
            (31, 11, "[ )", "[\u{2654}+", "[ ]", "[.]"),
            (41, 17, "[( )]", "[+\u{2654}+]", "[   ]", "[ . ]"),
            (57, 25, "[(   )]", "[+ \u{2654} +]", "[     ]", "[  .  ]"),
        ];
        for (width, height, capture, check, plain, quiet) in cases {
            let row = |highlights: &Highlights, name: &str| -> String {
                let (terminal, g) = draw_marked(
                    width,
                    height,
                    &position,
                    GlyphSet::Outline,
                    false,
                    highlights,
                    true,
                );
                let buf = terminal.backend().buffer();
                let rect = square_rect(&g, sq(name));
                let y = glyph_cell(rect).y;
                (0..rect.width)
                    .map(|dx| buf[(rect.x + dx, y)].symbol())
                    .collect()
            };
            let size = format!("{width}x{height}");
            assert_eq!(row(&at("d6", Some("e5"), None), "d6"), capture, "{size}");
            assert_eq!(row(&at("e1", None, Some("e1")), "e1"), check, "{size}");
            assert_eq!(row(&at("a1", None, None), "a1"), plain, "{size}");
            assert_eq!(row(&at("e6", Some("e5"), None), "e6"), quiet, "{size}");
            // Away from the cursor the marks stay in the outer columns.
            let away = row(&at("a1", Some("e5"), None), "d6");
            assert!(
                away.starts_with('(') && away.ends_with(')'),
                "{size}: {away}"
            );
        }
    }

    #[test]
    fn an_en_passant_target_is_tinted_like_a_capture() {
        let pal = palette(true);
        // Black has just played d7-d5 beside White's e5 pawn.
        let position =
            ChessPosition::from_fen("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1").expect("valid FEN");
        let highlights = Highlights {
            selected: Some(sq("e5")),
            targets: vec![sq("d6"), sq("e6")],
            ..Highlights::default()
        };
        let (terminal, g) = draw(31, 11, &position, GlyphSet::Solid, false, &highlights);
        let buf = terminal.backend().buffer();
        let d6 = square_rect(&g, sq("d6"));
        assert!(cells(buf, d6).iter().all(|c| c.bg == pal.target));
        assert_eq!(buf[glyph_cell(d6)].symbol(), " ", "no dot on a capture");
        let e6 = square_rect(&g, sq("e6"));
        assert_eq!(
            buf[glyph_cell(e6)].symbol(),
            ".",
            "the push is a quiet move"
        );

        // The en passant square is only a capture for a pawn: the b4 bishop just moves there.
        let position =
            ChessPosition::from_fen("4k3/8/8/3pP3/1B6/8/8/4K3 w - d6 0 1").expect("valid FEN");
        let highlights = Highlights {
            selected: Some(sq("b4")),
            targets: vec![sq("c5"), sq("d6")],
            ..Highlights::default()
        };
        let (terminal, g) = draw(31, 11, &position, GlyphSet::Solid, false, &highlights);
        let buf = terminal.backend().buffer();
        let d6 = square_rect(&g, sq("d6"));
        assert_eq!(buf[glyph_cell(d6)].symbol(), ".");
        assert!(cells(buf, d6).iter().all(|c| c.bg == pal.dark));
    }

    #[test]
    fn other_glyph_sets_render() {
        let (terminal, g) = draw(
            25,
            9,
            &ChessPosition::startpos(),
            GlyphSet::Outline,
            false,
            &Highlights::default(),
        );
        let buf = terminal.backend().buffer();
        assert_eq!(
            buf[glyph_cell(square_rect(&g, Square::E1))].symbol(),
            "\u{2654}"
        );
        assert_eq!(
            buf[glyph_cell(square_rect(&g, Square::E8))].symbol(),
            "\u{265A}"
        );
    }

    #[test]
    fn rendering_clips_to_the_given_area() {
        let pal = palette(false);
        let position = ChessPosition::startpos();
        let highlights = Highlights {
            cursor: Some(Square::H8),
            ..Highlights::default()
        };
        // A 7×3 geometry drawn into a much smaller area and buffer.
        let geometry = geometry(7, 3, 0, 0, false);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 6));
        let view = BoardView {
            position: &position,
            geometry,
            glyphs: GlyphSet::Solid,
            palette: &pal,
            highlights: &highlights,
            no_color: false,
            picker: None,
            overlays: &[],
        };
        view.render(Rect::new(0, 0, 10, 5), &mut buf);
        assert_eq!(buf[(9, 1)].bg, pal.dark, "b8 is dark");
        assert_eq!(buf[(10, 1)], Cell::EMPTY, "outside the area is untouched");
        assert_eq!(buf[(0, 5)], Cell::EMPTY);

        // A geometry entirely outside the buffer draws nothing.
        let far = geometry_at(200, 100);
        let mut small = Buffer::empty(Rect::new(0, 0, 5, 5));
        BoardView {
            geometry: far,
            ..view
        }
        .render(Rect::new(0, 0, 5, 5), &mut small);
        assert_eq!(small, Buffer::empty(Rect::new(0, 0, 5, 5)));
    }

    fn geometry_at(x: u16, y: u16) -> BoardGeometry {
        geometry(3, 1, x, y, false)
    }

    // ----- pictures (the Image style) -----

    /// Kings on e1 (dark) and e8 (light) and White's knight on g1 (dark).
    const KINGS_AND_KNIGHT: &str = "4k3/8/8/8/8/8/8/4K1N1 w - - 0 1";
    /// Black's rook on a1 (dark) checks White's king on e1 (dark).
    const ROOK_CHECK: &str = "4k3/8/8/8/8/8/8/r3K3 w - - 0 1";

    const WHITE_KING: Piece = Piece::new(Side::White, PieceKind::King);

    fn fen(fen: &str) -> ChessPosition {
        ChessPosition::from_fen(fen).expect("valid FEN")
    }

    fn halfblocks(font: CellSize) -> Picker {
        picker_for(ProtocolType::Halfblocks, font)
    }

    /// A board in the Image style, drawn by a half-blocks picker.
    struct Scene {
        size: (u16, u16),
        /// The font the squares are shaped for.
        cell: CellSize,
        position: ChessPosition,
        highlights: Highlights,
        palette: Palette,
        no_color: bool,
        picker: Option<Picker>,
        /// Boxes drawn over the board.
        overlays: Vec<Rect>,
    }

    impl Scene {
        /// `fen` on a `width`×`height` terminal with the default font and truecolor.
        fn new(width: u16, height: u16, position: &str) -> Scene {
            Scene {
                size: (width, height),
                cell: CellSize::DEFAULT,
                position: fen(position),
                highlights: Highlights::default(),
                palette: palette(true),
                no_color: false,
                picker: Some(halfblocks(CellSize::DEFAULT)),
                overlays: Vec::new(),
            }
        }

        fn view(&self, geometry: BoardGeometry) -> BoardView<'_> {
            BoardView {
                position: &self.position,
                geometry,
                glyphs: GlyphSet::Image,
                palette: &self.palette,
                highlights: &self.highlights,
                no_color: self.no_color,
                picker: self.picker.as_ref(),
                overlays: &self.overlays,
            }
        }

        /// Draws the board into a fresh `TestBackend`, keeping the pictures in `images`.
        fn draw(&self, images: &mut PieceImages) -> (Terminal<TestBackend>, BoardGeometry) {
            let (width, height) = self.size;
            let mut terminal =
                Terminal::new(TestBackend::new(width, height)).expect("test backend");
            let mut saved = None;
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let geometry = layout_board(area, false, self.cell).expect("board fits");
                    saved = Some(geometry);
                    frame.render_stateful_widget(self.view(geometry), area, images);
                })
                .expect("draw");
            (terminal, saved.expect("drawn"))
        }
    }

    /// `piece` on `background` drawn by `picker` into `area` on its own: what the board
    /// should show there.
    fn picture(picker: &Picker, piece: Piece, background: Color, area: Rect) -> Buffer {
        let font = picker.font_size();
        let image = composite(
            piece,
            xterm_rgb(background).expect("a palette colour"),
            u32::from(area.width) * u32::from(font.width),
            u32::from(area.height) * u32::from(font.height),
        );
        let protocol = picker
            .new_protocol(
                DynamicImage::ImageRgba8(image),
                area.as_size(),
                Resize::Fit(None),
            )
            .expect("half-blocks always encode");
        assert_eq!(protocol.size(), Size::new(area.width, area.height));
        let mut buf = Buffer::empty(area);
        Image::new(&protocol).render(area, &mut buf);
        buf
    }

    /// The cells of `area` in `buf`, as a buffer of their own.
    fn cut(buf: &Buffer, area: Rect) -> Buffer {
        let mut part = Buffer::empty(area);
        for pos in area.positions() {
            part[pos] = buf[pos].clone();
        }
        part
    }

    /// The half-block picture in `area` as text between `|`, two lines per row of
    /// cells: ` ` for the square's colour `background`, `#` for dark pixels (Black's
    /// pieces, White's outlines), `o` for light ones (White's fill) and `+` for the
    /// tones between.
    fn pixels(buf: &Buffer, area: Rect, background: Color) -> String {
        let [r, g, b] = xterm_rgb(background).expect("a palette colour");
        let background = Color::Rgb(r, g, b);
        let shade = |color: Color| match color {
            _ if color == background => ' ',
            Color::Rgb(r, g, b) => {
                let luma =
                    (2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b)) / 10_000;
                match luma {
                    0..64 => '#',
                    64..=192 => '+',
                    _ => 'o',
                }
            }
            other => panic!("{other:?} is not a half-block colour"),
        };
        let mut lines = Vec::new();
        for row in area.rows() {
            let (mut upper, mut lower) = (String::from("|"), String::from("|"));
            for pos in row.positions() {
                let cell = &buf[pos];
                // `▄` draws the lower half in the foreground; `▀` and ` ` the upper one.
                let (top, bottom) = if cell.symbol() == "\u{2584}" {
                    (cell.bg, cell.fg)
                } else {
                    (cell.fg, cell.bg)
                };
                upper.push(shade(top));
                lower.push(shade(bottom));
            }
            lines.push(upper + "|");
            lines.push(lower + "|");
        }
        lines.join("\n")
    }

    /// True when a half-block cell shows nothing but `background`.
    fn plain(cell: &Cell, background: Color) -> bool {
        let [r, g, b] = xterm_rgb(background).expect("a palette colour");
        let rgb = Color::Rgb(r, g, b);
        cell.symbol() == " " && cell.fg == rgb && cell.bg == rgb
    }

    #[test]
    fn the_image_area_leaves_the_outer_columns_to_text() {
        assert_eq!(MIN_IMAGE_SQUARE, (5, 2));
        assert_eq!(
            image_area(Rect::new(10, 5, 5, 2)),
            Some(Rect::new(11, 5, 3, 2))
        );
        assert_eq!(
            image_area(Rect::new(0, 0, 15, 7)),
            Some(Rect::new(1, 0, 13, 7))
        );
        for (width, height) in [(3, 1), (3, 2), (5, 1), (4, 2), (0, 0)] {
            assert_eq!(
                image_area(Rect::new(0, 0, width, height)),
                None,
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn a_half_blocks_picture_fills_the_image_area() {
        let scene = Scene::new(185, 89, KINGS_AND_KNIGHT);
        let picker = scene.picker.clone().expect("a picker");
        let pal = scene.palette;
        let knight = Piece::new(Side::White, PieceKind::Knight);
        let (terminal, g) = scene.draw(&mut PieceImages::new());
        assert_eq!((g.square_w, g.square_h), (23, 11));
        let buf = terminal.backend().buffer();

        // g1 is dark; its outer columns are plain text cells.
        let g1 = square_rect(&g, sq("g1"));
        for y in g1.top()..g1.bottom() {
            for x in [g1.left(), g1.right() - 1] {
                let cell = &buf[(x, y)];
                assert_eq!((cell.symbol(), cell.bg), (" ", pal.dark), "({x}, {y})");
            }
        }
        // Between them is exactly the picture the picker makes of the composite: the
        // square's colour around the knight, the knight in the middle, no glyph.
        let area = image_area(g1).expect("a 23x11 square has room");
        assert_eq!(cut(buf, area), picture(&picker, knight, pal.dark, area));
        for corner in [
            (area.left(), area.top()),
            (area.right() - 1, area.top()),
            (area.left(), area.bottom() - 1),
            (area.right() - 1, area.bottom() - 1),
        ] {
            assert!(
                plain(&buf[corner], pal.dark),
                "{corner:?}: {:?}",
                buf[corner]
            );
        }
        assert!(!plain(&buf[glyph_cell(area)], pal.dark));
        assert!(
            area.positions()
                .all(|pos| ["\u{2580}", "\u{2584}", " "].contains(&buf[pos].symbol())),
            "only half-blocks"
        );
        insta::assert_snapshot!("image_halfblocks_knight_23x11", pixels(buf, area, pal.dark));

        // Drawn again from scratch, and as a plain widget without a cache: the same cells.
        let (again, _) = scene.draw(&mut PieceImages::new());
        assert_eq!(again.backend().buffer(), buf);
        let mut uncached = Buffer::empty(*buf.area());
        scene.view(g).render(*buf.area(), &mut uncached);
        assert_eq!(&uncached, buf);
    }

    #[test]
    fn pictures_are_kept_and_reused_on_squares_of_the_same_colour() {
        let mut images = PieceImages::new();
        // White's king on dark e1, Black's on light e8, a white pawn on light a2.
        let mut scene = Scene::new(89, 41, "4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
        let picker = scene.picker.clone().expect("a picker");
        scene.draw(&mut images);
        assert_eq!(images.len(), 3);

        // The king steps to d2, dark too, and the pawn is gone: nothing new is built, the
        // king's picture is reused, and the pawn's is kept.
        scene.position = fen("4k3/8/8/8/8/8/3K4/8 w - - 0 1");
        let (terminal, g) = scene.draw(&mut images);
        assert_eq!(images.len(), 3);
        let d2 = image_area(square_rect(&g, sq("d2"))).expect("room");
        assert_eq!(
            cut(terminal.backend().buffer(), d2),
            picture(&picker, WHITE_KING, scene.palette.dark, d2)
        );

        // On light e2 the king needs another picture; back home, it needs none.
        scene.position = fen("4k3/8/8/8/8/8/4K3/8 w - - 0 1");
        scene.draw(&mut images);
        assert_eq!(images.len(), 4);
        scene.position = fen("4k3/8/8/8/8/8/P7/4K3 w - - 0 1");
        scene.draw(&mut images);
        assert_eq!(images.len(), 4);
    }

    /// The ids of the kitty pictures whose image data `buffer` sends (the `i=` of each
    /// transmission).
    fn kitty_ids_sent(buffer: &Buffer) -> Vec<u32> {
        buffer
            .content()
            .iter()
            .filter_map(|cell| cell.symbol().split_once("_Gq=2,i="))
            .map(|(_, rest)| {
                let id = rest.split(',').next().unwrap_or_default();
                id.parse().expect("a numeric id")
            })
            .collect()
    }

    #[test]
    fn kitty_pictures_use_ids_the_session_records_for_deletion() {
        // Kitty deletes a virtual placement only by its id, so every id a picture is
        // sent with must be one the terminal restore will delete, including those of
        // pictures dropped when the squares or the font change.
        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
        scene.picker = Some(picker_for(ProtocolType::Kitty, CellSize::DEFAULT));
        let mut images = PieceImages::new();
        let (terminal, _) = scene.draw(&mut images);
        let first = kitty_ids_sent(terminal.backend().buffer());
        assert_eq!(first.len(), 3, "{first:?}");

        images.clear();
        let (terminal, _) = scene.draw(&mut images);
        let second = kitty_ids_sent(terminal.backend().buffer());
        assert_eq!(second.len(), 3, "{second:?}");

        let recorded = crate::tui::terminal::recorded_kitty_ids();
        let mut all = first.clone();
        all.extend(&second);
        for id in &all {
            assert_ne!(*id, 0);
            assert!(recorded.contains(id), "{id} is not recorded");
        }
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 6, "every picture has its own id");
    }

    #[test]
    fn each_highlight_colour_gets_its_own_picture() {
        let mut scene = Scene::new(89, 41, ROOK_CHECK);
        let picker = scene.picker.clone().expect("a picker");
        let pal = scene.palette;
        let mut images = PieceImages::new();
        scene.draw(&mut images);
        assert_eq!(images.len(), 3);

        let steps = [
            (
                Highlights {
                    check: Some(sq("e1")),
                    ..Highlights::default()
                },
                "e1",
                pal.check,
            ),
            (
                Highlights {
                    selected: Some(sq("e1")),
                    check: Some(sq("e1")),
                    ..Highlights::default()
                },
                "e1",
                pal.selected,
            ),
            (
                Highlights {
                    targets: vec![sq("a1")],
                    ..Highlights::default()
                },
                "a1",
                pal.target,
            ),
            (
                Highlights {
                    last_move: Some((sq("a8"), sq("a1"))),
                    ..Highlights::default()
                },
                "a1",
                pal.last_move,
            ),
        ];
        for (count, (highlights, name, tint)) in (4..).zip(steps) {
            scene.highlights = highlights;
            let (terminal, g) = scene.draw(&mut images);
            assert_eq!(images.len(), count, "{name} on {tint:?}");
            let area = image_area(square_rect(&g, sq(name))).expect("room");
            let piece = scene.position.piece_at(sq(name)).expect("a piece");
            assert_eq!(
                cut(terminal.backend().buffer(), area),
                picture(&picker, piece, tint, area),
                "{name} on {tint:?}"
            );
        }

        // Plain colours again: the first pictures are still there.
        scene.highlights = Highlights::default();
        scene.draw(&mut images);
        assert_eq!(images.len(), 7);
    }

    #[test]
    fn half_blocks_need_bigger_squares_than_pixel_protocols() {
        assert_eq!(MIN_IMAGE_SQUARE, (5, 2));
        assert_eq!(MIN_HALFBLOCK_SQUARE, (11, 5));
        assert_eq!(
            image_area(Rect::new(0, 0, 11, 5)).map(|area| area.as_size()),
            Some(Size::new(9, 5))
        );
        assert_eq!(
            min_picture_square(ProtocolType::Halfblocks),
            MIN_HALFBLOCK_SQUARE
        );
        for protocol in [
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
            ProtocolType::Sixel,
        ] {
            assert_eq!(
                min_picture_square(protocol),
                MIN_IMAGE_SQUARE,
                "{protocol:?}"
            );
        }
    }

    #[test]
    fn squares_too_small_for_a_picture_show_the_solid_glyph() {
        let glyph = glyphs::glyph(GlyphSet::Solid, WHITE_KING);
        // (terminal, font the squares are shaped for, square size, protocol)
        let cases = [
            ((31, 11), CellSize::DEFAULT, (3, 1), ProtocolType::Sixel),
            ((41, 9), CellSize::new(5, 20), (5, 1), ProtocolType::Kitty),
            (
                (25, 17),
                CellSize::new(12, 12),
                (3, 2),
                ProtocolType::Iterm2,
            ),
            // Half blocks: 7×3 and 9×4 squares, and one short of 11×5 either way.
            (
                (57, 25),
                CellSize::DEFAULT,
                (7, 3),
                ProtocolType::Halfblocks,
            ),
            (
                (73, 33),
                CellSize::DEFAULT,
                (9, 4),
                ProtocolType::Halfblocks,
            ),
            (
                (89, 33),
                CellSize::new(8, 21),
                (11, 4),
                ProtocolType::Halfblocks,
            ),
            (
                (73, 41),
                CellSize::new(10, 17),
                (9, 5),
                ProtocolType::Halfblocks,
            ),
        ];
        for ((width, height), cell, size, protocol) in cases {
            let mut scene = Scene::new(width, height, KINGS_AND_KNIGHT);
            scene.cell = cell;
            scene.picker = Some(picker_for(protocol, cell));
            let mut images = PieceImages::new();
            let (terminal, g) = scene.draw(&mut images);
            assert_eq!((g.square_w, g.square_h), size);
            let e1 = &terminal.backend().buffer()[glyph_cell(square_rect(&g, Square::E1))];
            assert_eq!(
                (e1.symbol(), e1.fg),
                (glyph, scene.palette.white_piece),
                "{size:?} {protocol:?}"
            );
            assert!(
                images.is_empty(),
                "{size:?} {protocol:?}: a picture was built"
            );
        }

        // Half blocks from 11×5 on, pictures.
        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
        let mut images = PieceImages::new();
        let (terminal, g) = scene.draw(&mut images);
        assert_eq!((g.square_w, g.square_h), MIN_HALFBLOCK_SQUARE);
        assert_eq!(images.len(), 3);
        let e1 = glyph_cell(square_rect(&g, Square::E1));
        assert_ne!(terminal.backend().buffer()[e1].symbol(), glyph);

        // Without a picker the Image style is the Solid glyphs.
        scene.picker = None;
        let mut images = PieceImages::new();
        let (terminal, _) = scene.draw(&mut images);
        assert_eq!(terminal.backend().buffer()[e1].symbol(), glyph);
        assert!(images.is_empty());

        // The pixel protocols from 5×2 on.
        let mut scene = Scene::new(41, 17, KINGS_AND_KNIGHT);
        scene.picker = Some(picker_for(ProtocolType::Sixel, CellSize::DEFAULT));
        let mut images = PieceImages::new();
        let (terminal, g) = scene.draw(&mut images);
        assert_eq!((g.square_w, g.square_h), MIN_IMAGE_SQUARE);
        assert_eq!(images.len(), 3);
        let e1 = glyph_cell(square_rect(&g, Square::E1));
        assert_ne!(terminal.backend().buffer()[e1].symbol(), glyph);
    }

    #[test]
    fn a_picture_too_large_to_build_is_drawn_as_the_glyph() {
        // No real font gives an image area over 4096 pixels on a side; building one
        // would take seconds and hundreds of megabytes on the UI thread.
        let picker = halfblocks(CellSize::new(256, 256));
        let mut images = PieceImages::new();
        let too_wide = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(17, 1));
        assert!(too_wide.is_none(), "17 cells of 256 pixels");
        let too_tall = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(1, 17));
        assert!(too_tall.is_none(), "17 rows of 256 pixels");
        assert!(images.is_empty(), "nothing was built");
        let widest = images.picture(&picker, WHITE_KING, [0, 0, 0], Size::new(16, 1));
        assert!(widest.is_some(), "4096 pixels are built");
    }

    #[test]
    fn another_square_size_or_font_replaces_the_pictures() {
        let mut images = PieceImages::new();
        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
        scene.draw(&mut images);
        assert_eq!(images.len(), 3);

        // Bigger squares: the small pictures are dropped, not kept beside the new ones.
        scene.size = (105, 49);
        scene.draw(&mut images);
        assert_eq!(images.len(), 3);

        // Another font gives the same squares other pixel sizes.
        let font = CellSize::new(8, 16);
        scene.picker = Some(halfblocks(font));
        let (terminal, g) = scene.draw(&mut images);
        assert_eq!(images.len(), 3);
        let area = image_area(square_rect(&g, Square::E1)).expect("room");
        assert_eq!(
            cut(terminal.backend().buffer(), area),
            picture(&halfblocks(font), WHITE_KING, scene.palette.dark, area)
        );
    }

    #[test]
    fn the_cursor_and_the_no_colour_marks_stay_beside_a_picture() {
        let mut scene = Scene::new(89, 41, ROOK_CHECK);
        let picker = scene.picker.clone().expect("a picker");
        let pal = scene.palette;
        let rook = Piece::new(Side::Black, PieceKind::Rook);

        // The cursor's brackets and tint take the outer columns; the picture is whole.
        scene.highlights.cursor = Some(sq("e1"));
        let (terminal, g) = scene.draw(&mut PieceImages::new());
        let buf = terminal.backend().buffer();
        let e1 = square_rect(&g, sq("e1"));
        for y in e1.top()..e1.bottom() {
            assert_eq!(buf[(e1.left(), y)].bg, pal.cursor);
            assert_eq!(buf[(e1.right() - 1, y)].bg, pal.cursor);
        }
        let row = glyph_cell(e1).y;
        assert_eq!(buf[(e1.left(), row)].symbol(), "[");
        assert_eq!(buf[(e1.right() - 1, row)].symbol(), "]");
        let area = image_area(e1).expect("room");
        assert_eq!(cut(buf, area), picture(&picker, WHITE_KING, pal.dark, area));

        // Without colour the marks sit in the outer columns too, and under the cursor,
        // which holds those, the mark keeps its right side, as on a narrow square.
        scene.no_color = true;
        let cases = [
            ("a1", None, "(", ")", rook, pal.target),
            ("a1", Some("a1"), "[", ")", rook, pal.target),
            ("e1", None, "+", "+", WHITE_KING, pal.check),
            ("e1", Some("e1"), "[", "+", WHITE_KING, pal.check),
        ];
        for (name, cursor, left, right, piece, tint) in cases {
            scene.highlights = Highlights {
                targets: vec![sq("a1")],
                check: Some(sq("e1")),
                cursor: cursor.map(sq),
                ..Highlights::default()
            };
            let (terminal, g) = scene.draw(&mut PieceImages::new());
            let buf = terminal.backend().buffer();
            let rect = square_rect(&g, sq(name));
            let row = glyph_cell(rect).y;
            assert_eq!(
                (
                    buf[(rect.left(), row)].symbol(),
                    buf[(rect.right() - 1, row)].symbol()
                ),
                (left, right),
                "{name} cursor {cursor:?}"
            );
            let area = image_area(rect).expect("room");
            assert_eq!(
                cut(buf, area),
                picture(&picker, piece, tint, area),
                "{name} cursor {cursor:?}"
            );
        }
    }

    #[test]
    fn empty_targets_keep_their_dots_beside_pictures() {
        let mut scene = Scene::new(57, 25, KINGS_AND_KNIGHT);
        scene.highlights = Highlights {
            selected: Some(sq("g1")),
            targets: vec![sq("e2"), sq("f3"), sq("h3")],
            ..Highlights::default()
        };
        let (terminal, g) = scene.draw(&mut PieceImages::new());
        let buf = terminal.backend().buffer();
        for name in ["e2", "f3", "h3"] {
            let rect = square_rect(&g, sq(name));
            let dot = glyph_cell(rect);
            assert_eq!(
                (buf[dot].symbol(), buf[dot].fg),
                (".", scene.palette.black_piece),
                "{name}"
            );
            assert!(
                rect.positions()
                    .all(|pos| pos == dot || buf[pos].symbol() == " "),
                "{name}"
            );
        }
    }

    #[test]
    fn pictures_on_the_256_colour_palette_use_the_xterm_colours() {
        // Big squares, so the picture's corners are far enough from the king to be exact.
        let mut scene = Scene::new(121, 57, KINGS_AND_KNIGHT);
        scene.palette = palette(false);
        let picker = scene.picker.clone().expect("a picker");
        let (terminal, g) = scene.draw(&mut PieceImages::new());
        let buf = terminal.backend().buffer();
        let e1 = square_rect(&g, Square::E1);
        // The text cells keep the index; the picture is composited onto xterm's #875F00.
        assert_eq!(buf[(e1.left(), e1.top())].bg, Color::Indexed(94));
        let area = image_area(e1).expect("room");
        let corner = &buf[(area.left(), area.top())];
        assert_eq!(
            (corner.symbol(), corner.fg, corner.bg),
            (
                " ",
                Color::Rgb(0x87, 0x5F, 0x00),
                Color::Rgb(0x87, 0x5F, 0x00)
            )
        );
        assert_eq!(
            cut(buf, area),
            picture(&picker, WHITE_KING, Color::Indexed(94), area)
        );
    }

    #[test]
    fn a_picture_the_area_cuts_off_is_drawn_as_the_glyph() {
        let scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
        let full = Rect::new(0, 0, 89, 41);
        let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
        // Rank 1 takes rows 35 to 39; the area stops after row 38.
        let e1 = square_rect(&g, Square::E1);
        assert_eq!((e1.y, e1.height), (35, 5));
        let mut buf = Buffer::empty(full);
        let mut images = PieceImages::new();
        ratatui::widgets::StatefulWidget::render(
            scene.view(g),
            Rect::new(0, 0, 89, 39),
            &mut buf,
            &mut images,
        );
        let glyph = &buf[glyph_cell(e1)];
        assert_eq!(
            (glyph.symbol(), glyph.fg),
            (
                glyphs::glyph(GlyphSet::Solid, WHITE_KING),
                scene.palette.white_piece
            )
        );
        assert_eq!(buf[(e1.x, 39)], Cell::EMPTY);
        // Only Black's king, on rank 8, has its whole image area inside.
        assert_eq!(images.len(), 1);
    }

    #[test]
    fn a_piece_under_a_box_shows_its_glyph_until_the_box_closes() {
        let mut scene = Scene::new(89, 41, KINGS_AND_KNIGHT);
        let picker = scene.picker.clone().expect("a picker");
        let full = Rect::new(0, 0, 89, 41);
        let g = layout_board(full, false, CellSize::DEFAULT).expect("fits");
        let (e1, g1) = (square_rect(&g, Square::E1), square_rect(&g, sq("g1")));
        // A box over one cell of g1's picture, its last one, and nothing of e1's.
        let g1_area = image_area(g1).expect("room");
        scene.overlays = vec![Rect::new(g1_area.right() - 1, g1_area.bottom() - 1, 4, 2)];
        let mut images = PieceImages::new();
        let (terminal, _) = scene.draw(&mut images);
        let buf = terminal.backend().buffer();
        let knight = Piece::new(Side::White, PieceKind::Knight);
        let glyph = &buf[glyph_cell(g1)];
        assert_eq!(
            (glyph.symbol(), glyph.fg),
            (
                glyphs::glyph(GlyphSet::Solid, knight),
                scene.palette.white_piece
            )
        );
        let e1_area = image_area(e1).expect("room");
        assert_eq!(
            cut(buf, e1_area),
            picture(&picker, WHITE_KING, scene.palette.dark, e1_area)
        );
        // No picture is made for the covered knight.
        assert_eq!(images.len(), 2);

        // The box closes: the knight is a picture again.
        scene.overlays.clear();
        let (terminal, _) = scene.draw(&mut images);
        assert_eq!(
            cut(terminal.backend().buffer(), g1_area),
            picture(&picker, knight, scene.palette.dark, g1_area)
        );
        assert_eq!(images.len(), 3);
    }

    /// The image-encoding crates [`PieceImages::picture`] calls into (resize, sixel, PNG
    /// and base64) run at the default `opt-level = 0` under `cargo build`/`cargo run`
    /// unless the dev profile optimises them, which can stall the UI thread for seconds
    /// while a board's worth of pictures is built. This does not run the encoders (that
    /// needs a real build, not `cargo test`); it only guards the profile override that
    /// keeps them fast in a debug build, the same way `[profile.dev.package.chess]`
    /// already does for this crate's own code.
    #[test]
    fn dev_builds_optimise_the_picture_encoding_crates() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("Cargo.toml reads");
        for pkg in [
            "image",
            "ratatui-image",
            "icy_sixel",
            "png",
            "fdeflate",
            "miniz_oxide",
            "flate2",
            "base64",
        ] {
            let heading = format!("[profile.dev.package.{pkg}]");
            let after = manifest
                .split(&heading)
                .nth(1)
                .unwrap_or_else(|| panic!("{heading} is missing from Cargo.toml"));
            let body = after.split("[profile").next().unwrap_or(after);
            assert!(
                body.contains("opt-level = 3") || body.contains("opt-level = 2"),
                "{heading} does not raise opt-level in Cargo.toml"
            );
        }
    }
}
