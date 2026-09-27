//! Drawing every screen (spec sections 6.2 and 6.3).
//!
//! [`draw`] renders an [`App`] through its public accessors only: the menu, the playing
//! screen's panels, the game-over overlay, the top dialog and the too-small notice. It
//! returns the [`HitMap`] of everything clickable, which the app keeps for the next mouse
//! event, and the move-list scroll clamped to what the list can show.
//!
//! Playing layout: the left column holds the Board block, sized for the largest board that
//! fits beside the narrowest side column, with the Command box under it. The right column
//! stacks Status, the computer's panel (titled "Jev" or "Local search", only in games
//! against the computer), Moves and Captured with shared borders. On wide terminals the
//! right column stops growing at [`SIDE_MAX_WIDTH`] and the whole layout is centred.
//!
//! Status has a fixed height per mode and terminal height, so it never jumps. The Jev panel
//! grows with its text (the source, Jev's top three, confidence, latency, model and the
//! note) and takes the rows from Moves, which keeps at least [`MOVES_MIN_ROWS`]; when even
//! that is not enough, only the note is cut short, ending in `…`.
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
use super::board::{BoardGeometry, BoardView, layout_board};
use super::glyphs::{self, ELLIPSIS, GlyphSet, Palette, char_width};
use super::input::LineEditor;
use crate::core::{Color as Side, Game, Piece, PieceKind, Position as ChessPosition};
use crate::engine::{ComputerMove, MoveSource};

/// The help dialog's text. The first [`HELP_KEY_WIDTH`] characters of each line are the
/// key column (drawn bold); no line is wider than 56 cells, so the dialog fits a 60-column
/// terminal.
pub const HELP_LINES: [&str; 13] = [
    "Mouse     click a piece, then a square, or drag it there",
    "Arrows    move the cursor; Enter picks up and puts down",
    "Esc       drop the piece, or leave the command box",
    "/         type a move: e4, Nf3, e2e4, O-O, e8=Q",
    ":         type a command:",
    "            :undo :flip :new :resign :glyphs :help :quit",
    "            :fen <FEN>  :savefen <path>  :savepgn <path>",
    "u  f  n   undo, flip the board, new game",
    "g  m  ?   glyph set, menu, this help",
    "Ctrl+S    save the game as PGN",
    "q         quit (Ctrl+C works everywhere)",
    "Space     pause watching, or retry a failed engine",
    "+  -      watching: slower, faster",
];
/// Width of the key column in [`HELP_LINES`].
pub const HELP_KEY_WIDTH: usize = 10;

/// Narrowest right column; the board shrinks before the column does.
pub const SIDE_MIN_WIDTH: u16 = 30;
/// Widest right column; wider terminals centre the layout instead.
pub const SIDE_MAX_WIDTH: u16 = 48;
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
}

/// Draws `app` into `frame` and returns the click targets and the clamped move-list
/// scroll. `now` is used for the thinking spinner only.
pub fn draw(app: &App, frame: &mut Frame, now: Instant) -> Drawn {
    let mut drawn = Drawn {
        hits: HitMap::default(),
        move_scroll: app.move_scroll(),
    };
    let area = frame.area();
    if is_too_small(area) {
        too_small(frame, area);
        return drawn;
    }
    match app.screen() {
        Screen::Menu => menu(frame, area, app, &mut drawn.hits),
        Screen::Playing => playing(frame, area, app, now, &mut drawn),
        Screen::GameOver => {
            playing(frame, area, app, now, &mut drawn);
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
    let mut top = notes_area.y;
    for note in notes {
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

/// The engine status (green when Jev plays, yellow for the local search alone) and one
/// `! ` note per warning.
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
        marker: "! ",
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

/// The playing screen's two columns in `area`: the left edge of the layout, the board
/// column's width and the right column's width.
fn columns(area: Rect) -> (u16, u16, u16) {
    // The largest board whose block fits beside the narrowest right column and above the
    // command box.
    let room = Rect::new(
        0,
        0,
        area.width
            .saturating_sub(SIDE_MIN_WIDTH.saturating_add(BOARD_CHROME_WIDTH)),
        area.height
            .saturating_sub(COMMAND_HEIGHT.saturating_add(BOARD_CHROME_HEIGHT)),
    );
    let board_width = layout_board(room, false).map_or(area.width / 2, |g| {
        g.outer.width.saturating_add(BOARD_CHROME_WIDTH)
    });
    let side_width = area.width.saturating_sub(board_width).min(SIDE_MAX_WIDTH);
    let x = area.x
        + area
            .width
            .saturating_sub(board_width.saturating_add(side_width))
            / 2;
    (x, board_width, side_width)
}

/// Text width inside the right column's panels.
fn side_text_width(area: Rect) -> u16 {
    let (_, _, side_width) = columns(area);
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

/// Lays out the playing screen in `area` (not [`is_too_small`]). The Status panel gets
/// `status_rows` text rows; the Jev panel, when `jev_rows` is given, gets that many (at
/// least [`JEV_MIN_ROWS`]) as long as Moves keeps [`MOVES_MIN_ROWS`]; Moves gets the rest.
fn playing_layout(area: Rect, status_rows: u16, jev_rows: Option<u16>) -> PlayingLayout {
    let (x, board_width, side_width) = columns(area);
    let command_height = COMMAND_HEIGHT.min(area.height);
    let board = Rect::new(x, area.y, board_width, area.height - command_height);
    let command = Rect::new(x, board.bottom(), board_width, command_height);

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

/// The board, the command box and the side panels.
fn playing(frame: &mut Frame, area: Rect, app: &App, now: Instant, drawn: &mut Drawn) {
    let text_width = side_text_width(area);
    let jev = (app.mode() != Mode::HumanVsHuman)
        .then(|| JevText::new(app.last_computer(), app.engine_status(), text_width));
    let layout = playing_layout(
        area,
        status_rows(app.mode(), area.height),
        jev.as_ref().map(JevText::rows),
    );
    drawn.hits.board = board_panel(frame, layout.board, app);
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

/// The Board block with the board centred in it; returns the geometry for hit-testing.
fn board_panel(frame: &mut Frame, area: Rect, app: &App) -> Option<BoardGeometry> {
    let block = Block::bordered()
        .title(" Board ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let geometry = layout_board(inner, app.flipped())?;
    let highlights = app.highlights();
    frame.render_widget(
        BoardView {
            position: app.game().position(),
            geometry,
            glyphs: app.glyphs(),
            palette: app.palette(),
            highlights: &highlights,
            no_color: app.no_color(),
        },
        inner,
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

/// Status: the mode on the top border (the first of [`App::mode_labels`] that fits, so a
/// narrow panel shows `You (W) vs Local` rather than nothing), then whose turn it is, the
/// Jev vs Jev pace, the thinking spinner and the latest message.
fn status_panel(frame: &mut Frame, area: Rect, app: &App, now: Instant) {
    const TITLE: &str = " Status ";
    let block = side_block("Status");
    let inner = block.inner(area);
    // Two corners and at least two border cells between the titles.
    let room = usize::from(area.width).saturating_sub(TITLE.len() + 4);
    let mode = app
        .mode_labels()
        .into_iter()
        .map(|label| format!(" {label} "))
        .find(|mode| Span::raw(mode.as_str()).width() <= room);
    let block = match mode {
        Some(mode) => block.title_top(Line::from(mode).right_aligned()),
        None => block,
    };
    let lines = status_lines(app, now, inner.width, inner.height);
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
/// message is what gets cut when the panel is full: whose turn it is, the Jev vs Jev pace
/// (the only place the pause and step delay are shown), the thinking spinner (or the wait
/// for an earlier request), and the latest message in the rows left ([`fit_message`]). The
/// turn and thinking lines drop words rather than wrap (the computer's name "Local search"
/// is long), so they keep one row each; a game's outcome has no brief form and may wrap,
/// and the message gets the rows its wrapped lines leave.
fn status_lines(app: &App, now: Instant, width: u16, rows: u16) -> Vec<Line<'static>> {
    let game = app.game();
    let in_check = game.outcome().is_none() && game.position().is_check();
    let turn = Line::from(fitted(app.turn_text(), app.turn_text_brief(), width)).bold();
    let mut lines = vec![if in_check { turn.red() } else { turn }];
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
fn fit_message(text: &str, path: Option<&str>, width: u16, rows: u16) -> String {
    let Some(path) = path else {
        return fit_rows(text, width, rows);
    };
    let fits = |candidate: &String| wrapped_height(candidate, width) <= rows;
    let full = format!("{text}{path}");
    if fits(&full) {
        return full;
    }
    let shorter: Vec<String> = elided_paths(path)
        .into_iter()
        .map(|short| format!("{text}{short}"))
        .collect();
    match shorter.iter().find(|candidate| fits(candidate)) {
        Some(fitting) => fitting.clone(),
        None => cut_to_fit(shorter.last().unwrap_or(&full), width, rows),
    }
}

/// Shorter forms of `path`, longest first: the folders after its first one (`~`, `/tmp`,
/// `games`) are replaced by `…` from the left, a folder at a time, down to none; then the
/// first one goes too. The file name is always kept.
fn elided_paths(path: &str) -> Vec<String> {
    let Some((folders, name)) = path.rsplit_once('/') else {
        return Vec::new();
    };
    let mut parts: Vec<&str> = folders.split('/').collect();
    // An absolute path's first folder keeps its leading slash: `/tmp`, not ``.
    let head = if parts.first() == Some(&"") && parts.len() > 1 {
        parts.remove(0);
        format!("/{}", parts.remove(0))
    } else {
        parts.remove(0).to_string()
    };
    let mut shorter: Vec<String> = (0..parts.len())
        .rev()
        .map(|kept| {
            let tail = &parts[parts.len() - kept..];
            let mut short = format!("{head}/{ELLIPSIS}");
            for part in tail {
                short.push('/');
                short.push_str(part);
            }
            short.push('/');
            short.push_str(name);
            short
        })
        .collect();
    shorter.push(format!("{ELLIPSIS}/{name}"));
    shorter
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
/// with [`ELLIPSIS`] appended, cut between words when any such cut fits.
fn fit_rows(text: &str, width: u16, rows: u16) -> String {
    let fits = |cut: &String| wrapped_height(cut, width) <= rows;
    if wrapped_height(text, width) <= rows {
        return text.to_string();
    }
    let cut_at = |end: usize| format!("{}{ELLIPSIS}", text[..end].trim_end());
    text.char_indices()
        .rev()
        .filter(|&(_, c)| c.is_whitespace())
        .map(|(end, _)| cut_at(end))
        .find(fits)
        .unwrap_or_else(|| cut_to_fit(text, width, rows))
}

/// The longest start of `text` that wraps into `rows` rows of `width` cells with
/// [`ELLIPSIS`] appended, cut between any two characters.
fn cut_to_fit(text: &str, width: u16, rows: u16) -> String {
    let cut_at = |end: usize| format!("{}{ELLIPSIS}", text[..end].trim_end());
    text.char_indices()
        .rev()
        .map(|(end, _)| cut_at(end))
        .find(|cut| wrapped_height(cut, width) <= rows)
        .unwrap_or_else(|| ELLIPSIS.to_string())
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

// ----- overlays -----

/// The game-over overlay: the result and the New game / Save PGN / Menu buttons.
fn game_over(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let Some(outcome) = app.game().outcome() else {
        return;
    };
    let inner = dialog_frame(frame, centered(area, 46, 7), "Game over");
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

/// Draws the top dialog and records its targets (none for help: any click closes it).
fn dialog(frame: &mut Frame, area: Rect, app: &App, top: &Dialog, hits: &mut HitMap) {
    match top {
        Dialog::Help => help(frame, area),
        Dialog::Promotion { choice, .. } => promotion(frame, area, *choice, app, hits),
        Dialog::Input {
            purpose,
            editor,
            error,
        } => input(
            frame,
            area,
            top.title(),
            *purpose,
            editor,
            error.as_ref(),
            hits,
        ),
        Dialog::Confirm { question, yes } => {
            confirm(frame, area, top.title(), question, *yes, hits)
        }
    }
}

/// Keys and commands.
fn help(frame: &mut Frame, area: Rect) {
    let height = u16::try_from(HELP_LINES.len()).map_or(u16::MAX, |n| n.saturating_add(2));
    let rect = centered(area, 60, height);
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

/// The promotion picker: Queen, Rook, Bishop and Knight buttons in the colour of the side
/// to move (the one promoting), `choice` highlighted.
fn promotion(frame: &mut Frame, area: Rect, choice: usize, app: &App, hits: &mut HitMap) {
    let side = app.game().position().side_to_move();
    let inner = dialog_frame(frame, centered(area, 58, 5), "Promote to");
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

/// A text field dialog: prompt, field with the terminal cursor, error and buttons.
fn input(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    purpose: InputPurpose,
    editor: &LineEditor,
    error: Option<&Message>,
    hits: &mut HitMap,
) {
    let inner = dialog_frame(frame, centered(area, 66, 8), title);
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

/// A yes/no question; the dialog grows with the question's text.
fn confirm(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    question: &Question,
    yes: bool,
    hits: &mut HitMap,
) {
    const WIDTH: u16 = 56;
    let text = question.text();
    let lines = match question {
        // The path on its own rows, so a long one never splits the question.
        Question::Overwrite { path, .. } => {
            let path = path.display().to_string();
            let rest = text.strip_prefix(&path).unwrap_or(&text).trim_start();
            let rest = rest.to_string();
            vec![path, rest]
        }
        Question::Quit | Question::Resign | Question::NewGame | Question::Menu => vec![text],
    };
    let text_width = WIDTH.saturating_sub(4);
    let text_rows = lines
        .iter()
        .map(|line| wrapped_height(line, text_width))
        .fold(0u16, u16::saturating_add);
    // Borders, text, gap, buttons.
    let inner = dialog_frame(
        frame,
        centered(area, WIDTH, text_rows.saturating_add(4)),
        title,
    );
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
/// wrapper, with words longer than a row split between characters.
fn wrapped_height(text: &str, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    let mut rows = 1usize;
    let mut used = 0usize;
    for word in text.split_whitespace() {
        let w = Span::raw(word).width();
        if used > 0 && used + 1 + w <= width {
            used += 1 + w;
            continue;
        }
        if used > 0 {
            rows += 1;
            used = 0;
        }
        if w <= width {
            used = w;
            continue;
        }
        for c in word.chars() {
            let cw = char_width(c);
            if used > 0 && used + cw > width {
                rows += 1;
                used = 0;
            }
            used += cw;
        }
    }
    u16::try_from(rows).unwrap_or(u16::MAX)
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
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::KeyCode;
    use ratatui::widgets::Widget;

    use super::*;
    use crate::core::START_FEN;
    use crate::tui::event::AppEvent;
    use crate::tui::glyphs::initial_glyphs;
    use crate::tui::test_support::engine::{FakeEngine, JEV_STATUS};
    use crate::tui::test_support::harness::Harness;
    use crate::tui::test_support::{PROMOTION_FEN, game_from, sq};
    use crate::tui::worker::LOCAL_SEARCH_STATUS;

    /// An app playing against Jev (with a key), in a `width`×`height` terminal.
    fn jev(width: u16, height: u16) -> Harness {
        Harness::sized(FakeEngine::jev(), width, height)
    }

    /// The text rows inside the panel titled `title` in the right column, trimmed.
    fn panel_rows(h: &Harness, title: &str) -> Vec<String> {
        let buffer = h.buffer();
        let area = buffer.area;
        let layout = playing_layout(area, status_rows(h.app.mode(), area.height), None);
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

    #[test]
    fn playing_layout_fits_every_size() {
        let sizes = [
            ((60, 20), (3, 1)),
            ((80, 24), (5, 2)),
            ((100, 30), (7, 3)),
            ((120, 40), (7, 3)),
            ((200, 60), (7, 3)),
            ((70, 50), (3, 1)),
        ];
        for ((width, height), square) in sizes {
            let area = Rect::new(0, 0, width, height);
            for with_jev in [false, true] {
                let mode = if with_jev {
                    Mode::JevVsJev
                } else {
                    Mode::HumanVsHuman
                };
                let jev_rows = with_jev.then_some(40);
                let l = playing_layout(area, status_rows(mode, height), jev_rows);
                let at = format!("{width}x{height} jev={with_jev}");
                let inner = Block::bordered()
                    .padding(Padding::horizontal(1))
                    .inner(l.board);
                let board = layout_board(inner, false).expect("board fits");
                assert_eq!((board.square_w, board.square_h), square, "{at}");
                assert_eq!(board.outer.width, inner.width, "no spare columns: {at}");
                assert_eq!(
                    (l.command.x, l.command.y, l.command.width, l.command.height),
                    (l.board.x, l.board.bottom(), l.board.width, COMMAND_HEIGHT),
                    "{at}"
                );
                assert_eq!(l.command.bottom(), area.bottom(), "{at}");
                let side = l.status.width;
                assert!((SIDE_MIN_WIDTH..=SIDE_MAX_WIDTH).contains(&side), "{at}");
                assert_eq!(l.status.x, l.board.right(), "{at}");
                // Centred: the margins differ by at most one column.
                let left = l.board.x - area.x;
                let right = area.right() - l.status.right();
                assert!(left.abs_diff(right) <= 1, "{at}");
                // The right column: full height, one shared border row between panels.
                let column: Vec<Rect> = [Some(l.status), l.jev, Some(l.moves), Some(l.captured)]
                    .into_iter()
                    .flatten()
                    .collect();
                assert_eq!(column.len(), if with_jev { 4 } else { 3 }, "{at}");
                assert_eq!(column[0].y, area.y, "{at}");
                for pair in column.windows(2) {
                    assert_eq!(pair[1].y, pair[0].bottom() - 1, "{at}");
                    assert_eq!((pair[1].x, pair[1].width), (l.status.x, side), "{at}");
                }
                assert_eq!(l.captured.bottom(), area.bottom(), "{at}");
                assert!(
                    l.moves.height >= MOVES_MIN_ROWS + 2,
                    "room for three moves: {at}"
                );
                assert_eq!(l.status.height, status_rows(mode, height) + 2, "{at}");
                if let Some(jev) = l.jev {
                    assert!(jev.height >= JEV_MIN_ROWS + 2, "{at}");
                }
            }
        }
    }

    #[test]
    fn the_jev_panel_grows_with_its_text_and_moves_gives_way() {
        let area = Rect::new(0, 0, 80, 24);
        let rows = status_rows(Mode::HumanVsJev { human: Side::White }, 24);
        let short = playing_layout(area, rows, Some(1));
        let long = playing_layout(area, rows, Some(6));
        assert_eq!(short.jev.expect("jev").height, JEV_MIN_ROWS + 2);
        assert_eq!(long.jev.expect("jev").height, 6 + 2);
        assert_eq!(short.moves.height - long.moves.height, 6 - JEV_MIN_ROWS);
        let huge = playing_layout(area, rows, Some(100));
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
        let buffer = h.buffer();
        let area = buffer.area;
        let status = playing_layout(area, status_rows(h.app.mode(), area.height), None).status;
        (status.x..status.right())
            .map(|x| buffer[(x, status.y)].symbol())
            .collect()
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

        h.moves(&["f3", "e5", "g4", "Qh4#"]);
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
        let (_, glyph_warnings) = initial_glyphs(Some("fancy"), |_| None);
        let h = Harness::build(engine, (80, 24), glyph_warnings, |app| app);
        insta::assert_snapshot!("menu_80x24", h.terminal.backend());
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

    #[test]
    fn snapshot_too_small() {
        let h = jev(50, 12);
        insta::assert_snapshot!("too_small_50x12", h.terminal.backend());
    }
}
