//! Board widget and flip-aware hit-testing (spec section 6.3).
//!
//! [`layout_board`] picks a square size and places the board inside an area;
//! the resulting [`BoardGeometry`] is used both to draw ([`BoardView`]) and to
//! map mouse cells back to squares ([`square_at`]), so the App must keep the
//! geometry from its most recent draw for hit-testing.
//!
//! The widget draws only the 8×8 grid plus a rank-label column on the left and
//! a file-label row underneath; any enclosing block is the caller's.

use ratatui::{
    buffer::{Buffer, Cell},
    layout::{Position as CellPosition, Rect},
    style::{Color, Modifier},
    widgets::Widget,
};

use super::glyphs::{self, GlyphSet, Palette};
use crate::core::{Color as Side, PieceKind, Position as ChessPosition, Square};

/// Square sizes `(width, height)` in cells, largest first. Terminal cells are
/// about twice as tall as wide, so these look roughly square, and the odd
/// widths let the one-cell glyph sit exactly in the middle.
pub const SQUARE_SIZES: [(u16, u16); 3] = [(7, 3), (5, 2), (3, 1)];

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
    /// Width of one square in cells (7, 5 or 3).
    pub square_w: u16,
    /// Height of one square in cells (3, 2 or 1).
    pub square_h: u16,
    /// Black at the bottom when true; White at the bottom otherwise.
    pub flipped: bool,
}

/// Lays the board out in `area` with the largest square size from
/// [`SQUARE_SIZES`] that fits (labels included), centred in both directions.
/// Returns `None` when even 3×1 squares do not fit (25×9 cells).
pub fn layout_board(area: Rect, flipped: bool) -> Option<BoardGeometry> {
    SQUARE_SIZES.into_iter().find_map(|(square_w, square_h)| {
        let outer_w = 8 * square_w + LABEL_COLUMNS;
        let outer_h = 8 * square_h + LABEL_ROWS;
        if area.width < outer_w || area.height < outer_h {
            return None;
        }
        let x = area.x + (area.width - outer_w) / 2;
        let y = area.y + (area.height - outer_h) / 2;
        Some(BoardGeometry {
            outer: Rect::new(x, y, outer_w, outer_h),
            grid: Rect::new(x + LABEL_COLUMNS, y, 8 * square_w, 8 * square_h),
            square_w,
            square_h,
            flipped,
        })
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
}

impl Widget for BoardView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &BoardView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
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
            self.render_square(sq, clip, buf);
        }
        self.render_labels(clip, buf);
    }
}

impl BoardView<'_> {
    fn render_square(&self, sq: Square, clip: Rect, buf: &mut Buffer) {
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
        if let Some(cell) = cell_in(buf, clip, glyph_cell(rect)) {
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
            self.render_side_marks(rect, (left, right), cursor, clip, buf);
        }
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
    /// the cursor, whose `[` `]` hold the outer columns, they go just inside them (`[(p)]`);
    /// a square too narrow for both keeps the cursor's `[` and the mark's right side
    /// (`[p)`), so the cursor never hides the mark.
    fn render_side_marks(
        &self,
        rect: Rect,
        (left, right): (&'static str, &'static str),
        cursor: bool,
        clip: Rect,
        buf: &mut Buffer,
    ) {
        if rect.width < 3 {
            return;
        }
        let (first, last) = (rect.left(), rect.right() - 1);
        let columns = match (cursor, rect.width >= 5) {
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
    use ratatui::{Terminal, backend::TestBackend, style::Modifier, text::Span};

    use super::*;
    use crate::tui::glyphs::{SOLID_PAWN, palette};
    use crate::tui::test_support::sq;

    fn geometry(square_w: u16, square_h: u16, x: u16, y: u16, flipped: bool) -> BoardGeometry {
        let area = Rect::new(x, y, 8 * square_w + 1, 8 * square_h + 1);
        let g = layout_board(area, flipped).expect("exact-fit area");
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
                let geometry = layout_board(area, flipped).expect("board fits");
                saved = Some(geometry);
                frame.render_widget(
                    BoardView {
                        position,
                        geometry,
                        glyphs: glyph_set,
                        palette: &pal,
                        highlights,
                        no_color,
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
    fn layout_picks_the_largest_size_that_fits_and_centres_it() {
        let cases = [
            ((80, 24), (5, 2), Rect::new(19, 3, 41, 17)),
            ((120, 40), (7, 3), Rect::new(31, 7, 57, 25)),
            ((200, 60), (7, 3), Rect::new(71, 17, 57, 25)),
            ((60, 20), (5, 2), Rect::new(9, 1, 41, 17)),
            ((40, 16), (3, 1), Rect::new(7, 3, 25, 9)),
            ((25, 9), (3, 1), Rect::new(0, 0, 25, 9)),
        ];
        for ((width, height), size, outer) in cases {
            let g = layout_board(Rect::new(0, 0, width, height), false)
                .unwrap_or_else(|| panic!("{width}x{height} should fit"));
            assert_eq!((g.square_w, g.square_h), size, "{width}x{height}");
            assert_eq!(g.outer, outer, "{width}x{height}");
            assert_eq!(
                g.grid,
                Rect::new(outer.x + 1, outer.y, 8 * size.0, 8 * size.1),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn layout_is_none_when_too_small() {
        for (width, height) in [(24, 9), (25, 8), (0, 0), (100, 8), (24, 100)] {
            assert_eq!(layout_board(Rect::new(0, 0, width, height), false), None);
        }
    }

    #[test]
    fn layout_respects_the_area_origin_and_flip() {
        let g = layout_board(Rect::new(10, 5, 26, 10), true).expect("fits");
        assert_eq!(g.outer, Rect::new(10, 5, 25, 9));
        assert_eq!(g.grid, Rect::new(11, 5, 24, 8));
        assert!(g.flipped);
    }

    #[test]
    fn square_rect_and_square_at_round_trip() {
        for (w, h) in SQUARE_SIZES {
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
        for (w, h) in SQUARE_SIZES {
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
        for (w, h) in SQUARE_SIZES {
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
}
