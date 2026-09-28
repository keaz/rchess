//! Drawing every screen (spec sections 6.2, 6.3 and 9.2).
//!
//! [`draw`] renders an [`App`] through its public accessors only: the menu, the playing
//! screen's panels, the game-over overlay, the Jev exchange view (debug mode), the top
//! dialog and the too-small notice. It returns the [`HitMap`] of everything clickable,
//! which the app keeps for the next mouse event, the move-list scroll clamped to what the
//! list can show, and the exchange view's page size and scroll limit. The board's piece
//! pictures (the Image style) are kept in the [`PieceImages`] the app lends it, and are
//! not drawn where the game-over box or a dialog will cover them.
//!
//! Playing layout: the screen fills the terminal. The left column holds the Board block,
//! sized for the largest board that fits beside the narrowest side column
//! ([`SIDE_MIN_WIDTH`]) with squares shaped for the font ([`App::cell_size`]), and the
//! Command box under it. The right column takes every column left over and stacks Status,
//! the computer's panel (titled "Jev" or "Local search", only in games against the
//! computer), Moves and Captured with shared borders. A board limited by the width is
//! centred vertically in its block, which still fills the column.
//!
//! Status and Captured have fixed heights (Status per mode and terminal height), so they
//! never jump. The Jev panel grows with its text (the source, Jev's top three, confidence,
//! latency, model and the note) and takes the rows from Moves, which gets the rest and
//! keeps at least [`MOVES_MIN_ROWS`]; when even that is not enough, only the note is cut
//! short, ending in `…`. Menu, dialogs, help and the game-over overlay stay centred boxes.
//! In debug mode the Status panel says `DEBUG`: on the top border beside the mode when
//! both fit, else at the start of its first line.
//!
//! The exchange view takes the whole screen instead of the playing screen (nothing of the
//! board is drawn under it, so no piece picture is lost under it), with dialogs still on
//! top. Its body text is rendered and cut into rows for the exchange on screen only, and
//! kept for the next frame until the exchange or the width changes ([`BodyCache`]); a
//! frame only builds the rows it shows.
//!
//! The snapshot tests at the bottom of this file render every screen; their files in
//! `src/tui/snapshots/` (`chess__tui__panels__tests__*.snap`) show the exact output at
//! 60×20, 80×24 and 120×40.
//!
//! Colours: the board and the piece trays (captured pieces, promotion choices) use the
//! [`Palette`], so both piece colours stay readable. Everything else uses the terminal's
//! own colours (dim hints, red errors, yellow warnings, cyan dialog borders), which colour
//! themes keep readable on their own background.

use std::cmp::Reverse;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::merge::MergeStrategy;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Padding, Paragraph, Wrap};

use super::app::{
    App, Button, Dialog, GAME_OVER_BUTTONS, Hit, HitMap, InputPurpose, MENU_ITEMS, MenuItem,
    Message, Mode, PROMOTION_CHOICES, Question, Screen, SidePick, TOO_SMALL, WAITING_FOR_ENGINE,
    is_too_small, move_rows, outcome_text,
};
use super::board::{BoardGeometry, BoardView, CellSize, PieceImages, layout_board};
use super::debug::{BodyCache, BodyRows, ExchangeView, LineKind, NO_EXCHANGES, Record};
use super::glyphs::{self, ELLIPSIS, GlyphSet, Palette, char_width};
use super::input::LineEditor;
use crate::core::{Color as Side, Game, Piece, PieceKind, Position as ChessPosition};
use crate::engine::{ComputerMove, MoveSource};

/// The help dialog's text. The first [`HELP_KEY_WIDTH`] characters of each line are the
/// key column (drawn bold); no line is wider than 56 cells, so the dialog fits a 60-column
/// terminal.
pub const HELP_LINES: [&str; 14] = [
    "Mouse     click a piece, then a square, or drag it there",
    "Arrows    move the cursor; Enter picks up and puts down",
    "Esc       drop the piece, or leave the command box",
    "/         type a move: e4, Nf3, e2e4, O-O, e8=Q",
    ":         type a command:",
    "            :undo :flip :new :resign :glyphs :help :quit",
    "            :fen <FEN>  :savefen <path>  :savepgn <path>",
    "u  f  n   undo, flip the board, new game",
    "g  m  ?   glyph set, menu, this help",
    "d         exchange view (start with --debug)",
    "Ctrl+S    save the game as PGN",
    "q         quit (Ctrl+C works everywhere)",
    "Space     pause watching, or retry a failed engine",
    "+  -      watching: slower, faster",
];
/// Width of the key column in [`HELP_LINES`].
pub const HELP_KEY_WIDTH: usize = 10;

/// Narrowest right column; the board shrinks before the column does. The column takes
/// every column the board leaves.
pub const SIDE_MIN_WIDTH: u16 = 30;
/// Height of the Command box (one text row between borders).
const COMMAND_HEIGHT: u16 = 3;
/// Side panel chrome across: two borders and the blank column on the left.
const SIDE_CHROME_WIDTH: u16 = 3;
/// Text rows the Jev panel always gets, so short replies do not make it jump.
pub const JEV_MIN_ROWS: u16 = 3;
/// Text rows the move list keeps however much the Jev panel wants.
pub const MOVES_MIN_ROWS: u16 = 3;
/// Text rows of the Captured panel: one per side.
const CAPTURED_ROWS: u16 = 2;
/// Board block border plus one blank column each side of the board.
const BOARD_CHROME_WIDTH: u16 = 4;
/// Board block border rows.
const BOARD_CHROME_HEIGHT: u16 = 2;
/// Menu box width (narrower terminals get the full width).
const MENU_WIDTH: u16 = 60;
/// Blank columns inside the menu box on each side.
const MENU_PADDING: u16 = 2;
/// The command box and input dialog prompt.
const PROMPT: &str = "> ";
/// Cells taken by [`PROMPT`].
const PROMPT_WIDTH: u16 = 2;
/// Dialog borders.
const ACCENT: Color = Color::Cyan;
/// The Status panel's border tag in debug mode.
const DEBUG_TAG: &str = " DEBUG ";
/// The exchange view's keys, on its bottom border.
const EXCHANGE_KEYS: &str = " ↑↓ PgUp PgDn Home End · ←→ older/newer · Esc closes ";
/// Pawn, knight, bishop, rook, queen and king values for the material count; the king
/// never leaves the board, so its value is irrelevant.
const PIECE_VALUES: [i32; 6] = [1, 3, 3, 5, 9, 0];

/// What [`draw`] reports back to the app.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drawn {
    /// Everything clickable, in drawing order (later entries are on top).
    pub hits: HitMap,
    /// The move-list scroll, clamped so the list never scrolls past its first row.
    /// Unchanged when the move list was not drawn.
    pub move_scroll: usize,
    /// The exchange view with its scroll clamped and its page size and scroll limit as
    /// drawn. Unchanged when the view was not drawn.
    pub exchange_view: Option<ExchangeView>,
}

/// Draws `app` into `frame` and returns the click targets and the clamped move-list
/// scroll. `images` keeps the board's piece pictures between frames (the Image style),
/// and `bodies` the exchange view's body text for the exchange on screen. `now` is used
/// for the thinking spinner only.
pub fn draw(
    app: &App,
    images: &mut PieceImages,
    bodies: &mut BodyCache,
    frame: &mut Frame,
    now: Instant,
) -> Drawn {
    let mut drawn = Drawn {
        hits: HitMap::default(),
        move_scroll: app.move_scroll(),
        exchange_view: app.exchange_view(),
    };
    let area = frame.area();
    if is_too_small(area) {
        too_small(frame, area);
        return drawn;
    }
    let overlays = overlays(app, area);
    match (app.exchange_view(), app.screen()) {
        (Some(view), _) => {
            drawn.exchange_view = Some(exchange_screen(frame, area, app, view, bodies));
        }
        (None, Screen::Menu) => menu(frame, area, app, &mut drawn.hits),
        (None, Screen::Playing) => {
            playing(frame, area, app, images, &overlays, now, &mut drawn);
        }
        (None, Screen::GameOver) => {
            playing(frame, area, app, images, &overlays, now, &mut drawn);
            game_over(frame, area, app, &mut drawn.hits);
        }
    }
    // Drawn last, so its targets win in `HitMap::at`.
    if let Some(top) = app.dialog() {
        dialog(frame, area, app, top, &mut drawn.hits);
    }
    drawn
}

// ----- too small -----

/// Only [`TOO_SMALL`], centred.
fn too_small(frame: &mut Frame, area: Rect) {
    let rows = wrapped_height(TOO_SMALL, area.width).min(area.height);
    let y = area.y + (area.height - rows) / 2;
    frame.render_widget(
        Paragraph::new(TOO_SMALL)
            .centered()
            .wrap(Wrap { trim: true }),
        Rect::new(area.x, y, area.width, rows),
    );
}

// ----- menu -----

/// The game menu: entries, a line about the highlighted one, the engine status and
/// warnings, and the keys.
fn menu(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let width = MENU_WIDTH.min(area.width);
    let text_width = width.saturating_sub(2 + 2 * MENU_PADDING);
    let index = app.menu_index().min(MENU_ITEMS.len() - 1);
    let computer = app.computer_name();
    let about = menu_description(MENU_ITEMS[index], computer);
    let about_rows = wrapped_height(&about, text_width);
    let notes = menu_notes(app);
    let notes_rows = notes
        .iter()
        .map(|note| note.rows(text_width))
        .fold(0u16, u16::saturating_add);
    let items = u16::try_from(MENU_ITEMS.len()).unwrap_or(u16::MAX);
    // Borders, title, gap, items, gap, about, gap, notes, gap, keys.
    let height = [2, 1, 1, items, 1, about_rows, 1, notes_rows, 1, 1]
        .into_iter()
        .fold(0u16, u16::saturating_add);
    let rect = centered(area, width, height);
    let block = Block::bordered()
        .title(" rchess ".bold())
        .padding(Padding::horizontal(MENU_PADDING));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    frame.render_widget(Line::from("New game").bold(), row_of(inner, 0));
    for (index, (item, offset)) in MENU_ITEMS.iter().zip(2u16..).enumerate() {
        let row = row_of(inner, offset);
        let selected = index == app.menu_index();
        let marker = if selected { ">" } else { " " };
        let line = Line::from(format!("{marker} {}. {}", index + 1, item.label(computer)));
        frame.render_widget(if selected { line.reversed() } else { line }, row);
        hits.push(row, Hit::MenuItem(index));
    }
    let about_top = inner.y.saturating_add(items + 3);
    let about_area = Rect::new(inner.x, about_top, inner.width, about_rows).intersection(inner);
    frame.render_widget(
        Paragraph::new(about).dim().wrap(Wrap { trim: true }),
        about_area,
    );
    // The keys stay on the bottom row; the notes get what is left above them.
    let keys = last_row(inner);
    let notes_top = about_area.bottom().saturating_add(1);
    let notes_height = keys.y.saturating_sub(notes_top.saturating_add(1));
    let notes_area = Rect::new(inner.x, notes_top, inner.width, notes_height).intersection(inner);
    let (notes, more) = fit_notes(notes, notes_area.width, notes_area.height);
    let mut top = notes_area.y;
    for note in notes.into_iter().chain(more) {
        let rows = note.rows(notes_area.width);
        let rect = Rect::new(notes_area.x, top, notes_area.width, rows).intersection(notes_area);
        note.render(frame, rect);
        top = top.saturating_add(rows);
    }
    frame.render_widget(
        Line::from("arrows/jk choose · Enter/1-7 start · q quit").dim(),
        keys,
    );
}

/// One line about a menu entry, with the computer called `computer`.
fn menu_description(item: MenuItem, computer: &str) -> String {
    match item {
        MenuItem::HumanVsHuman => "Two players take turns at this keyboard.".to_string(),
        MenuItem::HumanVsJev(SidePick::White) => "You play White and move first.".to_string(),
        MenuItem::HumanVsJev(SidePick::Black) => {
            format!("{computer} plays White and opens; the board is flipped.")
        }
        MenuItem::HumanVsJev(SidePick::Random) => {
            "A coin flip decides which side you play.".to_string()
        }
        MenuItem::JevVsJev => format!("Watch {computer} play itself (space pauses, +/- pace)."),
        MenuItem::LoadFen => "Paste a FEN and play it, Human vs Human.".to_string(),
        MenuItem::Quit => "Leave rchess.".to_string(),
    }
}

/// A menu note: the engine status or a warning, wrapped with a hanging indent after
/// its marker.
struct Note {
    marker: &'static str,
    text: String,
    style: Style,
}

impl Note {
    /// Rows the note takes in `width` cells.
    fn rows(&self, width: u16) -> u16 {
        wrapped_height(&self.text, width.saturating_sub(self.marker_width()))
    }

    fn marker_width(&self) -> u16 {
        u16::try_from(Span::raw(self.marker).width()).unwrap_or(u16::MAX)
    }

    fn render(self, frame: &mut Frame, area: Rect) {
        let indent = self.marker_width().min(area.width);
        frame.render_widget(
            Span::styled(self.marker, self.style),
            Rect {
                width: indent,
                ..area
            },
        );
        let text = Rect {
            x: area.x + indent,
            width: area.width - indent,
            ..area
        };
        frame.render_widget(
            Paragraph::new(self.text)
                .style(self.style)
                .wrap(Wrap { trim: true }),
            text,
        );
    }
}

/// The first of `notes` that fit in `rows` rows of `width` cells, and when some do not, a
/// last note in their place saying how many warnings are not shown ("+2 more warnings").
fn fit_notes(mut notes: Vec<Note>, width: u16, rows: u16) -> (Vec<Note>, Option<Note>) {
    let needed = notes
        .iter()
        .map(|note| note.rows(width))
        .fold(0u16, u16::saturating_add);
    if needed <= rows || rows == 0 {
        return (notes, None);
    }
    // One row for the count.
    let room = rows - 1;
    let mut used = 0u16;
    let shown = notes
        .iter()
        .take_while(|note| {
            used = used.saturating_add(note.rows(width));
            used <= room
        })
        .count();
    let hidden = notes
        .drain(shown..)
        .filter(|note| note.marker == WARNING_MARKER)
        .count();
    let text = match hidden {
        1 => "+1 more warning".to_string(),
        n => format!("+{n} more warnings"),
    };
    let more = Note {
        marker: "",
        text,
        style: Style::new().yellow(),
    };
    (notes, Some(more))
}

/// What a warning note starts with on the menu.
const WARNING_MARKER: &str = "! ";

/// The engine status (green when Jev plays, yellow for the local search alone) and one
/// [`WARNING_MARKER`] note per warning.
fn menu_notes(app: &App) -> Vec<Note> {
    let mut notes = vec![Note {
        marker: "",
        text: app.engine_status().to_string(),
        style: if app.uses_jev() {
            Style::new().green()
        } else {
            Style::new().yellow()
        },
    }];
    notes.extend(app.warnings().iter().map(|warning| Note {
        marker: WARNING_MARKER,
        text: warning.clone(),
        style: Style::new().yellow(),
    }));
    notes
}

// ----- playing screen -----

/// Where the playing screen's blocks go. The right column's rects overlap by one row, so
/// neighbouring panels share a border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlayingLayout {
    board: Rect,
    command: Rect,
    status: Rect,
    jev: Option<Rect>,
    moves: Rect,
    captured: Rect,
}

/// The playing screen's two columns in `area`, which together take its full width: the
/// board column's width and the right column's width.
fn columns(area: Rect, cell: CellSize) -> (u16, u16) {
    // The largest board whose block fits beside the narrowest right column and above the
    // command box; the block is exactly as wide as that board.
    let room = Rect::new(
        0,
        0,
        area.width
            .saturating_sub(SIDE_MIN_WIDTH.saturating_add(BOARD_CHROME_WIDTH)),
        area.height
            .saturating_sub(COMMAND_HEIGHT.saturating_add(BOARD_CHROME_HEIGHT)),
    );
    let board_width = layout_board(room, false, cell).map_or(area.width / 2, |g| {
        g.outer.width.saturating_add(BOARD_CHROME_WIDTH)
    });
    (board_width, area.width.saturating_sub(board_width))
}

/// Text width inside the right column's panels.
fn side_text_width(area: Rect, cell: CellSize) -> u16 {
    let (_, side_width) = columns(area, cell);
    side_width.saturating_sub(SIDE_CHROME_WIDTH)
}

/// Text rows of the Status panel: whose turn, the Jev vs Jev pace line, the thinking line
/// and a message that may wrap once. Fixed per mode and height, so the panels below never
/// jump when a message comes or goes.
fn status_rows(mode: Mode, height: u16) -> u16 {
    let roomy = height >= 24;
    match (mode, roomy) {
        (Mode::JevVsJev, true) => 5,
        (Mode::JevVsJev, false) | (_, true) => 4,
        (_, false) => 3,
    }
}

/// Lays out the playing screen in `area` (not [`is_too_small`]) for the font `cell`, using
/// every cell of it. The Status panel gets `status_rows` text rows and Captured
/// [`CAPTURED_ROWS`]; the Jev panel, when `jev_rows` is given, gets that many (at least
/// [`JEV_MIN_ROWS`]) as long as Moves keeps [`MOVES_MIN_ROWS`]; Moves gets the rest.
fn playing_layout(
    area: Rect,
    cell: CellSize,
    status_rows: u16,
    jev_rows: Option<u16>,
) -> PlayingLayout {
    let (board_width, side_width) = columns(area, cell);
    let command_height = COMMAND_HEIGHT.min(area.height);
    let board = Rect::new(area.x, area.y, board_width, area.height - command_height);
    let command = Rect::new(area.x, board.bottom(), board_width, command_height);

    let right = Rect::new(board.right(), area.y, side_width, area.height);
    let panels: u16 = if jev_rows.is_some() { 4 } else { 3 };
    // Neighbouring panels share a border row.
    let text_rows = area.height.saturating_sub(panels + 1);
    let fixed = status_rows + CAPTURED_ROWS;
    let jev_rows = jev_rows.map(|wanted| {
        let most = text_rows.saturating_sub(fixed + MOVES_MIN_ROWS).max(1);
        wanted.max(JEV_MIN_ROWS).min(most)
    });
    let moves_rows = text_rows
        .saturating_sub(fixed + jev_rows.unwrap_or(0))
        .max(1);
    let mut top = right.y;
    let mut next = |rows: u16| {
        let rect = Rect::new(right.x, top, right.width, rows.saturating_add(2));
        top = rect.bottom().saturating_sub(1);
        rect.intersection(right)
    };
    let status = next(status_rows);
    let jev = jev_rows.map(&mut next);
    let moves = next(moves_rows);
    let captured = next(CAPTURED_ROWS);
    PlayingLayout {
        board,
        command,
        status,
        jev,
        moves,
        captured,
    }
}

/// The board, the command box and the side panels. `overlays` are the boxes drawn over
/// them afterwards (see [`overlays`]).
fn playing(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    images: &mut PieceImages,
    overlays: &[Rect],
    now: Instant,
    drawn: &mut Drawn,
) {
    let text_width = side_text_width(area, app.cell_size());
    let jev = (app.mode() != Mode::HumanVsHuman)
        .then(|| JevText::new(app.last_computer(), app.engine_status(), text_width));
    let layout = playing_layout(
        area,
        app.cell_size(),
        status_rows(app.mode(), area.height),
        jev.as_ref().map(JevText::rows),
    );
    drawn.hits.board = board_panel(frame, layout.board, app, images, overlays);
    command_panel(
        frame,
        layout.command,
        app.command_editor(),
        app.command_has_focus(),
    );
    drawn.hits.push(layout.command, Hit::CommandBox);
    status_panel(frame, layout.status, app, now);
    if let (Some(rect), Some(jev)) = (layout.jev, jev) {
        let block = side_block(app.computer_name());
        let rows = block.inner(rect).height;
        side_panel(frame, rect, block, jev.into_lines(rows));
    }
    drawn.move_scroll = moves_panel(
        frame,
        layout.moves,
        &move_rows(app.game()),
        app.move_scroll(),
    );
    drawn.hits.push(layout.moves, Hit::MoveList);
    side_panel(
        frame,
        layout.captured,
        side_block("Captured"),
        captured_lines(app.game(), app.glyphs(), app.palette()),
    );
}

/// The Board block with the board centred in it, its pictures clear of `overlays`; returns
/// the geometry for hit-testing.
fn board_panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    images: &mut PieceImages,
    overlays: &[Rect],
) -> Option<BoardGeometry> {
    let block = Block::bordered()
        .title(" Board ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let geometry = layout_board(inner, app.flipped(), app.cell_size())?;
    let highlights = app.highlights();
    frame.render_stateful_widget(
        BoardView {
            position: app.game().position(),
            geometry,
            glyphs: app.glyphs(),
            palette: app.palette(),
            highlights: &highlights,
            no_color: app.no_color(),
            picker: app.picker(),
            overlays,
        },
        inner,
        images,
    );
    Some(geometry)
}

/// The command box: a hint while idle, else the text with the terminal cursor when
/// `focused`. Unfocused text (kept after a click elsewhere) is dimmed.
fn command_panel(frame: &mut Frame, area: Rect, editor: &LineEditor, focused: bool) {
    let border = if focused {
        Style::new().fg(ACCENT).bold()
    } else {
        Style::new()
    };
    let block = Block::bordered()
        .title(" Command ")
        .border_style(border)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    if !focused && editor.is_empty() {
        frame.render_widget(Line::from("/ move  : command  ? help").dim(), inner);
        return;
    }
    let (text, cursor) = visible_input(editor, inner.width.saturating_sub(PROMPT_WIDTH));
    let line = Line::from(vec![Span::raw(PROMPT).bold(), Span::raw(text)]);
    frame.render_widget(if focused { line } else { line.dim() }, inner);
    if focused {
        frame.set_cursor_position((inner.x + PROMPT_WIDTH + cursor, inner.y));
    }
}

/// Status: the mode on the top border (the first of [`App::mode_labels`] that fits, so
/// a narrow panel shows `You (W) vs Local` rather than nothing) always keeps the
/// border; `DEBUG` joins it there too when both fit. When they do not, the mode title
/// still keeps the border and `DEBUG` starts the first status line instead ([`status_lines`]).
/// Then comes whose turn it is, the Jev vs Jev pace, the thinking spinner and the
/// latest message.
fn status_panel(frame: &mut Frame, area: Rect, app: &App, now: Instant) {
    const TITLE: &str = " Status ";
    let block = side_block("Status");
    // Two corners and at least two border cells between the title and the mode.
    let room = usize::from(area.width).saturating_sub(TITLE.len() + 4);
    let mode = app
        .mode_labels()
        .into_iter()
        .map(|label| format!(" {label} "))
        .find(|mode| Span::raw(mode.as_str()).width() <= room);
    // DEBUG only takes the border when the mode title (chosen above, without shrinking
    // for DEBUG) still fits alongside it; the mode title never loses its spot to DEBUG.
    let debug_on_border = app.debug_mode()
        && mode.as_ref().is_some_and(|mode| {
            let both = TITLE.len() + DEBUG_TAG.len() + 1 + Span::raw(mode.as_str()).width() + 4;
            both <= usize::from(area.width)
        });
    let mut block = block;
    if debug_on_border {
        block = block.title_top(Line::from(DEBUG_TAG).yellow().bold());
    }
    let block = match mode {
        Some(mode) => block.title_top(Line::from(mode).right_aligned()),
        None => block,
    };
    let inner = block.inner(area);
    let debug_in_lines = app.debug_mode() && !debug_on_border;
    let lines = status_lines(app, now, inner.width, inner.height, debug_in_lines);
    side_panel(frame, area, block, lines);
}

/// `full` when it fits on one row of `width` cells, else `brief`.
fn fitted(full: String, brief: String, width: u16) -> String {
    if Span::raw(full.as_str()).width() <= usize::from(width) {
        full
    } else {
        brief
    }
}

/// The Status panel's text for `rows` rows `width` cells wide, steadiest first, so a long
/// message is what gets cut when the panel is full: whose turn it is (`DEBUG` in front of
/// it when `debug_first`, because the Status border had no room for both titles), the Jev
/// vs Jev pace (the only place the pause and step delay are shown), the thinking spinner
/// (or the wait for an earlier request), and the latest message in the rows left
/// ([`fit_message`]). The turn and thinking lines drop words rather than wrap (the
/// computer's name "Local search" is long), so they keep one row each; a game's outcome
/// has no brief form and may wrap, and the message gets the rows its wrapped lines leave.
fn status_lines(
    app: &App,
    now: Instant,
    width: u16,
    rows: u16,
    debug_first: bool,
) -> Vec<Line<'static>> {
    let game = app.game();
    let in_check = game.outcome().is_none() && game.position().is_check();
    const DEBUG_PREFIX: &str = "DEBUG ";
    let prefix_width = if debug_first {
        u16::try_from(Span::raw(DEBUG_PREFIX).width()).unwrap_or(u16::MAX)
    } else {
        0
    };
    let turn_text = fitted(
        app.turn_text(),
        app.turn_text_brief(),
        width.saturating_sub(prefix_width),
    );
    let turn_style = if in_check {
        Style::new().bold().red()
    } else {
        Style::new().bold()
    };
    let mut spans = Vec::new();
    if debug_first {
        spans.push(Span::raw(DEBUG_PREFIX).yellow().bold());
    }
    spans.push(Span::styled(turn_text, turn_style));
    let mut lines = vec![Line::from(spans)];
    if app.mode() == Mode::JevVsJev && game.outcome().is_none() {
        let pace = if app.paused() {
            "paused · space resumes".to_string()
        } else {
            format!(
                "step {:.1} s · space pauses",
                app.step_delay().as_secs_f32()
            )
        };
        lines.push(Line::from(pace).dim());
    }
    if let Some(thinking) = app.thinking(now) {
        let (spinner, computer) = (thinking.spinner, app.computer_name());
        let seconds = thinking.elapsed.as_secs_f32();
        let full = format!("{spinner} {computer} thinking... {seconds:.1}s");
        let brief = format!("{spinner} {computer}... {seconds:.1}s");
        lines.push(Line::from(fitted(full, brief, width)).yellow());
    } else if app.waiting_for_engine(now) {
        lines.push(Line::from(WAITING_FOR_ENGINE).yellow());
    }
    // Rows, not lines: an outcome ("Insufficient material — draw (1/2-1/2)") has no brief
    // form and may wrap.
    let used = lines
        .iter()
        .map(|line| wrapped_height(&line.to_string(), width))
        .fold(0, u16::saturating_add);
    if let Some(message) = app.message()
        && rows > used
    {
        let text = fit_message(&message.text, message.path.as_deref(), width, rows - used);
        let line = Line::from(text);
        lines.push(if message.is_error { line.red() } else { line });
    }
    lines
}

/// `text` followed by `path` if they wrap into `rows` rows of `width` cells. Otherwise the
/// path loses folders from the middle of its folder part first (`~/…/chess/game.pgn`, then
/// `~/…/game.pgn`, then `…/game.pgn`), so its start and file name stay; only when even that
/// does not fit is the end cut, marked with [`ELLIPSIS`]. Without a path, [`fit_rows`].
///
/// Linear in the length of the message for a given panel: the shortened forms share the
/// wrap of their start, and only those whose characters could fit at all are wrapped.
fn fit_message(text: &str, path: Option<&str>, width: u16, rows: u16) -> String {
    let Some(path) = path else {
        return fit_rows(text, width, rows);
    };
    let full = format!("{text}{path}");
    if wrapped_height(&full, width) <= rows {
        return full;
    }
    let Some(parts) = PathParts::new(path) else {
        return cut_to_fit(&full, width, rows);
    };
    let limit = usize::from(rows);
    // Every non-blank cell takes a cell of some row, so a form with more cannot fit.
    let room = usize::from(width.max(1)) * limit;
    let separator = solid_width("/");
    // `{text}{head}/…`, then `/{folder}` for each kept folder, then `/{name}`.
    let start = format!("{text}{}/{ELLIPSIS}", parts.head);
    let fixed = solid_width(&start) + separator + solid_width(parts.name);
    let mut wrapped_start = Wrapper::new(width);
    wrapped_start.push_str(&start);
    let mut kept_width = vec![0; parts.folders.len()];
    for kept in 1..parts.folders.len() {
        let folder = parts.folders[parts.folders.len() - kept];
        kept_width[kept] = kept_width[kept - 1] + separator + solid_width(folder);
    }
    for kept in (0..parts.folders.len()).rev() {
        if fixed + kept_width[kept] > room {
            continue;
        }
        let tail = parts.tail(kept);
        let mut wrapped = wrapped_start;
        wrapped.push_str(&tail);
        if wrapped.rows() <= limit {
            return format!("{start}{tail}");
        }
    }
    let shortest = format!("{text}{ELLIPSIS}/{}", parts.name);
    if wrapped_height(&shortest, width) <= rows {
        return shortest;
    }
    cut_to_fit(&shortest, width, rows)
}

/// A path split the way [`fit_message`] shortens it: its first folder (`~`, `/tmp`,
/// `games`), the folders after it, and the file name.
struct PathParts<'a> {
    head: String,
    folders: Vec<&'a str>,
    name: &'a str,
}

impl<'a> PathParts<'a> {
    /// `None` for a path without a folder.
    fn new(path: &'a str) -> Option<PathParts<'a>> {
        let (folders, name) = path.rsplit_once('/')?;
        let mut folders = folders.split('/');
        let first = folders.next().unwrap_or_default();
        let mut folders: Vec<&str> = folders.collect();
        // An absolute path's first folder keeps its leading slash: `/tmp`, not ``.
        let head = if first.is_empty() && !folders.is_empty() {
            format!("/{}", folders.remove(0))
        } else {
            first.to_string()
        };
        Some(PathParts {
            head,
            folders,
            name,
        })
    }

    /// The end of a shortened form that keeps the last `kept` folders:
    /// `/{folder}` for each, then `/{name}`.
    fn tail(&self, kept: usize) -> String {
        let mut tail = String::new();
        for folder in &self.folders[self.folders.len() - kept..] {
            tail.push('/');
            tail.push_str(folder);
        }
        tail.push('/');
        tail.push_str(self.name);
        tail
    }
}

/// Cells of `text` outside its whitespace: at least this many cells of any rows it
/// wraps into are taken.
fn solid_width(text: &str) -> usize {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .map(char_width)
        .sum()
}

/// The Jev panel's text, laid out for one width: lines that always show (what was played
/// and by whom, Jev's top three, confidence · latency · model), then the note, which is the
/// only part cut short when the panel has fewer rows than [`JevText::rows`].
struct JevText {
    lines: Vec<Line<'static>>,
    note: Option<String>,
    width: u16,
}

impl JevText {
    /// The text for the last computer move, or the engine status before there is one.
    /// Each line is packed to `width` so it breaks between items, never inside one.
    fn new(computer: Option<&ComputerMove>, engine_status: &str, width: u16) -> JevText {
        let Some(computer) = computer else {
            return JevText {
                lines: vec![
                    Line::from(engine_status.to_string()).dim(),
                    Line::from("no move yet").dim(),
                ],
                note: None,
                width,
            };
        };
        // A veto's note already names Jev's pick ("Jev picked Qh4, which ..."), so the
        // source says just "vetoed" then, which saves a row on narrow panels.
        let source = match (&computer.source, &computer.note) {
            (MoveSource::Vetoed { .. }, Some(_)) => "vetoed".to_string(),
            (source, _) => source.to_string(),
        };
        let played = vec![
            vec![Span::raw("played "), Span::raw(computer.san.clone()).bold()],
            vec![Span::raw(source)],
        ];
        let mut lines = pack(played, " · ", width);
        let top: Vec<Vec<Span<'static>>> = computer
            .top
            .iter()
            .take(3)
            .map(|(san, probability)| vec![Span::raw(format!("{san} {:.0}%", probability * 100.0))])
            .collect();
        lines.extend(pack(top, "  ", width));
        let mut details = Vec::new();
        if let Some(confidence) = computer.confidence {
            details.push(vec![Span::raw(format!("conf {confidence:.2}"))]);
        }
        details.push(vec![Span::raw(format!(
            "{} ms",
            computer.latency.as_millis()
        ))]);
        if let Some(model) = &computer.model {
            details.push(vec![Span::raw(model.clone())]);
        }
        lines.extend(pack(details, " · ", width).into_iter().map(Line::dim));
        JevText {
            lines,
            note: computer.note.clone(),
            width,
        }
    }

    /// Text rows needed to show everything.
    fn rows(&self) -> u16 {
        self.fixed_rows().saturating_add(
            self.note
                .as_deref()
                .map_or(0, |note| wrapped_height(note, self.width)),
        )
    }

    fn fixed_rows(&self) -> u16 {
        self.lines
            .iter()
            .map(|line| wrapped_height(&line.to_string(), self.width))
            .fold(0, u16::saturating_add)
    }

    /// The lines for a panel `rows` text rows high: the note is cut to the rows left.
    fn into_lines(self, rows: u16) -> Vec<Line<'static>> {
        let room = rows.saturating_sub(self.fixed_rows());
        let mut lines = self.lines;
        if let Some(note) = self.note
            && room > 0
        {
            lines.push(Line::from(fit_rows(&note, self.width, room)).yellow());
        }
        lines
    }
}

/// Joins `segments` with `separator` into lines at most `width` cells wide, starting a new
/// line instead of splitting a segment. A segment wider than `width` gets a line of its own
/// (the paragraph wraps it).
fn pack(
    segments: Vec<Vec<Span<'static>>>,
    separator: &'static str,
    width: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let separator_width = Span::raw(separator).width();
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for segment in segments {
        let segment_width: usize = segment.iter().map(Span::width).sum();
        if !current.is_empty() && used + separator_width + segment_width <= width {
            current.push(Span::raw(separator));
            used += separator_width + segment_width;
        } else {
            if !current.is_empty() {
                lines.push(Line::from(std::mem::take(&mut current)));
            }
            used = segment_width;
        }
        current.extend(segment);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

/// `text` if it wraps into `rows` rows of `width` cells, else its longest start that does
/// with [`ELLIPSIS`] appended, cut between words when any such cut fits. Linear in the
/// length of `text`: every cut is measured in one pass ([`Wrapper`]).
fn fit_rows(text: &str, width: u16, rows: u16) -> String {
    if wrapped_height(text, width) <= rows {
        return text.to_string();
    }
    let limit = usize::from(rows);
    let mut wrapper = Wrapper::new(width);
    let mut best = None;
    for (end, c) in text.char_indices() {
        if c.is_whitespace() && wrapper.rows_with_ellipsis() <= limit {
            best = Some(end);
        }
        wrapper.push(c);
    }
    match best {
        Some(end) => format!("{}{ELLIPSIS}", text[..end].trim_end()),
        None => cut_to_fit(text, width, rows),
    }
}

/// The longest start of `text` that wraps into `rows` rows of `width` cells with
/// [`ELLIPSIS`] appended, cut between any two characters. Linear in the length of `text`.
fn cut_to_fit(text: &str, width: u16, rows: u16) -> String {
    let limit = usize::from(rows);
    let mut wrapper = Wrapper::new(width);
    let mut best = None;
    for (end, c) in text.char_indices() {
        if wrapper.rows_with_ellipsis() <= limit {
            best = Some(end);
        }
        wrapper.push(c);
    }
    match best {
        Some(end) => format!("{}{ELLIPSIS}", text[..end].trim_end()),
        None => ELLIPSIS.to_string(),
    }
}

/// Narrowest column for White's moves: `e4` or `Nf3` and at least two spaces, as in the
/// spec's `1. e4   e5` / `2. Nf3  Nc6`.
const WHITE_MOVE_COLUMN: usize = 5;

/// A [`move_rows`] row as its move number, White's move (`...` when the game starts with
/// Black to move) and Black's move: `12... Kd7` gives `("12", "...", Some("Kd7"))`.
fn split_move_row(row: &str) -> (&str, &str, Option<&str>) {
    let (number, moves) = row.split_once(' ').unwrap_or((row, ""));
    if let Some(number) = number.strip_suffix("...") {
        return (number, "...", Some(moves));
    }
    let number = number.strip_suffix('.').unwrap_or(number);
    match moves.split_once(' ') {
        Some((white, black)) => (number, white, Some(black)),
        None => (number, moves, None),
    }
}

/// The move list, scrolled `scroll` rows up from the latest (clamped and returned). Move
/// numbers are right-aligned, White's moves are padded to one width (at least
/// [`WHITE_MOVE_COLUMN`], wider when a long move needs it) so Black's line up, and the
/// latest row is bold.
fn moves_panel(frame: &mut Frame, area: Rect, rows: &[String], scroll: usize) -> usize {
    let mut block = side_block("Moves");
    let visible = usize::from(block.inner(area).height);
    let scroll = scroll.min(rows.len().saturating_sub(visible));
    if scroll > 0 {
        block = block.title_top(Line::from(format!(" +{scroll} below ")).right_aligned());
    }
    if rows.is_empty() {
        side_panel(frame, area, block, vec![Line::from("no moves yet").dim()]);
        return 0;
    }
    let split: Vec<_> = rows.iter().map(|row| split_move_row(row)).collect();
    let number_width = split.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
    let white_width = split
        .iter()
        .map(|(_, white, _)| Span::raw(*white).width() + 2)
        .max()
        .unwrap_or(0)
        .max(WHITE_MOVE_COLUMN);
    let end = rows.len() - scroll;
    let start = end.saturating_sub(visible);
    let lines: Vec<Line> = split[start..end]
        .iter()
        .zip(start..)
        .map(|(&(number, white, black), index)| {
            let text = match black {
                Some(black) => format!("{number:>number_width$}. {white:<white_width$}{black}"),
                None => format!("{number:>number_width$}. {white}"),
            };
            let line = Line::from(text);
            if index + 1 == rows.len() {
                line.bold()
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
    scroll
}

/// One row per side: the pieces it has captured, on a light-square tray so both piece
/// colours show, and its material lead.
fn captured_lines(game: &Game, glyph_set: GlyphSet, palette: &Palette) -> Vec<Line<'static>> {
    let taken = captured_pieces(game);
    let balance = material_balance(game.position());
    Side::ALL
        .into_iter()
        .map(|side| {
            // `Side`'s Display ignores padding, so pad its string.
            let mut spans = vec![Span::raw(format!("{:<6}", side.to_string()))];
            let pieces = &taken[side.index()];
            if !pieces.is_empty() {
                let tray = Style::new().bg(palette.light);
                spans.push(Span::styled(" ", tray));
                spans.extend(
                    pieces
                        .iter()
                        .map(|&piece| piece_span(piece, glyph_set, palette)),
                );
                spans.push(Span::styled(" ", tray));
            }
            let lead = match side {
                Side::White => balance,
                Side::Black => -balance,
            };
            if lead > 0 {
                spans.push(Span::raw(format!(" +{lead}")).bold());
            }
            Line::from(spans)
        })
        .collect()
}

/// Pieces each side captured in `game`, most valuable first: `[0]` is what White took
/// (Black's pieces), `[1]` what Black took.
fn captured_pieces(game: &Game) -> [Vec<Piece>; 2] {
    let mut taken = [Vec::new(), Vec::new()];
    for (&mv, pos) in game.moves().iter().zip(game.positions()) {
        if !mv.is_capture() {
            continue;
        }
        let mover = pos.side_to_move();
        let victim = if mv.is_en_passant() {
            Some(Piece::new(!mover, PieceKind::Pawn))
        } else {
            pos.piece_at(mv.to())
        };
        if let Some(victim) = victim {
            taken[mover.index()].push(victim);
        }
    }
    for pieces in &mut taken {
        pieces.sort_by_key(|piece| Reverse(piece.kind.index()));
    }
    taken
}

/// White's material minus Black's, in pawns (knight and bishop 3, rook 5, queen 9).
fn material_balance(pos: &ChessPosition) -> i32 {
    PieceKind::ALL
        .into_iter()
        .map(|kind| {
            let count = |side| i32::try_from(pos.pieces_of(side, kind).count()).unwrap_or(0);
            PIECE_VALUES[kind.index()] * (count(Side::White) - count(Side::Black))
        })
        .sum()
}

/// A right-column block: titled, sharing borders with its neighbours, one blank column
/// on the left.
fn side_block(title: &str) -> Block<'static> {
    Block::bordered()
        .title(format!(" {title} "))
        .merge_borders(MergeStrategy::Exact)
        .padding(Padding::left(1))
}

/// Draws `lines`, word-wrapped, in `block` at `area`.
fn side_panel(frame: &mut Frame, area: Rect, block: Block, lines: Vec<Line<'static>>) {
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        area,
    );
}

/// `piece` in its colour on a light square, like on the board.
fn piece_span(piece: Piece, glyph_set: GlyphSet, palette: &Palette) -> Span<'static> {
    let fg = match piece.color {
        Side::White => palette.white_piece,
        Side::Black => palette.black_piece,
    };
    Span::styled(
        glyphs::glyph(glyph_set, piece),
        Style::new().fg(fg).bg(palette.light),
    )
}

// ----- exchange view -----

/// The Jev exchange view on the whole of `area` (spec 9.4): the exchange `view` shows,
/// with a header saying which one it is and how it went, then its body, wrapped and
/// scrolled; or [`NO_EXCHANGES`]. Returns `view` with its scroll clamped and its page size
/// and scroll limit for this size. `bodies` keeps the body text rendered and cut into rows
/// for the next frame.
fn exchange_screen(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    view: ExchangeView,
    bodies: &mut BodyCache,
) -> ExchangeView {
    let block = Block::bordered()
        .title(Line::from(" Jev exchange ").bold())
        .title_bottom(Line::from(EXCHANGE_KEYS).dim())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let history = app.exchanges();
    let shown = history.and_then(|history| {
        let index = view.index(history)?;
        Some((index, history.len(), history.get(index)?))
    });
    let Some((index, count, record)) = shown else {
        frame.render_widget(block, area);
        frame.render_widget(Line::from(NO_EXCHANGES).dim(), row_of(inner, 0));
        return ExchangeView {
            scroll: 0,
            page: usize::from(inner.height),
            max_scroll: 0,
            ..view
        };
    };
    let header = pack(exchange_header(record, index, count), " · ", inner.width);
    let header_rows = header
        .iter()
        .map(|line| wrapped_height(&line.to_string(), inner.width))
        .fold(0u16, u16::saturating_add)
        .min(inner.height);
    // One blank row between the header and the body.
    let body_top = header_rows.saturating_add(1).min(inner.height);
    let body_area = Rect {
        y: inner.y + body_top,
        height: inner.height - body_top,
        ..inner
    };
    let width = usize::from(body_area.width.max(1));
    let page = usize::from(body_area.height);
    let body = bodies.rows(record, width);
    let total = body.len();
    let max_scroll = total.saturating_sub(page);
    let scroll = view.scroll.min(max_scroll);
    let block = if total > page {
        let last = (scroll + page).min(total);
        block.title_top(Line::from(format!(" {}-{last} of {total} ", scroll + 1)).right_aligned())
    } else {
        block
    };
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: true }),
        Rect {
            height: header_rows,
            ..inner
        },
    );
    frame.render_widget(Paragraph::new(body_rows(&body, scroll, page)), body_area);
    ExchangeView {
        scroll,
        page,
        max_scroll,
        ..view
    }
}

/// The exchange view's header for `record`, the `index`th of `count` (from 0), as items
/// for [`pack`]: `exchange N of M`, `move <fullmove>`, the move, how it was chosen, the
/// last status, the attempts and the latency, and `stale — not played` when it was not
/// (`held — not played yet` while Jev vs Jev is paused with it).
fn exchange_header(record: &Record, index: usize, count: usize) -> Vec<Vec<Span<'static>>> {
    let exchange = &record.exchange;
    let attempts = match exchange.attempts() {
        1 => "1 attempt".to_string(),
        n => format!("{n} attempts"),
    };
    let mut items = vec![
        vec![Span::raw(format!("exchange {} of {count}", index + 1)).bold()],
        vec![Span::raw(format!("move {}", exchange.fullmove))],
        vec![Span::raw(exchange.san.clone()).bold()],
        vec![Span::raw(exchange.source.clone())],
        vec![Span::raw(exchange.status())],
        vec![Span::raw(attempts)],
        vec![Span::raw(format!("{} ms", exchange.latency.as_millis()))],
    ];
    if record.stale {
        items.push(vec![Span::raw("stale — not played").yellow()]);
    } else if record.held {
        items.push(vec![Span::raw("held — not played yet").yellow()]);
    }
    items
}

/// The rows of an exchange's `body` from row `scroll`, at most `page` of them.
fn body_rows(body: &BodyRows, scroll: usize, page: usize) -> Vec<Line<'static>> {
    let end = scroll.saturating_add(page).min(body.len());
    (scroll.min(end)..end)
        .filter_map(|index| body.get(index))
        .map(|(kind, text)| {
            let style = match kind {
                LineKind::Heading => Style::new().bold(),
                LineKind::Error => Style::new().red(),
                LineKind::Text => Style::new(),
            };
            Line::styled(text.to_string(), style)
        })
        .collect()
}

// ----- overlays -----

/// Where the boxes drawn over the playing screen go in `area`: the game-over box (on its
/// screen, once the game has an outcome) and the top dialog. The board keeps its pictures
/// clear of them ([`BoardView::overlays`]).
fn overlays(app: &App, area: Rect) -> Vec<Rect> {
    let game_over = (app.screen() == Screen::GameOver && app.game().outcome().is_some())
        .then(|| game_over_rect(area));
    let dialog = app.dialog().map(|top| dialog_rect(area, top));
    game_over.into_iter().chain(dialog).collect()
}

/// Where the game-over box goes in `area`.
fn game_over_rect(area: Rect) -> Rect {
    centered(area, 46, 7)
}

/// The game-over overlay: the result and the New game / Save PGN / Menu buttons.
fn game_over(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let Some(outcome) = app.game().outcome() else {
        return;
    };
    let inner = dialog_frame(frame, game_over_rect(area), "Game over");
    frame.render_widget(
        Line::from(outcome_text(outcome)).bold().centered(),
        row_of(inner, 0),
    );
    frame.render_widget(
        Line::from(format!("Result {}", outcome.result())).centered(),
        row_of(inner, 1),
    );
    let buttons = GAME_OVER_BUTTONS
        .iter()
        .enumerate()
        .map(|(index, &button)| {
            let selected = index == app.game_over_choice();
            (
                Hit::Button(button),
                text_button(button_label(button), selected),
            )
        })
        .collect();
    render_buttons(frame, row_of(inner, 3), buttons, hits);
    // While the command box has focus, keys are typed into it, not given to the overlay.
    let hint = if app.command_has_focus() {
        "Esc: leave the command box"
    } else {
        "Esc: see the board   u: undo"
    };
    frame.render_widget(Line::from(hint).dim().centered(), row_of(inner, 4));
}

/// Where the dialog `top` goes in `area`.
fn dialog_rect(area: Rect, top: &Dialog) -> Rect {
    match top {
        Dialog::Help => {
            let height = u16::try_from(HELP_LINES.len()).map_or(u16::MAX, |n| n.saturating_add(2));
            centered(area, 60, height)
        }
        Dialog::Promotion { .. } => centered(area, 58, 5),
        Dialog::Input { .. } => centered(area, 66, 8),
        Dialog::Confirm { question, .. } => {
            let text_width = CONFIRM_WIDTH.saturating_sub(4);
            let text_rows = confirm_lines(question)
                .iter()
                .map(|line| wrapped_height(line, text_width))
                .fold(0u16, u16::saturating_add);
            // Borders, text, gap, buttons.
            centered(area, CONFIRM_WIDTH, text_rows.saturating_add(4))
        }
    }
}

/// Draws the top dialog and records its targets (none for help: any click closes it).
fn dialog(frame: &mut Frame, area: Rect, app: &App, top: &Dialog, hits: &mut HitMap) {
    let rect = dialog_rect(area, top);
    match top {
        Dialog::Help => help(frame, rect),
        Dialog::Promotion { choice, .. } => promotion(frame, rect, *choice, app, hits),
        Dialog::Input {
            purpose,
            editor,
            error,
        } => input(
            frame,
            rect,
            top.title(),
            *purpose,
            editor,
            error.as_ref(),
            hits,
        ),
        Dialog::Confirm { question, yes } => {
            confirm(frame, rect, top.title(), question, *yes, hits)
        }
    }
}

/// Keys and commands, in `rect`.
fn help(frame: &mut Frame, rect: Rect) {
    frame.render_widget(Clear, rect);
    let block =
        dialog_block("Help").title_bottom(Line::from(" Esc or click closes ").right_aligned());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let lines: Vec<Line> = HELP_LINES
        .iter()
        .map(|&text| {
            let split = text
                .char_indices()
                .nth(HELP_KEY_WIDTH)
                .map_or(text.len(), |(at, _)| at);
            let (key, rest) = text.split_at(split);
            Line::from(vec![Span::raw(key).bold(), Span::raw(rest)])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The promotion picker in `rect`: Queen, Rook, Bishop and Knight buttons in the colour
/// of the side to move (the one promoting), `choice` highlighted.
fn promotion(frame: &mut Frame, rect: Rect, choice: usize, app: &App, hits: &mut HitMap) {
    let side = app.game().position().side_to_move();
    let inner = dialog_frame(frame, rect, "Promote to");
    let buttons = PROMOTION_CHOICES
        .iter()
        .enumerate()
        .map(|(index, &kind)| {
            let text = if index == choice {
                Style::new().reversed().bold()
            } else {
                Style::new()
            };
            let label = Line::from(vec![
                Span::styled("[ ", text),
                piece_span(Piece::new(side, kind), app.glyphs(), app.palette()),
                Span::styled(format!(" {} ]", piece_name(kind)), text),
            ]);
            (Hit::Promote(kind), label)
        })
        .collect();
    render_buttons(frame, row_of(inner, 0), buttons, hits);
    frame.render_widget(
        Line::from("q r b n, arrows and Enter, or click; Esc cancels")
            .dim()
            .centered(),
        row_of(inner, 2),
    );
}

/// A text field dialog in `rect`: prompt, field with the terminal cursor, error and
/// buttons.
fn input(
    frame: &mut Frame,
    rect: Rect,
    title: &str,
    purpose: InputPurpose,
    editor: &LineEditor,
    error: Option<&Message>,
    hits: &mut HitMap,
) {
    let inner = dialog_frame(frame, rect, title);
    let (prompt, action) = match purpose {
        InputPurpose::LoadFen { .. } => ("Paste or type a FEN:".to_string(), "Load"),
        InputPurpose::Save(kind) => (
            format!("File name (~ expands, .{} is added):", kind.extension()),
            "Save",
        ),
    };
    frame.render_widget(Line::from(prompt), row_of(inner, 0));
    let field = row_of(inner, 1);
    let (text, cursor) = visible_input(editor, field.width.saturating_sub(PROMPT_WIDTH));
    frame.render_widget(
        Line::from(vec![Span::raw(PROMPT).bold(), Span::raw(text)]),
        field,
    );
    if !field.is_empty() {
        frame.set_cursor_position((field.x + PROMPT_WIDTH + cursor, field.y));
    }
    if let Some(error) = error {
        let rows =
            Rect::new(inner.x, inner.y.saturating_add(3), inner.width, 2).intersection(inner);
        let text = fit_message(&error.text, error.path.as_deref(), rows.width, rows.height);
        frame.render_widget(Paragraph::new(text).red().wrap(Wrap { trim: true }), rows);
    }
    let buttons = vec![
        (Hit::Button(Button::Confirm), text_button(action, false)),
        (
            Hit::Button(Button::Cancel),
            text_button(button_label(Button::Cancel), false),
        ),
    ];
    render_buttons(frame, last_row(inner), buttons, hits);
}

/// Width of the yes/no question dialog.
const CONFIRM_WIDTH: u16 = 56;

/// The text of a yes/no question, one entry per paragraph.
fn confirm_lines(question: &Question) -> Vec<String> {
    let text = question.text();
    match question {
        // The path on its own rows, so a long one never splits the question.
        Question::Overwrite { path, .. } => {
            let path = path.display().to_string();
            let rest = text.strip_prefix(&path).unwrap_or(&text).trim_start();
            let rest = rest.to_string();
            vec![path, rest]
        }
        Question::Quit | Question::Resign | Question::NewGame | Question::Menu => vec![text],
    }
}

/// A yes/no question in `rect`, which [`dialog_rect`] makes grow with the question's text.
fn confirm(
    frame: &mut Frame,
    rect: Rect,
    title: &str,
    question: &Question,
    yes: bool,
    hits: &mut HitMap,
) {
    let lines = confirm_lines(question);
    let inner = dialog_frame(frame, rect, title);
    let text_area = Rect {
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text_area);
    let buttons = vec![
        (Hit::Button(Button::Confirm), text_button("Yes", yes)),
        (Hit::Button(Button::Cancel), text_button("No", !yes)),
    ];
    render_buttons(frame, last_row(inner), buttons, hits);
}

/// Clears `rect`, draws a dialog border titled `title` there and returns its inside.
fn dialog_frame(frame: &mut Frame, rect: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, rect);
    let block = dialog_block(title);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

/// A dialog's block: accent border, bold title, one blank column each side.
fn dialog_block(title: &str) -> Block<'static> {
    Block::bordered()
        .title(Line::from(format!(" {title} ")).bold())
        .border_style(Style::new().fg(ACCENT))
        .padding(Padding::horizontal(1))
}

/// A button's usual label; the input and question dialogs name their confirm and cancel
/// buttons after what they do instead.
const fn button_label(button: Button) -> &'static str {
    match button {
        Button::Confirm => "OK",
        Button::Cancel => "Cancel",
        Button::NewGame => "New game",
        Button::SavePgn => "Save PGN",
        Button::Menu => "Menu",
    }
}

/// `[ label ]`, reversed when selected.
fn text_button(label: &str, selected: bool) -> Line<'static> {
    let line = Line::from(format!("[ {label} ]"));
    if selected {
        line.reversed().bold()
    } else {
        line
    }
}

/// Draws `buttons` centred on the one-row `row`, two cells apart, and records each one.
fn render_buttons(frame: &mut Frame, row: Rect, buttons: Vec<(Hit, Line)>, hits: &mut HitMap) {
    const GAP: u16 = 2;
    let width = |line: &Line| u16::try_from(line.width()).unwrap_or(u16::MAX);
    let total = buttons
        .iter()
        .fold(0u16, |sum, (_, line)| {
            sum.saturating_add(width(line)).saturating_add(GAP)
        })
        .saturating_sub(GAP);
    let mut x = row.x.saturating_add(row.width.saturating_sub(total) / 2);
    for (hit, line) in buttons {
        let w = width(&line);
        let rect = Rect::new(x, row.y, w, row.height.min(1)).intersection(row);
        frame.render_widget(line, rect);
        hits.push(rect, hit);
        x = x.saturating_add(w).saturating_add(GAP);
    }
}

// ----- geometry and text helpers -----

/// The part of `editor`'s text that fits in `width` cells with the cursor visible, and the
/// cursor's column within it. Wide characters count as two cells.
fn visible_input(editor: &LineEditor, width: u16) -> (String, u16) {
    let width = usize::from(width.max(1));
    let before = editor.before_cursor();
    let after = &editor.text()[before.len()..];
    // As much of the text before the cursor as fits, keeping one cell for the cursor.
    let mut used = 0;
    let mut shown: Vec<char> = Vec::new();
    for c in before.chars().rev() {
        let w = char_width(c);
        if used + w + 1 > width {
            break;
        }
        used += w;
        shown.push(c);
    }
    shown.reverse();
    let cursor = used;
    for c in after.chars() {
        let w = char_width(c);
        if used + w > width {
            break;
        }
        used += w;
        shown.push(c);
    }
    (
        shown.into_iter().collect(),
        u16::try_from(cursor).unwrap_or(u16::MAX),
    )
}

/// A `width` × `height` rect centred in `area` (shrunk to fit).
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Row `index` of `area` (empty when outside it).
fn row_of(area: Rect, index: u16) -> Rect {
    Rect::new(area.x, area.y.saturating_add(index), area.width, 1).intersection(area)
}

/// The bottom row of `area`.
fn last_row(area: Rect) -> Rect {
    row_of(area, area.height.saturating_sub(1))
}

/// Rows `text` takes when word-wrapped to `width` cells: greedy, like ratatui's word
/// wrapper, with words longer than a row split between characters. Widths are
/// [`char_width`]'s.
fn wrapped_height(text: &str, width: u16) -> u16 {
    let mut wrapper = Wrapper::new(width);
    wrapper.push_str(text);
    u16::try_from(wrapper.rows()).unwrap_or(u16::MAX)
}

#[cfg(test)]
thread_local! {
    /// Characters fed to every [`Wrapper`] on this thread: the work of the fitting
    /// functions, which a test bounds to keep them linear.
    static WRAPPED_CHARS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The word wrap of [`wrapped_height`], fed a character at a time, so that the rows of
/// every start of a text (with or without [`ELLIPSIS`] after it) are known in one pass.
///
/// The last word is kept apart until the next one starts: a start that ends in
/// whitespace is cut back to that word, and an ellipsis joins it.
#[derive(Clone, Copy, Debug)]
struct Wrapper {
    /// Cells per row (at least 1).
    width: usize,
    /// Rows taken by the words before `last`.
    rows: usize,
    /// Cells used on the last of those rows.
    used: usize,
    /// The last word so far.
    last: Option<Word>,
    /// Whether the last character pushed belongs to `last`.
    in_word: bool,
}

/// A word as [`Wrapper`] places it: its width, and how it splits between characters when
/// it is wider than a row.
#[derive(Clone, Copy, Debug, Default)]
struct Word {
    width: usize,
    /// Rows after the first that the word takes when split, starting a row.
    split_rows: usize,
    /// Cells used on the last of those rows.
    split_used: usize,
}

impl Word {
    /// Adds a character `cells` wide, in rows of `row` cells.
    fn push(&mut self, cells: usize, row: usize) {
        self.width += cells;
        if self.split_used > 0 && self.split_used + cells > row {
            self.split_rows += 1;
            self.split_used = 0;
        }
        self.split_used += cells;
    }
}

impl Wrapper {
    fn new(width: u16) -> Wrapper {
        Wrapper {
            width: usize::from(width.max(1)),
            rows: 1,
            used: 0,
            last: None,
            in_word: false,
        }
    }

    fn push(&mut self, c: char) {
        #[cfg(test)]
        WRAPPED_CHARS.with(|count| count.set(count.get() + 1));
        if c.is_whitespace() {
            self.in_word = false;
            return;
        }
        if !self.in_word {
            if let Some(word) = self.last.take() {
                (self.rows, self.used) = self.place(word);
            }
            self.in_word = true;
        }
        let width = self.width;
        self.last
            .get_or_insert_with(Word::default)
            .push(char_width(c), width);
    }

    fn push_str(&mut self, text: &str) {
        for c in text.chars() {
            self.push(c);
        }
    }

    /// Rows, and cells used on the last row, once `word` follows the words before `last`.
    fn place(&self, word: Word) -> (usize, usize) {
        if self.used > 0 && self.used + 1 + word.width <= self.width {
            return (self.rows, self.used + 1 + word.width);
        }
        let rows = self.rows + usize::from(self.used > 0);
        if word.width <= self.width {
            (rows, word.width)
        } else {
            (rows + word.split_rows, word.split_used)
        }
    }

    /// Rows of the text pushed so far.
    fn rows(&self) -> usize {
        self.last.map_or(self.rows, |word| self.place(word).0)
    }

    /// Rows of the text pushed so far with its trailing whitespace dropped and
    /// [`ELLIPSIS`] appended.
    fn rows_with_ellipsis(&self) -> usize {
        let mut word = self.last.unwrap_or_default();
        for c in ELLIPSIS.chars() {
            word.push(char_width(c), self.width);
        }
        self.place(word).0
    }
}

/// The piece's English name.
const fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "Pawn",
        PieceKind::Knight => "Knight",
        PieceKind::Bishop => "Bishop",
        PieceKind::Rook => "Rook",
        PieceKind::Queen => "Queen",
        PieceKind::King => "King",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::KeyCode;
    use ratatui::widgets::Widget;
    use ratatui_image::picker::ProtocolType;

    use super::*;
    use crate::core::{START_FEN, Square};
    use crate::tui::board::{image_area, square_at, square_rect};
    use crate::tui::debug::{BodyCache, DebugLog, NO_LOG_PATH};
    use crate::tui::event::AppEvent;
    use crate::tui::glyphs::{ImageSupport, initial_glyphs};
    use crate::tui::graphics::picker_for;
    use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS, traced_jev_move};
    use crate::tui::test_support::harness::Harness;
    use crate::tui::test_support::{PROMOTION_FEN, game_from, sq};
    use crate::tui::worker::EngineOutcome;
    use crate::tui::worker::LOCAL_SEARCH_STATUS;

    /// An app playing against Jev (with a key), in a `width`×`height` terminal.
    fn jev(width: u16, height: u16) -> Harness {
        Harness::sized(FakeEngine::jev(), width, height)
    }

    /// The text rows inside the panel titled `title` in the right column, trimmed.
    fn panel_rows(h: &Harness, title: &str) -> Vec<String> {
        let buffer = h.buffer();
        let area = buffer.area;
        let layout = playing_layout(
            area,
            h.app.cell_size(),
            status_rows(h.app.mode(), area.height),
            None,
        );
        let (left, right) = (layout.status.x, layout.status.right());
        let rows: Vec<String> = (0..area.height)
            .map(|y| (left..right).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let start = rows
            .iter()
            .position(|row| {
                row.starts_with(&format!("┌ {title} ")) || row.starts_with(&format!("├ {title} "))
            })
            .unwrap_or_else(|| panic!("no {title} panel:\n{}", h.screen()));
        rows[start + 1..]
            .iter()
            .take_while(|row| !row.starts_with('├') && !row.starts_with('└'))
            .map(|row| {
                row.trim_matches(|c: char| c == '│' || c.is_whitespace())
                    .to_string()
            })
            .collect()
    }

    /// The text inside the panel titled `title`, joined into one line with single spaces (so
    /// wrapped words can be searched for).
    fn panel_text(h: &Harness, title: &str) -> String {
        panel_rows(h, title)
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A copy of `area` of `buffer`, for a focused colour snapshot.
    fn crop(buffer: &Buffer, area: Rect) -> Buffer {
        let mut part = Buffer::empty(area);
        for pos in area.positions() {
            part[pos] = buffer[pos].clone();
        }
        part
    }

    /// A Human vs Human game after 1. d4 e5 2. dxe5 Bb4+, with White's b1 knight picked up
    /// by keyboard: last move, check, selection, targets and cursor are all on show.
    fn mid_game(width: u16, height: u16) -> Harness {
        let mut h = jev(width, height);
        h.char('1');
        h.moves(&["d4", "e5", "dxe5", "Bb4+"]);
        h.press(KeyCode::Up);
        for _ in 0..3 {
            h.press(KeyCode::Left);
        }
        h.press(KeyCode::Down);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.selected(), Some(sq("b1")));
        assert_eq!(h.app.targets(), [sq("d2"), sq("c3")]);
        h
    }

    // ----- layout -----

    /// A Human vs Jev game in which Jev answered 1. e4 with a vetoed move and a long note.
    fn vetoed_reply(width: u16, height: u16) -> Harness {
        let mut h = jev(width, height);
        h.char('2');
        h.moves(&["e4"]);
        h.reply_with("g8f6", |computer| {
            computer.source = MoveSource::Vetoed {
                jev_pick: "Qh4".to_string(),
            };
            computer.top = vec![
                ("Qh4".to_string(), 0.62),
                ("Nf6".to_string(), 0.21),
                ("d5".to_string(), 0.09),
            ];
            computer.latency = Duration::from_millis(12345);
            computer.model = Some("jev-latest".to_string());
            computer.note =
                Some("Jev picked Qh4, which the search rates much worse; played Nf6".to_string());
        });
        h
    }

    /// Terminal sizes the layout is checked at, from the minimum up.
    const SIZES: [(u16, u16); 5] = [(60, 20), (80, 24), (120, 40), (200, 60), (300, 100)];
    /// Font sizes (cell width × height in pixels) the layout is checked with.
    const FONTS: [(u16, u16); 3] = [(10, 20), (8, 16), (16, 32)];

    /// An app with a `cell_w`×`cell_h` pixel font in a `width`×`height` terminal, started
    /// from the menu with `key` (against Jev for '2' to '5').
    fn with_font((width, height): (u16, u16), (cell_w, cell_h): (u16, u16), key: char) -> Harness {
        let mut h = Harness::build(FakeEngine::jev(), (width, height), Vec::new(), |mut app| {
            app.set_cell_size(CellSize::new(cell_w, cell_h));
            app
        });
        h.char(key);
        h
    }

    #[test]
    fn the_board_takes_the_tallest_squares_that_fit() {
        // (terminal, font, square width and height). The three fonts are all twice as tall
        // as wide, so they agree; other shapes follow.
        let cases = [
            ((60, 20), (3, 1)),
            ((80, 24), (5, 2)),
            ((120, 40), (9, 4)),
            ((200, 60), (13, 6)),
            ((300, 100), (23, 11)),
        ];
        for ((width, height), square) in cases {
            for font in FONTS {
                let h = with_font((width, height), font, '1');
                let g = h.app.hit_map().board.expect("board drawn");
                let at = format!("{width}x{height} with a {font:?} font");
                assert_eq!((g.square_w, g.square_h), square, "{at}");
            }
        }
        // A 10x25 font makes squares wider: at 120x40 four rows would need 11 columns,
        // which leave the side column too narrow, so the board is limited by the width.
        let cases = [
            ((60, 20), (3, 1)),
            ((80, 24), (5, 2)),
            ((120, 40), (9, 3)),
            ((200, 60), (15, 6)),
            ((300, 100), (29, 11)),
        ];
        for ((width, height), square) in cases {
            let h = with_font((width, height), (10, 25), '1');
            let g = h.app.hit_map().board.expect("board drawn");
            assert_eq!((g.square_w, g.square_h), square, "{width}x{height}");
        }
    }

    #[test]
    fn the_playing_screen_uses_every_cell() {
        for (width, height) in SIZES {
            for font in FONTS.into_iter().chain([(10, 25), (5, 20)]) {
                for with_jev in [false, true] {
                    let at = format!("{width}x{height} font={font:?} jev={with_jev}");
                    let area = Rect::new(0, 0, width, height);
                    let (mode, jev_rows) = if with_jev {
                        (Mode::JevVsJev, Some(40))
                    } else {
                        (Mode::HumanVsHuman, None)
                    };
                    let cell = CellSize::new(font.0, font.1);
                    let l = playing_layout(area, cell, status_rows(mode, height), jev_rows);

                    // Left column: the board block over the command box, full height.
                    assert_eq!((l.board.x, l.board.y), (area.x, area.y), "{at}");
                    assert_eq!(
                        l.command,
                        Rect::new(l.board.x, l.board.bottom(), l.board.width, COMMAND_HEIGHT),
                        "{at}"
                    );
                    assert_eq!(l.command.bottom(), area.bottom(), "{at}");
                    // The board block is exactly as wide as its board: no spare columns.
                    let inner = Block::bordered()
                        .padding(Padding::horizontal(1))
                        .inner(l.board);
                    let board = layout_board(inner, false, cell).expect("board fits");
                    assert_eq!(board.outer.width, inner.width, "{at}");

                    // Right column: every column left, at least the minimum, full height,
                    // one shared border row between panels.
                    let column: Vec<Rect> =
                        [Some(l.status), l.jev, Some(l.moves), Some(l.captured)]
                            .into_iter()
                            .flatten()
                            .collect();
                    assert_eq!(column.len(), if with_jev { 4 } else { 3 }, "{at}");
                    for rect in &column {
                        assert_eq!(rect.x, l.board.right(), "{at}");
                        assert_eq!(rect.right(), area.right(), "{at}");
                    }
                    assert!(l.status.width >= SIDE_MIN_WIDTH, "{at}");
                    assert_eq!(column[0].y, area.y, "{at}");
                    for pair in column.windows(2) {
                        assert_eq!(pair[1].y, pair[0].bottom() - 1, "{at}");
                    }
                    assert_eq!(l.captured.bottom(), area.bottom(), "{at}");

                    // Status and Captured keep their heights; Moves takes what Jev leaves.
                    assert_eq!(l.status.height, status_rows(mode, height) + 2, "{at}");
                    assert_eq!(l.captured.height, CAPTURED_ROWS + 2, "{at}");
                    assert!(l.moves.height >= MOVES_MIN_ROWS + 2, "{at}");
                    if let Some(jev) = l.jev {
                        assert!(jev.height >= JEV_MIN_ROWS + 2, "{at}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_drawn_screen_reaches_every_edge() {
        for size in SIZES {
            for font in FONTS {
                for key in ['1', '2'] {
                    let h = with_font(size, font, key);
                    let at = format!("{size:?} font={font:?} key={key}");
                    let buffer = h.buffer();
                    let (right, bottom) = (size.0 - 1, size.1 - 1);
                    assert_eq!(buffer[(0, 0)].symbol(), "┌", "{at}");
                    assert_eq!(buffer[(right, 0)].symbol(), "┐", "{at}");
                    assert_eq!(buffer[(0, bottom)].symbol(), "└", "{at}");
                    assert_eq!(buffer[(right, bottom)].symbol(), "┘", "{at}");
                    for y in 1..bottom {
                        assert_ne!(buffer[(0, y)].symbol(), " ", "{at} row {y}");
                        assert_ne!(buffer[(right, y)].symbol(), " ", "{at} row {y}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_right_column_panels_stretch() {
        // Jev's text needs a few rows; Moves takes the rest of a tall terminal.
        let h = with_font((300, 100), (10, 20), '2');
        let area = h.buffer().area;
        let rows = status_rows(h.app.mode(), area.height);
        let jev = JevText::new(
            None,
            h.app.engine_status(),
            side_text_width(area, h.app.cell_size()),
        );
        let l = playing_layout(area, h.app.cell_size(), rows, Some(jev.rows()));
        let jev_rect = l.jev.expect("jev panel");
        assert_eq!(jev_rect.height, JEV_MIN_ROWS + 2);
        // Four panels share three border rows.
        assert_eq!(
            l.moves.height,
            area.height - (rows + 2) - (JEV_MIN_ROWS + 2) - (CAPTURED_ROWS + 2) + 3
        );
        assert_eq!(
            panel_rows(&h, "Moves").len(),
            usize::from(l.moves.height - 2)
        );
    }

    #[test]
    fn a_board_limited_by_width_is_centred_in_a_full_height_block() {
        // Tall and narrow: the side column's minimum width limits the board.
        for (size, font) in [
            ((70, 50), (10, 20)),
            ((120, 40), (10, 25)),
            ((60, 40), (8, 16)),
        ] {
            let h = with_font(size, font, '1');
            let at = format!("{size:?} font={font:?}");
            let g = h.app.hit_map().board.expect("board drawn");
            let block = playing_layout(
                h.buffer().area,
                h.app.cell_size(),
                status_rows(h.app.mode(), size.1),
                None,
            )
            .board;
            assert_eq!(block.height, size.1 - COMMAND_HEIGHT, "{at}");
            let inner = Rect::new(block.x + 2, block.y + 1, block.width - 4, block.height - 2);
            assert_eq!(g.outer.width, inner.width, "{at}");
            let (above, below) = (g.outer.y - inner.y, inner.bottom() - g.outer.bottom());
            assert!(above + below >= 8, "width-limited: {at}");
            assert!(above.abs_diff(below) <= 1, "centred: {at}");
        }
    }

    #[test]
    fn every_square_is_hit_at_every_size() {
        for size in SIZES {
            for font in FONTS.into_iter().chain([(10, 25)]) {
                // Human vs Human (White at the bottom), then flipped with `f`.
                let mut h = with_font(size, font, '1');
                for flipped in [false, true] {
                    if flipped {
                        h.char('f');
                    }
                    let at = format!("{size:?} font={font:?} flipped={flipped}");
                    let g = h.app.hit_map().board.expect("board drawn");
                    assert_eq!(g.flipped, flipped, "{at}");
                    let mut covered = 0;
                    for sq in Square::all() {
                        let rect = square_rect(&g, sq);
                        assert_eq!(rect.intersection(g.grid), rect, "{at}: {sq}");
                        for pos in rect.positions() {
                            assert_eq!(square_at(&g, pos.x, pos.y), Some(sq), "{at}: {pos:?}");
                            covered += 1;
                        }
                    }
                    assert_eq!(covered, g.grid.area(), "{at}");
                }
            }
        }
    }

    #[test]
    fn a_click_and_a_drag_move_pieces_on_the_largest_board_flipped() {
        let mut h = with_font((300, 100), (10, 20), '1');
        h.char('f');
        h.click_square(sq("e2"));
        h.click_square(sq("e4"));
        assert_eq!(h.uci(), ["e2e4"]);
        h.drag_square(sq("g8"), sq("f6"));
        assert_eq!(h.uci(), ["e2e4", "g8f6"]);
    }

    #[test]
    fn the_jev_panel_grows_with_its_text_and_moves_gives_way() {
        let area = Rect::new(0, 0, 80, 24);
        let rows = status_rows(Mode::HumanVsJev { human: Side::White }, 24);
        let short = playing_layout(area, CellSize::DEFAULT, rows, Some(1));
        let long = playing_layout(area, CellSize::DEFAULT, rows, Some(6));
        assert_eq!(short.jev.expect("jev").height, JEV_MIN_ROWS + 2);
        assert_eq!(long.jev.expect("jev").height, 6 + 2);
        assert_eq!(short.moves.height - long.moves.height, 6 - JEV_MIN_ROWS);
        let huge = playing_layout(area, CellSize::DEFAULT, rows, Some(100));
        assert_eq!(
            huge.moves.height,
            MOVES_MIN_ROWS + 2,
            "moves keeps its minimum"
        );
        assert_eq!(huge.captured.bottom(), area.bottom());
    }

    #[test]
    fn a_vetoed_reply_shows_every_detail_at_every_size() {
        let note = "Jev picked Qh4, which the search rates much worse; played Nf6";
        for (width, height) in [(60, 20), (80, 24), (120, 40)] {
            let h = vetoed_reply(width, height);
            let jev = panel_text(&h, "Jev");
            for part in [
                "played Nf6 · vetoed",
                "Qh4 62% Nf6 21% d5 9%",
                "conf 0.81",
                "12345 ms",
                "jev-latest",
                note,
            ] {
                assert!(jev.contains(part), "{width}x{height}: {part:?} in {jev:?}");
            }
            let moves = panel_text(&h, "Moves");
            assert!(moves.contains("1. e4 Nf6"), "{width}x{height}: {moves:?}");
        }
    }

    #[test]
    fn a_long_failure_note_is_cut_and_the_details_stay() {
        let error = format!("network error: {}", "connection refused ".repeat(12));
        let note = format!("Jev unavailable ({}) — local search", error.trim_end());
        for (width, height) in [(60, 20), (80, 24)] {
            let mut h = jev(width, height);
            h.char('2');
            h.moves(&["e4"]);
            h.reply_with("e7e5", |computer| {
                computer.source = MoveSource::Fallback;
                computer.top.clear();
                computer.confidence = None;
                computer.model = None;
                computer.latency = Duration::from_millis(15003);
                computer.note = Some(note.clone());
            });
            let jev = panel_text(&h, "Jev");
            assert!(jev.contains("played e5 · local search"), "{jev:?}");
            assert!(jev.contains("15003 ms"), "{jev:?}");
            assert!(jev.contains("Jev unavailable (network error:"), "{jev:?}");
            assert!(jev.ends_with('…'), "cut short: {jev:?}");
            assert!(panel_text(&h, "Moves").contains("1. e4 e5"));
        }
    }

    #[test]
    fn without_a_jev_key_the_panels_say_local_search_on_one_row_each() {
        for (width, height) in [(60, 20), (80, 24), (120, 40)] {
            let at = format!("{width}x{height}");
            let mut h = Harness::sized(FakeEngine::local(), width, height);
            h.char('3');
            let screen = h.screen();
            assert!(
                !screen.replace("JEV_API_KEY", "").contains("Jev"),
                "{at}:\n{screen}"
            );
            let status = panel_rows(&h, "Status");
            assert!(status[0].starts_with("White to move"), "{at}: {status:?}");
            assert!(status[1].starts_with("| Local search"), "{at}: {status:?}");
            assert!(status[1].ends_with("0.0s"), "not wrapped: {at}: {status:?}");
            let panel = panel_text(&h, "Local search");
            assert!(panel.contains(LOCAL_SEARCH_STATUS), "{at}: {panel:?}");
        }
        // Wide enough, nothing is left out.
        let mut h = Harness::sized(FakeEngine::local(), 120, 40);
        h.char('3');
        assert_eq!(
            panel_rows(&h, "Status")[..2],
            [
                "White to move (Local search)",
                "| Local search thinking... 0.0s"
            ]
        );
    }

    /// The Status panel's top border, where the mode title goes.
    fn status_title(h: &Harness) -> String {
        status_row(h, 0)
    }

    /// Row `row` of the Status panel, counted from its top border.
    fn status_row(h: &Harness, row: u16) -> String {
        let buffer = h.buffer();
        let area = buffer.area;
        let status = playing_layout(
            area,
            h.app.cell_size(),
            status_rows(h.app.mode(), area.height),
            None,
        )
        .status;
        (status.x..status.right())
            .map(|x| buffer[(x, status.y + row)].symbol())
            .collect()
    }

    #[test]
    fn the_mode_title_keeps_its_room_in_debug_mode() {
        for (width, height) in [(60, 20), (80, 24), (120, 40)] {
            for jev in [false, true] {
                for key in ['1', '2', '3', '5'] {
                    let engine = || {
                        if jev {
                            FakeEngine::jev()
                        } else {
                            FakeEngine::local()
                        }
                    };
                    let mut plain = Harness::sized(engine(), width, height);
                    plain.char(key);
                    let mut h = Harness::build(engine(), (width, height), Vec::new(), |app| {
                        app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
                    });
                    h.char(key);
                    let case = format!("{width}x{height} jev={jev} {key}");
                    // Without debug mode nothing says DEBUG.
                    for row in 0..2 {
                        let plain_row = status_row(&plain, row);
                        assert!(!plain_row.contains("DEBUG"), "{case}: {plain_row}");
                    }
                    let top = status_title(&h);
                    let first = status_row(&h, 1);
                    // The same mode title as without debug mode, which always has one.
                    let title = h
                        .app
                        .mode_labels()
                        .into_iter()
                        .find(|label| status_title(&plain).contains(&format!(" {label} ┐")))
                        .unwrap_or_else(|| panic!("{case}: no mode title"));
                    assert!(top.contains(&format!(" {title} ┐")), "{case}: {top}");
                    // DEBUG beside it when both fit, else at the start of the first line.
                    let on_top = top.contains("┌ Status ─ DEBUG ─");
                    let in_text = first.starts_with("│ DEBUG White to move");
                    assert!(on_top != in_text, "{case}: {top} / {first}");
                    if width >= 120 && jev {
                        assert!(on_top, "{case}: {top}");
                    }
                }
            }
        }
        // At the smallest size no mode leaves room for both.
        let mut h = Harness::build(FakeEngine::jev(), (60, 20), Vec::new(), |app| {
            app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
        });
        h.char('5');
        assert!(status_title(&h).contains(" Jev vs Jev ┐"));
        insta::assert_snapshot!("jev_vs_jev_debug_60x20", h.terminal.backend());
    }

    #[test]
    fn the_mode_title_shortens_until_it_fits() {
        let engine = |jev: bool| {
            if jev {
                FakeEngine::jev()
            } else {
                FakeEngine::local()
            }
        };
        // (size, with a Jev key, menu key, the title shown)
        let cases = [
            ((60, 20), false, '2', "You (W) vs Local"),
            ((60, 20), false, '5', "Local vs Local"),
            ((60, 20), true, '3', "You (B) vs Jev"),
            ((60, 20), true, '5', "Jev vs Jev"),
            ((80, 24), false, '3', "You (B) vs Local"),
            ((80, 24), false, '5', "Local vs Local"),
            ((80, 24), true, '2', "You (White) vs Jev"),
            ((120, 40), false, '3', "You (Black) vs Local search"),
            ((120, 40), false, '5', "Local search vs Local search"),
        ];
        for ((width, height), jev, key, title) in cases {
            let mut h = Harness::sized(engine(jev), width, height);
            h.char(key);
            let shown = status_title(&h);
            assert!(
                shown.contains(&format!(" {title} ┐")),
                "{width}x{height} {key}: {shown}"
            );
        }
        // Some title always fits: the last one is never wider than the narrowest panel.
        for (width, height) in [(60, 20), (80, 24), (100, 30), (120, 40)] {
            for jev in [false, true] {
                for key in ['1', '2', '3', '5'] {
                    let mut h = Harness::sized(engine(jev), width, height);
                    h.char(key);
                    let shown = status_title(&h);
                    assert!(
                        h.app
                            .mode_labels()
                            .iter()
                            .any(|label| shown.contains(&format!(" {label} ┐"))),
                        "{width}x{height} jev={jev} {key}: {shown}"
                    );
                }
            }
        }
    }

    #[test]
    fn menu_warnings_that_do_not_fit_are_counted() {
        let warnings: Vec<String> = (1..=6).map(|n| format!("warning {n}")).collect();
        let screen = |height: u16| {
            let h = Harness::build(FakeEngine::local(), (60, height), warnings.clone(), |app| {
                app
            });
            h.screen()
        };
        // All six fit at 60×24.
        let tall = screen(24);
        assert!(tall.contains("! warning 6"), "{tall}");
        assert!(!tall.contains("more warning"), "{tall}");
        // At 60×20 the notes get four rows: the status, two warnings and the count.
        let short = screen(20);
        assert!(short.contains("! warning 2"), "{short}");
        assert!(!short.contains("! warning 3"), "{short}");
        assert!(short.contains("+4 more warnings"), "{short}");
        // One row more shows one more warning.
        let taller = screen(21);
        assert!(taller.contains("! warning 3"), "{taller}");
        assert!(taller.contains("+3 more warnings"), "{taller}");
    }

    #[test]
    fn a_warning_too_long_for_the_rows_left_is_counted_too() {
        // At 60x20 the notes get four rows: the status, a short warning, and no room for
        // a warning three rows long, so the count takes its place.
        let warnings = vec!["short".to_string(), "long ".repeat(25)];
        let h = Harness::build(FakeEngine::local(), (60, 20), warnings, |app| app);
        let screen = h.screen();
        assert!(screen.contains("! short"), "{screen}");
        assert!(!screen.contains("! long"), "{screen}");
        assert!(screen.contains("+1 more warning "), "{screen}");
    }

    /// The characters [`Wrapper`] is fed on this thread while `fit` runs: the work of the
    /// fitting functions, which wrap their text and every cut they try through it.
    fn wrapped_chars<T>(fit: impl FnOnce() -> T) -> (T, usize) {
        let before = WRAPPED_CHARS.with(Cell::get);
        let result = fit();
        (result, WRAPPED_CHARS.with(Cell::get) - before)
    }

    #[test]
    fn long_messages_are_fitted_in_linear_time() {
        // A pasted path of 20 000 characters (the error echoes it) used to take seconds a
        // frame: every cut was wrapped again from the start. Each fit now wraps the message
        // a few times at most, however long it is (plus a few forms that fit the panel):
        // once to see whether it fits, once to find a cut, and once more to cut between
        // characters when there is no word to cut at.
        for n in [2_000, 20_000] {
            let name = "x".repeat(n);
            let words = "word ".repeat(n / 5);
            let folders = "d/".repeat(n / 4);
            let panel = 30 * 2;
            // At most `times` passes over `len` characters, plus the forms that fit.
            let linear = |len: usize, times: usize| times * len + panel * panel;

            let (cut, work) = wrapped_chars(|| fit_rows(&name, 30, 2));
            assert_eq!(cut.chars().count(), 60);
            assert!(work <= linear(n, 3), "{n}: {work}");

            let (cut, work) = wrapped_chars(|| fit_rows(&words, 30, 2));
            assert!(cut.ends_with("word…"));
            assert!(work <= linear(words.len(), 2), "{n}: {work}");

            let path = format!("/{folders}game.pgn");
            let (deep, work) = wrapped_chars(|| fit_message("saved ", Some(&path), 30, 2));
            assert_eq!(deep, format!("saved /d/…/{}game.pgn", "d/".repeat(8)));
            assert!(work <= linear(path.len(), 2), "{n}: {work}");

            let path = format!("~/{name}");
            let (long, work) = wrapped_chars(|| fit_message("saved ", Some(&path), 30, 2));
            assert!(
                long.starts_with("saved …/xxx") && long.ends_with('…'),
                "{long}"
            );
            assert!(work <= linear(path.len(), 3), "{n}: {work}");
        }
    }

    #[test]
    fn fit_rows_cuts_at_a_word_and_marks_the_cut() {
        assert_eq!(fit_rows("short note", 20, 1), "short note");
        assert_eq!(fit_rows("one two three four", 9, 1), "one two…");
        assert_eq!(fit_rows("one two three four", 9, 2), "one two three…");
        assert_eq!(fit_rows("abcdefghij", 4, 2), "abcdefg…");
        for (text, width, rows) in [("a b c d e f g h", 3, 2), ("x".repeat(50).as_str(), 7, 3)] {
            assert!(wrapped_height(&fit_rows(text, width, rows), width) <= rows);
        }
    }

    #[test]
    fn a_message_that_does_not_fit_its_rows_ends_in_an_ellipsis() {
        assert_eq!(fit_message("glyphs: ascii", None, 28, 1), "glyphs: ascii");
        assert_eq!(
            fit_message("nothing to resign while watching", None, 28, 1),
            "nothing to resign while…"
        );
        // Watching at 60x20 leaves the message one row under the turn, pace and thinking.
        let mut h = Harness::sized(FakeEngine::local(), 60, 20);
        h.char('5');
        h.command(":resign");
        let status = panel_rows(&h, "Status");
        assert_eq!(status.len(), 4, "{status:?}");
        assert_eq!(status[3], "nothing to resign while…");
    }

    #[test]
    fn paths_in_messages_lose_folders_before_the_file_name() {
        let path = "~/Projects/Rust/rchess/chess/game.pgn";
        assert_eq!(
            fit_message("saved ", Some(path), 60, 1),
            format!("saved {path}")
        );
        // The middle of the folder part goes first, a folder at a time.
        assert_eq!(
            fit_message("saved ", Some(path), 28, 1),
            "saved ~/…/chess/game.pgn"
        );
        assert_eq!(
            fit_message("saved ", Some(path), 20, 1),
            "saved ~/…/game.pgn"
        );
        assert_eq!(
            fit_message("saved ", Some("/tmp/rchess-save/deep/game.pgn"), 22, 1),
            "saved /tmp/…/game.pgn"
        );
        // The reason stays in front of the path.
        assert_eq!(
            fit_message(
                "cannot save (folder does not exist): ",
                Some("~/one/two/three/four/game.pgn"),
                28,
                2
            ),
            "cannot save (folder does not exist): ~/…/four/game.pgn"
        );
        // A long file name is kept whole while the folders can go instead.
        let long = "~/games/a-very-long-file-name-for-a-game.pgn";
        assert_eq!(
            fit_message("saved ", Some(long), 40, 2),
            "saved ~/…/a-very-long-file-name-for-a-game.pgn"
        );
        // Only a name that cannot fit at all is cut, and the cut is marked.
        let cut = fit_message("saved ", Some(long), 20, 1);
        assert_eq!(cut, "saved …/a-very-long…");
        assert!(cut.ends_with('…'), "{cut}");
        assert!(wrapped_height(&cut, 20) <= 1, "{cut}");
    }

    #[test]
    fn a_save_message_names_the_file_at_the_minimum_size() {
        let dir = crate::tui::test_support::TempDir::new("a-home-folder-with-a-long-name");
        let home = dir.path().to_path_buf();
        let mut h = Harness::build(FakeEngine::local(), (60, 20), Vec::new(), move |app| {
            app.with_home(Some(home))
        });
        h.char('1');
        h.command(":savepgn ~/game");
        assert_eq!(h.app.status_line(), "saved ~/game.pgn");
        assert!(panel_text(&h, "Status").ends_with("saved ~/game.pgn"));

        let deep = dir
            .path()
            .join("a")
            .join("rather")
            .join("deep")
            .join("folder");
        std::fs::create_dir_all(&deep).expect("folders");
        h.command(":savepgn ~/a/rather/deep/folder/game");
        let status = panel_rows(&h, "Status");
        assert!(
            status.iter().any(|row| row.ends_with("/folder/game.pgn")),
            "{status:?}"
        );
    }

    #[test]
    fn a_save_message_under_a_wrapped_outcome_names_the_file() {
        // "Black resigned — White wins (1-0)" is wider than the Status panel at 60x20,
        // so it takes two rows and the save message gets what is left.
        let dir = crate::tui::test_support::TempDir::new("outcome-save");
        let folder = dir.path().join("subfolder12");
        std::fs::create_dir_all(&folder).expect("folder");
        let mut h = Harness::sized(FakeEngine::local(), 60, 20);
        h.char('1');
        h.command("e4");
        h.command(":resign");
        h.char('y');
        assert_eq!(h.app.screen_name(), "game over");
        h.ctrl('s');
        h.type_text(folder.join("game").to_str().expect("utf-8 temp path"));
        h.press(KeyCode::Enter);
        assert!(folder.join("game.pgn").exists());
        let status = panel_rows(&h, "Status");
        assert!(status.iter().any(|row| row.contains("(1-0)")), "{status:?}");
        let last = status.last().expect("a message row");
        assert!(last.starts_with("saved "), "{status:?}");
        assert!(last.ends_with("/game.pgn"), "{status:?}");
    }

    #[test]
    fn pack_breaks_between_items_only() {
        let items = |texts: &[&'static str]| -> Vec<Vec<Span<'static>>> {
            texts.iter().map(|&t| vec![Span::raw(t)]).collect()
        };
        let lines = pack(items(&["conf 0.81", "12345 ms", "jev-latest"]), " · ", 28);
        assert_eq!(texts(&lines), ["conf 0.81 · 12345 ms", "jev-latest"]);
        let lines = pack(items(&["conf 0.81", "12345 ms", "jev-latest"]), " · ", 40);
        assert_eq!(texts(&lines), ["conf 0.81 · 12345 ms · jev-latest"]);
        let lines = pack(items(&["a-very-long-item", "b"]), " · ", 5);
        assert_eq!(texts(&lines), ["a-very-long-item", "b"]);
        assert!(pack(Vec::new(), " · ", 10).is_empty());
    }

    // ----- hit map and cursor -----

    #[test]
    fn every_screen_records_its_click_targets() {
        let mut h = jev(80, 24);
        let hits = h.app.hit_map().clone();
        assert_eq!(hits.board, None);
        for index in 0..MENU_ITEMS.len() {
            let rect = hits.rect_of(Hit::MenuItem(index)).expect("menu row");
            assert_eq!(rect.height, 1);
            assert_eq!(hits.at(rect.x, rect.y), Some(Hit::MenuItem(index)));
        }

        h.char('1');
        let playing = h.app.hit_map().clone();
        let board = playing.board.expect("board geometry");
        for hit in [Hit::CommandBox, Hit::MoveList] {
            let rect = playing.rect_of(hit).expect("panel");
            assert!(rect.intersection(board.outer).is_empty(), "{hit:?}");
        }

        h.char('?');
        assert_eq!(h.app.hit_map(), &playing, "help records nothing");
        h.press(KeyCode::Esc);

        // A move first, so that q asks.
        h.moves(&["f3"]);
        h.char('q');
        for button in [Button::Confirm, Button::Cancel] {
            let rect = h
                .app
                .hit_map()
                .rect_of(Hit::Button(button))
                .expect("button");
            assert_eq!(
                h.app.hit_map().at(rect.x, rect.y),
                Some(Hit::Button(button))
            );
        }
        h.press(KeyCode::Esc);

        h.ctrl('s');
        assert_eq!(h.app.dialog_name(), Some("save pgn"));
        for button in [Button::Confirm, Button::Cancel] {
            let rect = h
                .app
                .hit_map()
                .rect_of(Hit::Button(button))
                .expect("button");
            assert_eq!(
                h.app.hit_map().at(rect.x, rect.y),
                Some(Hit::Button(button))
            );
        }
        h.press(KeyCode::Esc);

        h.moves(&["e5", "g4", "Qh4#"]);
        for button in GAME_OVER_BUTTONS {
            let rect = h
                .app
                .hit_map()
                .rect_of(Hit::Button(button))
                .expect("button");
            assert_eq!(
                h.app.hit_map().at(rect.x, rect.y),
                Some(Hit::Button(button))
            );
        }

        let mut h = jev(80, 24);
        h.char('1');
        h.moves(&[&format!(":fen {PROMOTION_FEN}")]);
        h.promote_e7_by_keyboard();
        for kind in PROMOTION_CHOICES {
            let rect = h.app.hit_map().rect_of(Hit::Promote(kind)).expect("choice");
            assert_eq!(h.app.hit_map().at(rect.x, rect.y), Some(Hit::Promote(kind)));
        }

        let mut h = jev(59, 30);
        h.char('1');
        assert_eq!(h.app.hit_map(), &HitMap::default(), "too small");
    }

    #[test]
    fn the_terminal_cursor_shows_only_while_typing() {
        let mut h = jev(80, 24);
        h.char('1');
        // A move first, so that Ctrl+C asks below.
        h.moves(&["e4"]);
        assert!(!h.terminal.backend().cursor_visible());
        h.char('/');
        h.type_text("e4");
        let command = h
            .app
            .hit_map()
            .rect_of(Hit::CommandBox)
            .expect("command box");
        // Border and padding, then the prompt, then two characters.
        let expected = (command.x + 2 + PROMPT_WIDTH + 2, command.y + 1);
        assert!(h.terminal.backend().cursor_visible());
        assert_eq!(h.terminal.backend().cursor_position(), expected.into());
        h.ctrl('c');
        assert_eq!(h.app.dialog_name(), Some("quit"));
        assert!(
            !h.terminal.backend().cursor_visible(),
            "covered by a dialog"
        );
        h.char('n');
        assert!(h.terminal.backend().cursor_visible());
        h.press(KeyCode::Esc);
        assert!(!h.terminal.backend().cursor_visible());

        h.ctrl('s');
        h.type_text("game");
        assert!(h.terminal.backend().cursor_visible(), "in the save dialog");
        let cancel = h
            .app
            .hit_map()
            .rect_of(Hit::Button(Button::Cancel))
            .expect("button");
        let cursor = h.terminal.backend().cursor_position();
        assert!(cursor.y < cancel.y);
        assert_eq!(h.buffer()[(cursor.x - 1, cursor.y)].symbol(), "e");
    }

    // ----- panel contents -----

    #[test]
    fn captured_pieces_count_en_passant_and_promotions() {
        let white = |kind| Piece::new(Side::White, kind);
        let black = |kind| Piece::new(Side::Black, kind);
        let game = game_from("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", &["e5d6"]);
        assert_eq!(
            captured_pieces(&game),
            [vec![black(PieceKind::Pawn)], vec![]]
        );

        let game = game_from("r3k3/1P6/8/8/8/8/8/4K3 w - - 0 1", &["b7a8q", "e8d7"]);
        assert_eq!(
            captured_pieces(&game),
            [vec![black(PieceKind::Rook)], vec![]]
        );

        let game = game_from(
            START_FEN,
            &["e2e4", "d7d5", "e4d5", "d8d5", "b1c3", "d5a2", "a1a2"],
        );
        let [by_white, by_black] = captured_pieces(&game);
        assert_eq!(by_white, [black(PieceKind::Queen), black(PieceKind::Pawn)]);
        assert_eq!(by_black, [white(PieceKind::Pawn), white(PieceKind::Pawn)]);
    }

    #[test]
    fn material_balance_is_white_minus_black() {
        assert_eq!(material_balance(&ChessPosition::startpos()), 0);
        let pos = ChessPosition::from_fen("4k3/8/8/8/8/8/8/QR2K1N1 w - - 0 1").expect("fen");
        assert_eq!(material_balance(&pos), 9 + 5 + 3);
        let pos = ChessPosition::from_fen("3qk3/pp6/8/8/8/8/8/4K2B w - - 0 1").expect("fen");
        assert_eq!(material_balance(&pos), 3 - 9 - 2);
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    /// The Jev panel's lines at `width` with room for everything.
    fn jev_lines(computer: Option<&ComputerMove>, status: &str, width: u16) -> Vec<Line<'static>> {
        let text = JevText::new(computer, status, width);
        let rows = text.rows();
        text.into_lines(rows)
    }

    #[test]
    fn jev_lines_describe_the_last_computer_move() {
        assert_eq!(
            texts(&jev_lines(None, LOCAL_SEARCH_STATUS, 40)),
            [LOCAL_SEARCH_STATUS, "no move yet"]
        );
        let game = Game::new();
        let mv = game.position().parse_uci("g1f3").expect("legal");
        let mut computer = ComputerMove {
            mv,
            san: "Nf3".to_string(),
            source: MoveSource::Jev,
            top: vec![
                ("Nf3".to_string(), 0.615),
                ("e4".to_string(), 0.2),
                ("d4".to_string(), 0.1),
                ("c4".to_string(), 0.05),
            ],
            confidence: Some(0.8),
            model: Some("jev-latest".to_string()),
            latency: Duration::from_millis(870),
            input_tokens: Some(900),
            note: None,
            exchange: None,
        };
        assert_eq!(
            texts(&jev_lines(Some(&computer), JEV_STATUS, 40)),
            [
                "played Nf3 · Jev",
                "Nf3 62%  e4 20%  d4 10%",
                "conf 0.80 · 870 ms · jev-latest",
            ]
        );
        // Narrow: lines break between items, never inside one.
        assert_eq!(
            texts(&jev_lines(Some(&computer), JEV_STATUS, 20)),
            [
                "played Nf3 · Jev",
                "Nf3 62%  e4 20%",
                "d4 10%",
                "conf 0.80 · 870 ms",
                "jev-latest",
            ]
        );
        computer.source = MoveSource::Fallback;
        computer.top.clear();
        computer.confidence = None;
        computer.model = None;
        computer.note = Some("Jev timed out".to_string());
        assert_eq!(
            texts(&jev_lines(Some(&computer), JEV_STATUS, 40)),
            ["played Nf3 · local search", "870 ms", "Jev timed out"]
        );
        // Too few rows: the note is cut, the rest stays.
        let text = JevText::new(Some(&computer), JEV_STATUS, 40);
        assert_eq!(text.rows(), 3);
        assert_eq!(
            texts(&text.into_lines(2)),
            ["played Nf3 · local search", "870 ms"]
        );
    }

    #[test]
    fn visible_input_keeps_the_cursor_on_screen() {
        let mut editor = LineEditor::new();
        editor.insert_str("abcdef");
        assert_eq!(visible_input(&editor, 4), ("def".to_string(), 3));
        editor.home();
        assert_eq!(visible_input(&editor, 4), ("abcd".to_string(), 0));
        editor.right();
        editor.right();
        assert_eq!(visible_input(&editor, 10), ("abcdef".to_string(), 2));
        let mut wide = LineEditor::new();
        wide.insert_str("日本語");
        assert_eq!(visible_input(&wide, 5), ("本語".to_string(), 4));
        assert_eq!(visible_input(&LineEditor::new(), 0), (String::new(), 0));
    }

    #[test]
    fn the_move_list_clamps_its_scroll_and_aligns_numbers() {
        let rows: Vec<String> = (1..=12).map(|n| format!("{n}. a3 a6")).collect();
        let mut terminal = Terminal::new(TestBackend::new(20, 7)).expect("test terminal");
        let mut scroll = 0;
        terminal
            .draw(|frame| scroll = moves_panel(frame, frame.area(), &rows, 100))
            .expect("draw");
        assert_eq!(scroll, 12 - 5, "the first row at the top, no further");
        let screen = terminal.backend().to_string();
        assert!(screen.contains("+7 below"), "{screen}");
        assert!(
            screen.contains("  1. a3   a6"),
            "numbers right-aligned: {screen}"
        );
        assert!(!screen.contains("6. a3"), "{screen}");

        terminal
            .draw(|frame| scroll = moves_panel(frame, frame.area(), &rows, 0))
            .expect("draw");
        assert_eq!(scroll, 0);
        let screen = terminal.backend().to_string();
        assert!(
            screen.contains("12. a3   a6") && screen.contains(" 8. a3   a6"),
            "{screen}"
        );
        assert!(!screen.contains("below"), "{screen}");
    }

    #[test]
    fn the_move_list_lines_up_blacks_moves() {
        let rows = |list: &[&str]| -> Vec<String> {
            let rows: Vec<String> = list.iter().map(|row| (*row).to_string()).collect();
            let mut terminal = Terminal::new(TestBackend::new(24, 7)).expect("test terminal");
            terminal
                .draw(|frame| {
                    moves_panel(frame, frame.area(), &rows, 0);
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            (1..buffer.area.height - 1)
                .map(|y| {
                    (2..buffer.area.width - 1)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .filter(|row| !row.is_empty())
                .collect()
        };
        // As in the spec's layout: White's moves padded, Black's in one column.
        assert_eq!(
            rows(&["1. e4 e5", "2. Nf3 Nc6", "3. Bb5"]),
            ["1. e4   e5", "2. Nf3  Nc6", "3. Bb5"]
        );
        // A longer move widens the column for every row; numbers stay right-aligned.
        assert_eq!(
            rows(&["9. e4 e5", "10. exd8=Q+ Kxd8"]),
            [" 9. e4       e5", "10. exd8=Q+  Kxd8"]
        );
        // A game from a FEN with Black to move starts in Black's column.
        assert_eq!(
            rows(&["12... Kd7", "13. e4 Ke6"]),
            ["12. ...  Kd7", "13. e4   Ke6"]
        );
    }

    #[test]
    fn wrapped_height_agrees_with_the_paragraph_wrapper() {
        let texts = [
            "",
            "short",
            TOO_SMALL,
            "Watch Jev play itself (space pauses, +/- pace).",
            "/a/very/long/path/that/does/not/fit/on/one/row/at/all/game.pgn exists",
            "--glyphs: unknown glyph set \"fancy\" (expected solid, outline or ascii); using solid",
            "ab verylongword x",
            "No JEV_API_KEY — local search",
        ];
        // Narrow characters only: ratatui 0.30's wrapper lets two-cell characters overflow
        // a row, so it is no reference for them.
        for text in texts {
            for width in [1u16, 5, 10, 17, 30, 52] {
                let mut buffer = Buffer::empty(Rect::new(0, 0, width, 100));
                Paragraph::new(text)
                    .wrap(Wrap { trim: true })
                    .render(buffer.area, &mut buffer);
                let used = (0..100u16)
                    .rev()
                    .find(|&y| (0..width).any(|x| buffer[(x, y)].symbol() != " "))
                    .map_or(1, |y| y + 1);
                assert_eq!(wrapped_height(text, width), used, "{text:?} at {width}");
            }
        }
    }

    #[test]
    fn help_lines_fit_a_sixty_column_terminal() {
        for line in HELP_LINES {
            assert!(Span::raw(line).width() <= 56, "{line:?}");
            assert!(line.is_char_boundary(HELP_KEY_WIDTH.min(line.len())));
        }
    }

    // ----- snapshots -----

    #[test]
    fn snapshot_menu() {
        let engine =
            FakeEngine::local().with_warnings(&["JEV_TIMEOUT_MS is not a number; using 20000"]);
        let (_, glyph_warnings) = initial_glyphs(Some("fancy"), |_| None, ImageSupport::Off);
        let h = Harness::build(engine, (80, 24), glyph_warnings, |app| app);
        insta::assert_snapshot!("menu_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_menu_with_more_warnings_than_fit() {
        let warnings = (1..=6)
            .map(|n| format!("warning {n}: something at start-up was not as expected"))
            .collect();
        let h = Harness::build(FakeEngine::local(), (60, 20), warnings, |app| app);
        insta::assert_snapshot!("menu_warnings_60x20", h.terminal.backend());
    }

    #[test]
    fn snapshot_playing_start() {
        let mut h = jev(80, 24);
        h.char('1');
        insta::assert_snapshot!("playing_start_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_playing_start_large_as_black() {
        // Flipped, and Jev is thinking about its first move.
        let mut h = jev(120, 40);
        h.char('3');
        assert!(h.app.is_thinking());
        insta::assert_snapshot!("playing_start_120x40_vs_jev", h.terminal.backend());
    }

    #[test]
    fn snapshot_mid_game_highlights() {
        let h = mid_game(80, 24);
        insta::assert_snapshot!("mid_game_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_board_colours() {
        let h = mid_game(60, 20);
        let board = h.app.hit_map().board.expect("board drawn").outer;
        insta::assert_debug_snapshot!("mid_game_board_colours", crop(h.buffer(), board));
    }

    #[test]
    fn snapshot_jev_panel_after_a_reply() {
        let mut h = jev(80, 24);
        h.char('2');
        h.moves(&["e4"]);
        h.reply("c7c5");
        insta::assert_snapshot!("jev_reply_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_jev_vetoed_reply() {
        let h = vetoed_reply(80, 24);
        insta::assert_snapshot!("jev_vetoed_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_jev_vs_jev_at_the_minimum_size() {
        let mut h = jev(60, 20);
        h.char('5');
        h.reply("e2e4");
        h.char(' ');
        insta::assert_snapshot!("jev_vs_jev_60x20", h.terminal.backend());
    }

    #[test]
    fn jev_vs_jev_status_shows_pace_and_thinking_once_each() {
        let mut h = jev(60, 20);
        h.char('5');
        h.reply("e2e4");
        h.char(' ');
        let status = panel_text(&h, "Status");
        assert_eq!(status, "Black to move (Jev) paused · space resumes");
        h.char(' ');
        h.char('+');
        // Thinking again: the pace line stays above the spinner and nothing repeats.
        h.now += Duration::from_secs(2);
        h.send(AppEvent::Tick);
        let status = panel_text(&h, "Status");
        assert_eq!(
            status,
            "Black to move (Jev) step 1.5 s · space pauses | Jev thinking... 0.0s"
        );
    }

    #[test]
    fn snapshot_promotion_dialog() {
        let mut h = jev(80, 24);
        h.char('1');
        h.moves(&[&format!(":fen {PROMOTION_FEN}")]);
        h.promote_e7_by_keyboard();
        assert_eq!(h.app.dialog_name(), Some("promotion"));
        insta::assert_snapshot!("promotion_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_game_over() {
        let mut h = jev(80, 24);
        h.char('1');
        h.moves(&["f3", "e5", "g4", "Qh4#"]);
        assert_eq!(h.app.screen(), Screen::GameOver);
        insta::assert_snapshot!("game_over_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_help() {
        let mut h = jev(80, 24);
        h.char('1');
        h.char('?');
        insta::assert_snapshot!("help_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_quit_confirmation() {
        let mut h = jev(80, 24);
        h.char('1');
        // q asks only once a move has been played.
        h.moves(&["e4"]);
        h.char('q');
        insta::assert_snapshot!("quit_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_load_fen_error_at_the_minimum_size() {
        let mut h = jev(60, 20);
        h.char('6');
        h.type_text("hello");
        h.press(KeyCode::Enter);
        insta::assert_snapshot!("load_fen_error_60x20", h.terminal.backend());
    }

    #[test]
    fn snapshot_local_search_at_the_minimum_size() {
        let mut h = Harness::sized(FakeEngine::local(), 60, 20);
        h.char('3');
        insta::assert_snapshot!("local_search_thinking_60x20", h.terminal.backend());
    }

    #[test]
    fn snapshot_local_search_watching() {
        // "Local search vs Local search" is too wide for the title here: "Local vs Local".
        let mut h = Harness::sized(FakeEngine::local(), 80, 24);
        h.char('5');
        insta::assert_snapshot!("local_vs_local_80x24", h.terminal.backend());
    }

    /// Human vs Jev (the person plays White) at `width`×`height` in debug mode: Jev's
    /// answer to 1. e4 came after an undo (stale), then its answer to 1. e4 played again.
    fn two_exchanges(width: u16, height: u16) -> Harness {
        let mut h = Harness::build(FakeEngine::jev(), (width, height), Vec::new(), |app| {
            app.with_debug(DebugLog::open(Err(NO_LOG_PATH.to_string())))
        });
        h.char('2');
        h.moves(&["e4"]);
        let first = h.request.take().expect("Jev asked");
        h.char('u');
        h.answer(
            &first,
            EngineOutcome::Move(traced_jev_move(first.game.position(), "e7e5")),
        );
        h.moves(&["e4"]);
        h.reply_traced("c7c5");
        assert_eq!(h.app.exchanges().map(|history| history.len()), Some(2));
        h
    }

    #[test]
    fn snapshot_exchange_view() {
        let mut h = two_exchanges(80, 24);
        h.char('d');
        insta::assert_snapshot!("exchange_80x24", h.terminal.backend());
    }

    #[test]
    fn snapshot_exchange_view_large_on_a_stale_reply() {
        let mut h = two_exchanges(120, 40);
        h.char('d');
        h.press(KeyCode::Left);
        h.press(KeyCode::PageDown);
        insta::assert_snapshot!("exchange_120x40_stale", h.terminal.backend());
    }

    #[test]
    fn the_exchange_header_breaks_between_items() {
        let mut h = two_exchanges(60, 20);
        h.char('d');
        h.press(KeyCode::Left);
        let rows: Vec<String> = h.screen().lines().map(str::to_string).collect();
        assert_eq!(
            rows[1].trim_matches(|c: char| c == '│' || c.is_whitespace()),
            "exchange 1 of 2 · move 1 · e5 · Jev · HTTP 200"
        );
        assert_eq!(
            rows[2].trim_matches(|c: char| c == '│' || c.is_whitespace()),
            "2 attempts · 1234 ms · stale — not played"
        );
        assert_eq!(rows[3].trim_matches('│').trim(), "", "a blank row");
        assert!(rows[4].contains("│ REQUEST"), "{}", rows[4]);
        assert!(rows[0].contains(" 1-15 of "), "{}", rows[0]);
        let bottom = &rows[19];
        assert!(bottom.contains("Esc"), "{bottom}");
    }

    #[test]
    fn the_exchange_view_draws_only_the_rows_it_shows() {
        let h = two_exchanges(80, 24);
        let record = h
            .app
            .exchanges()
            .and_then(|history| history.last())
            .unwrap();
        let mut cache = BodyCache::default();
        let body = cache.rows(record, 20);
        let all = body_rows(&body, 0, usize::MAX);
        let total: usize = record
            .exchange
            .body()
            .iter()
            .map(|line| line.wrapped(20).len())
            .sum();
        assert_eq!(all.len(), total);
        assert_eq!(body.len(), total);
        assert!(all.iter().all(|row| row.width() <= 20));
        for (scroll, page) in [(0, 5), (7, 10), (total - 3, 10), (total, 4)] {
            let rows = body_rows(&body, scroll, page);
            let expected: Vec<Line> = all.iter().skip(scroll).take(page).cloned().collect();
            assert_eq!(rows, expected, "scroll {scroll}, page {page}");
        }
    }

    #[test]
    fn snapshot_too_small() {
        let h = jev(50, 12);
        insta::assert_snapshot!("too_small_50x12", h.terminal.backend());
    }

    // ----- pictures under the boxes drawn over the board -----

    /// An app against Jev in a 120×40 terminal, started from the menu with `key` and
    /// switched to the Image style, its pictures drawn by a `protocol` picker.
    fn pictures(protocol: ProtocolType, key: char) -> Harness {
        let picker = picker_for(protocol, CellSize::DEFAULT);
        let mut h = Harness::build(FakeEngine::jev(), (120, 40), Vec::new(), |app| {
            app.with_picker(Some(picker))
        });
        h.char(key);
        for _ in 0..3 {
            h.char('g');
        }
        assert_eq!(h.app.glyphs(), GlyphSet::Image);
        h
    }

    /// Adds to `ids` the kitty pictures whose image data `buffer` sends to the terminal
    /// (the `i=` of each transmit).
    fn transmitted(buffer: &Buffer, ids: &mut HashSet<String>) {
        for cell in buffer.content() {
            if let Some((_, rest)) = cell.symbol().split_once("_Gq=2,i=") {
                let id = rest.split(',').next().unwrap_or_default();
                ids.insert(id.to_string());
            }
        }
    }

    /// Draws the app again and returns the whole frame, Skip cells included (the
    /// backend keeps only what the diff sent it).
    fn frame(h: &mut Harness) -> Buffer {
        let now = h.now;
        h.terminal
            .draw(|frame| h.app.render(frame, now))
            .expect("draw")
            .buffer
            .clone()
    }

    /// The image area of `square` as last drawn.
    fn picture_area(h: &Harness, square: Square) -> Rect {
        let geometry = h.app.hit_map().board.expect("the board is drawn");
        image_area(square_rect(&geometry, square)).expect("room for a picture")
    }

    #[test]
    fn a_kitty_picture_first_drawn_under_the_help_still_reaches_the_terminal() {
        let mut h = pictures(ProtocolType::Kitty, '2');
        let mut ids = HashSet::new();
        transmitted(h.buffer(), &mut ids);
        h.command("e4");
        transmitted(h.buffer(), &mut ids);
        h.press(KeyCode::Esc);
        h.char('?');
        transmitted(h.buffer(), &mut ids);
        // Jev answers while the help is open: its pawn on the last move's tint is a new
        // picture, under the help box.
        h.reply("e7e5");
        transmitted(h.buffer(), &mut ids);
        let [help] = overlays(&h.app, h.buffer().area)[..] else {
            panic!("the help box alone is over the board");
        };
        assert!(picture_area(&h, sq("e5")).intersects(help));
        h.char('?');
        assert_eq!(h.app.dialog_name(), None);
        transmitted(h.buffer(), &mut ids);
        h.draw();
        transmitted(h.buffer(), &mut ids);
        assert_eq!(
            ids.len(),
            h.app.piece_images().len(),
            "a picture never sent"
        );
    }

    #[test]
    fn a_kitty_picture_first_drawn_under_the_game_over_box_still_reaches_the_terminal() {
        let mut h = pictures(ProtocolType::Kitty, '1');
        let mut ids = HashSet::new();
        transmitted(h.buffer(), &mut ids);
        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
            transmitted(h.buffer(), &mut ids);
        }
        assert_eq!(h.app.screen(), Screen::GameOver);
        let [game_over] = overlays(&h.app, h.buffer().area)[..] else {
            panic!("the game-over box alone is over the board");
        };
        assert!(picture_area(&h, sq("h4")).intersects(game_over));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.screen(), Screen::Playing);
        transmitted(h.buffer(), &mut ids);
        assert_eq!(
            ids.len(),
            h.app.piece_images().len(),
            "a picture never sent"
        );
    }

    #[test]
    fn closing_the_help_sends_the_pictures_it_covered_again() {
        for protocol in [ProtocolType::Iterm2, ProtocolType::Sixel] {
            let mut h = pictures(protocol, '1');
            // Black's pawn on d6 has its first row above the help box and the rest in it.
            h.moves(&["e4", "d6"]);
            h.char('?');
            let open = frame(&mut h);
            let [help] = overlays(&h.app, open.area)[..] else {
                panic!("the help box alone is over the board");
            };
            h.char('?');
            let closed = frame(&mut h);
            let sent: Vec<(u16, u16)> =
                open.diff(&closed).iter().map(|&(x, y, _)| (x, y)).collect();
            // Pictures the box covered only part of, leaving their first cell alone.
            let mut partly = 0;
            for square in Square::all() {
                let area = picture_area(&h, square);
                let first = area.as_position();
                // A picture's image data is all in its first cell.
                if closed[first].symbol().len() < 16 || !area.intersects(help) {
                    continue;
                }
                assert!(
                    sent.contains(&(first.x, first.y)),
                    "{protocol:?}: {square:?} is not sent again"
                );
                if !help.contains(first) {
                    partly += 1;
                }
            }
            assert!(
                partly > 0,
                "{protocol:?}: the help covers no picture partly"
            );
        }
    }
}
