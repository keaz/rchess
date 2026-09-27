//! The application state machine (spec sections 6.2, 6.4 and 6.5).
//!
//! [`App`] owns the game and all UI state. The run loop feeds it [`AppEvent`]s through
//! [`App::handle`] and draws it with [`App::render`]; nothing else changes it. It never
//! blocks and never spawns threads: when the computer should move, `handle` returns an
//! [`Action::RequestEngine`] and the run loop starts the worker. The only I/O it does is
//! writing the files the user asks it to save.
//!
//! Screens are [`Screen::Menu`], [`Screen::Playing`] and [`Screen::GameOver`] (an overlay on
//! the playing screen), with a stack of [`Dialog`]s on top. Input goes to the topmost focus
//! only: the top dialog, else the game-over overlay, else the command box when it has
//! focus, else the board. Ctrl+C is the one global key: it asks to quit from anywhere.
//! Ctrl+S opens the PGN save dialog from the board, the command box and the game-over
//! overlay. Esc typed quickly before a key arrives as Alt+key: outside text fields it is
//! handled as both keys; in the command box and the text dialogs Alt chords are ignored, so
//! readline habits (Alt+B, Alt+F, ...) neither change the text nor reach the board.
//!
//! The computer player is called "Jev" when the engine uses Jev ([`Engine::uses_jev`]) and
//! "Local search" otherwise, in every label, message and saved PGN.
//!
//! Mouse events are hit-tested against the [`HitMap`] recorded by the most recent
//! [`App::render`], so a click before the first draw does nothing. Drawing lives in
//! [`panels`], which reads the app only through its public accessors and
//! returns the hit map: the board geometry, a [`Hit`] for every menu row, the command box,
//! the move list, and the buttons of the top dialog or the game-over overlay.
//!
//! Engine requests carry a generation counter and the position hash. The generation is
//! bumped whenever the game is replaced or rewound (undo, new game, load, resign, menu), and
//! a reply is applied only when [`is_current`] matches both, so a late reply is discarded
//! instead of being played in a position the engine never saw. Engine threads cannot be
//! cancelled, so a discarded request keeps running (and may be a paid Jev call with
//! retries); at most [`MAX_IN_FLIGHT`] requests are out at once, and a new one waits until
//! an old one has answered.
//!
//! The app only ever applies moves the worker computed. When the worker could not produce
//! one ([`EngineOutcome::Failed`]), the app stops asking and says so; space retries, from
//! the board or an empty command box.
//!
//! Actions that throw away a game in progress (quit, new game, menu, resign) ask first, and
//! their confirmation defaults to No, so a stray Enter never confirms one.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position as CellPosition, Rect};

use super::board::{BoardGeometry, Highlights, square_at};
use super::event::AppEvent;
use super::files::{SaveError, pgn_export, resolve_path, today, write_file};
use super::glyphs::{self, GlyphSet, Palette};
use super::input::{Command, LineEditor, parse_command};
use super::movetext::{MoveTextError, parse_move};
use super::panels;
use super::worker::{
    ENGINE_ERROR_NOTE, Engine, EngineOutcome, EngineReply, EngineRequest, is_current,
};
use crate::core::{ChessError, Color as Side, Game, Move, Outcome, PieceKind, Square};
use crate::engine::ComputerMove;

/// Smallest terminal the UI draws in; below it only [`TOO_SMALL`] is shown.
pub const MIN_WIDTH: u16 = 60;
/// Smallest terminal height the UI draws in; see [`MIN_WIDTH`].
pub const MIN_HEIGHT: u16 = 20;
/// The whole screen when the terminal is smaller than [`MIN_WIDTH`] × [`MIN_HEIGHT`].
pub const TOO_SMALL: &str = "Terminal too small (need 60×20)";

/// Engine requests allowed out at once: the live one plus one discarded one still running.
/// Undo, new game or menu while the engine thinks leave the old thread running (it cannot
/// be cancelled), so without a cap a held key would start a burst of paid Jev calls (or of
/// CPU-bound searches without a key).
pub const MAX_IN_FLIGHT: usize = 2;

/// Status message when the worker could produce no move at all; the app stops asking.
/// Space retries on the board and in an empty command box, so only typing gets in the way.
pub const ENGINE_FAILED: &str = "the engine failed; space retries unless typing";
/// Status line while a new request waits for [`MAX_IN_FLIGHT`] to allow it.
pub const WAITING_FOR_ENGINE: &str = "waiting for an old request";

/// The computer player's name when it uses Jev.
pub const JEV_NAME: &str = "Jev";
/// The computer player's name without a Jev key: the local search plays alone.
pub const LOCAL_SEARCH_NAME: &str = "Local search";
/// [`LOCAL_SEARCH_NAME`] in the brief mode titles, where there is no room for it.
pub const LOCAL_SEARCH_SHORT_NAME: &str = "Local";

/// Jev vs Jev pauses between moves, shortest first. `-` and `+` step through them.
pub const STEP_DELAYS: [Duration; 10] = [
    Duration::from_millis(200),
    Duration::from_millis(300),
    Duration::from_millis(500),
    Duration::from_millis(700),
    Duration::from_millis(1000),
    Duration::from_millis(1500),
    Duration::from_millis(2000),
    Duration::from_millis(3000),
    Duration::from_millis(4000),
    Duration::from_millis(5000),
];
/// Jev vs Jev pause between moves when a game starts.
pub const DEFAULT_STEP_DELAY: Duration = Duration::from_secs(1);

/// Promotion picker order; `q`, `r`, `b` and `n` pick directly.
pub const PROMOTION_CHOICES: [PieceKind; 4] = [
    PieceKind::Queen,
    PieceKind::Rook,
    PieceKind::Bishop,
    PieceKind::Knight,
];

/// Menu entries, top to bottom. The digits `1`..=`7` start them directly.
pub const MENU_ITEMS: [MenuItem; 7] = [
    MenuItem::HumanVsHuman,
    MenuItem::HumanVsJev(SidePick::White),
    MenuItem::HumanVsJev(SidePick::Black),
    MenuItem::HumanVsJev(SidePick::Random),
    MenuItem::JevVsJev,
    MenuItem::LoadFen,
    MenuItem::Quit,
];

/// Game-over overlay buttons, left to right; `n`, `s` and `m` press them directly.
pub const GAME_OVER_BUTTONS: [Button; 3] = [Button::NewGame, Button::SavePgn, Button::Menu];

/// Spinner frames. ASCII, so every frame is one cell wide in any font.
const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
/// Milliseconds each spinner frame is shown.
const SPINNER_FRAME_MS: u128 = 100;

const GAME_IS_OVER: &str = "the game is over (u: undo, n: new game)";
const NOTHING_TO_UNDO: &str = "nothing to undo";
const NOTHING_TO_RESIGN: &str = "nothing to resign while watching";
const NOT_SAVED: &str = "not saved";
const RETRYING: &str = "asking the engine again";

/// True when `area` is below [`MIN_WIDTH`] × [`MIN_HEIGHT`], so only [`TOO_SMALL`] is drawn.
pub const fn is_too_small(area: Rect) -> bool {
    area.width < MIN_WIDTH || area.height < MIN_HEIGHT
}

/// Who plays which side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Two people share the keyboard and mouse.
    HumanVsHuman,
    /// The person plays `human`; the computer (Jev or the local search) plays the other.
    HumanVsJev {
        /// The person's side.
        human: Side,
    },
    /// The computer plays both sides; the person watches.
    JevVsJev,
}

impl Mode {
    /// True when the engine plays `side` in this mode.
    pub fn engine_plays(self, side: Side) -> bool {
        match self {
            Mode::HumanVsHuman => false,
            Mode::HumanVsJev { human } => side != human,
            Mode::JevVsJev => true,
        }
    }

    /// A short label with the computer called `computer`: `Human vs Human`,
    /// `You (White) vs Jev` or `Local search vs Local search`.
    pub fn label(self, computer: &str) -> String {
        match self {
            Mode::HumanVsHuman => "Human vs Human".to_string(),
            Mode::HumanVsJev { human } => format!("You ({human}) vs {computer}"),
            Mode::JevVsJev => format!("{computer} vs {computer}"),
        }
    }

    /// Labels to try in turn until one fits, longest first: [`label`](Self::label), then
    /// briefer forms with the computer called `short`. Human vs computer as White gives
    /// `You (White) vs Local search`, `You (W) vs Local`, `W You · B Local`; watching
    /// gives `Local search vs Local search`, `Local vs Local`. No label repeats, so with
    /// Jev watching is just `Jev vs Jev`.
    pub fn labels(self, computer: &str, short: &str) -> Vec<String> {
        let mut labels = vec![self.label(computer)];
        match self {
            Mode::HumanVsHuman => {}
            Mode::HumanVsJev { human } => {
                let initial = |side: Side| if side == Side::White { 'W' } else { 'B' };
                let (white, black) = if human == Side::White {
                    ("You", short)
                } else {
                    (short, "You")
                };
                labels.push(format!("You ({}) vs {short}", initial(human)));
                labels.push(format!("W {white} · B {black}"));
            }
            Mode::JevVsJev => labels.push(format!("{short} vs {short}")),
        }
        labels.dedup();
        labels
    }
}

/// The screen under any dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Game selection.
    Menu,
    /// A game is shown (possibly finished, when the game-over overlay was dismissed).
    Playing,
    /// The playing screen with the game-over overlay on top.
    GameOver,
}

impl Screen {
    /// `menu`, `playing` or `game over`.
    pub const fn name(self) -> &'static str {
        match self {
            Screen::Menu => "menu",
            Screen::Playing => "playing",
            Screen::GameOver => "game over",
        }
    }
}

/// Work the run loop does on the app's behalf.
#[derive(Clone, Debug)]
pub enum Action {
    /// Run this request on an engine thread (`worker::spawn_request`) and deliver the reply
    /// as [`AppEvent::Engine`]. If the thread cannot be spawned, deliver an
    /// [`EngineOutcome::Failed`] reply carrying the request's generation and hash instead:
    /// every request must be answered exactly once, or the app waits forever (and counts
    /// it against [`MAX_IN_FLIGHT`]).
    RequestEngine(EngineRequest),
}

/// Which side a Human vs Jev game gives the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePick {
    /// The person plays White.
    White,
    /// The person plays Black (the board starts flipped).
    Black,
    /// A coin flip when the game starts.
    Random,
}

/// A menu entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// Start a Human vs Human game.
    HumanVsHuman,
    /// Start a Human vs Jev game.
    HumanVsJev(SidePick),
    /// Start watching Jev play itself.
    JevVsJev,
    /// Open the FEN dialog; the position is played Human vs Human.
    LoadFen,
    /// Leave the program.
    Quit,
}

impl MenuItem {
    /// The text shown in the menu, with the computer called `computer`.
    pub fn label(self, computer: &str) -> String {
        match self {
            MenuItem::HumanVsHuman => "Human vs Human".to_string(),
            MenuItem::HumanVsJev(SidePick::White) => format!("Human vs {computer}: play White"),
            MenuItem::HumanVsJev(SidePick::Black) => format!("Human vs {computer}: play Black"),
            MenuItem::HumanVsJev(SidePick::Random) => format!("Human vs {computer}: random side"),
            MenuItem::JevVsJev => format!("{computer} vs {computer} (watch)"),
            MenuItem::LoadFen => "Load FEN".to_string(),
            MenuItem::Quit => "Quit".to_string(),
        }
    }
}

/// What a save writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    /// The whole game as PGN.
    Pgn,
    /// The current position as one FEN line.
    Fen,
}

impl SaveKind {
    /// File extension added when the typed name has none.
    pub const fn extension(self) -> &'static str {
        match self {
            SaveKind::Pgn => "pgn",
            SaveKind::Fen => "fen",
        }
    }
}

/// What a text-input dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputPurpose {
    /// Load a FEN. From the menu the position is played Human vs Human; otherwise in the
    /// current mode.
    LoadFen {
        /// Opened from the menu.
        from_menu: bool,
    },
    /// Save to the typed path.
    Save(SaveKind),
}

/// Where a save was typed, so an overwrite prompt answered No can give the text back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOrigin {
    /// The save dialog; No reopens it with the typed path.
    Dialog,
    /// A `:savepgn` or `:savefen` command; No puts the command back in the command box.
    Command,
}

/// A yes/no question. Every question starts on No.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// Quit while a game is in progress.
    Quit,
    /// Resign the game.
    Resign,
    /// Throw away the game in progress for a new one.
    NewGame,
    /// Throw away the game in progress and go back to the menu.
    Menu,
    /// Replace an existing file. Holds exactly what will be written.
    Overwrite {
        /// PGN or FEN.
        kind: SaveKind,
        /// The resolved path that already exists.
        path: PathBuf,
        /// The file contents, captured when the save was asked for.
        contents: String,
        /// The path as typed, given back when the answer is No.
        typed: String,
        /// Where it was typed.
        origin: SaveOrigin,
    },
}

impl Question {
    /// The question as shown in its dialog.
    pub fn text(&self) -> String {
        match self {
            Question::Quit => "Quit the game in progress?".to_string(),
            Question::Resign => "Resign this game?".to_string(),
            Question::NewGame => "Abandon the game in progress and start a new one?".to_string(),
            Question::Menu => "Abandon the game in progress and go to the menu?".to_string(),
            Question::Overwrite { path, .. } => format!("{} exists. Overwrite it?", path.display()),
        }
    }
}

/// A modal dialog. The top of the stack gets every key and click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// Pick the promotion piece for a board move from `from` to `to`.
    Promotion {
        /// Pawn square.
        from: Square,
        /// Promotion square.
        to: Square,
        /// Index into [`PROMOTION_CHOICES`] highlighted for Enter.
        choice: usize,
    },
    /// A one-line text field (FEN or save path).
    Input {
        /// What Enter does with the text.
        purpose: InputPurpose,
        /// The text being typed.
        editor: LineEditor,
        /// Why the last submission failed; shown under the field.
        error: Option<String>,
    },
    /// A yes/no question.
    Confirm {
        /// What is being asked.
        question: Question,
        /// True when Enter answers yes (false when the dialog opens).
        yes: bool,
    },
    /// Keys and commands ([`panels::HELP_LINES`]).
    Help,
}

impl Dialog {
    /// A stable name for tests and logs: `promotion`, `load fen`, `save pgn`, `save fen`,
    /// `quit`, `resign`, `new game`, `menu`, `overwrite` or `help`.
    pub const fn name(&self) -> &'static str {
        match self {
            Dialog::Promotion { .. } => "promotion",
            Dialog::Input { purpose, .. } => match purpose {
                InputPurpose::LoadFen { .. } => "load fen",
                InputPurpose::Save(SaveKind::Pgn) => "save pgn",
                InputPurpose::Save(SaveKind::Fen) => "save fen",
            },
            Dialog::Confirm { question, .. } => match question {
                Question::Quit => "quit",
                Question::Resign => "resign",
                Question::NewGame => "new game",
                Question::Menu => "menu",
                Question::Overwrite { .. } => "overwrite",
            },
            Dialog::Help => "help",
        }
    }

    /// The dialog's border title.
    pub const fn title(&self) -> &'static str {
        match self {
            Dialog::Promotion { .. } => "Promote to",
            Dialog::Input { purpose, .. } => match purpose {
                InputPurpose::LoadFen { .. } => "Load FEN",
                InputPurpose::Save(SaveKind::Pgn) => "Save PGN",
                InputPurpose::Save(SaveKind::Fen) => "Save FEN",
            },
            Dialog::Confirm { question, .. } => match question {
                Question::Quit => "Quit",
                Question::Resign => "Resign",
                Question::NewGame => "New game",
                Question::Menu => "Menu",
                Question::Overwrite { .. } => "Overwrite",
            },
            Dialog::Help => "Help",
        }
    }
}

/// A clickable button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Yes, or the input dialog's Load/Save.
    Confirm,
    /// No, or close the dialog.
    Cancel,
    /// Game over: start a new game in the same mode.
    NewGame,
    /// Game over: save the game as PGN.
    SavePgn,
    /// Game over: back to the menu.
    Menu,
}

/// Something the mouse can hit, recorded while drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Index into [`MENU_ITEMS`].
    MenuItem(usize),
    /// A button of the top dialog or the game-over overlay.
    Button(Button),
    /// A promotion picker choice.
    Promote(PieceKind),
    /// The command box (a click focuses it).
    CommandBox,
    /// The move list (the wheel scrolls it while the pointer is over it).
    MoveList,
}

/// Screen areas saved during the last draw, for mouse hit-testing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitMap {
    /// The board as drawn, when it was drawn.
    pub board: Option<BoardGeometry>,
    /// Clickable areas in drawing order; later entries are on top.
    pub areas: Vec<(Rect, Hit)>,
}

impl HitMap {
    /// Records `hit` at `rect` (empty rects are ignored).
    pub fn push(&mut self, rect: Rect, hit: Hit) {
        if !rect.is_empty() {
            self.areas.push((rect, hit));
        }
    }

    /// The topmost target under cell (`column`, `row`).
    pub fn at(&self, column: u16, row: u16) -> Option<Hit> {
        let cell = CellPosition::new(column, row);
        self.areas
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(cell))
            .map(|&(_, hit)| hit)
    }

    /// Where `hit` was drawn (its topmost entry).
    pub fn rect_of(&self, hit: Hit) -> Option<Rect> {
        self.areas
            .iter()
            .rev()
            .find(|&&(_, h)| h == hit)
            .map(|&(rect, _)| rect)
    }
}

/// A status message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// One line of text.
    pub text: String,
    /// Errors are drawn in red.
    pub is_error: bool,
}

/// The engine is working on a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thinking {
    /// Time since the request was sent.
    pub elapsed: Duration,
    /// The spinner frame for `elapsed` (one ASCII cell).
    pub spinner: &'static str,
}

/// The request in flight for the current generation.
#[derive(Clone, Copy, Debug)]
struct Pending {
    since: Instant,
}

/// A piece picked up with the mouse button still held.
#[derive(Clone, Copy, Debug)]
struct Drag {
    from: Square,
    /// The press was on the already selected piece: releasing on the same square
    /// deselects it (a second click), releasing elsewhere drops it there.
    release_deselects: bool,
}

/// The whole TUI state. See the [module documentation](self).
pub struct App {
    engine: Arc<dyn Engine>,
    engine_status: String,
    /// [`Engine::uses_jev`], read once: names the computer "Jev" or "Local search".
    uses_jev: bool,
    warnings: Vec<String>,
    palette: Palette,
    glyphs: GlyphSet,
    pick_side: fn() -> Side,
    today: fn() -> String,
    home: Option<PathBuf>,

    screen: Screen,
    mode: Mode,
    game: Game,
    flipped: bool,
    dialogs: Vec<Dialog>,
    menu_index: usize,
    game_over_choice: usize,

    selected: Option<Square>,
    cursor: Option<Square>,
    drag: Option<Drag>,
    command: LineEditor,
    command_focused: bool,
    message: Option<Message>,
    move_scroll: usize,

    generation: u64,
    pending: Option<Pending>,
    /// Requests sent and not yet answered, current or discarded.
    in_flight: usize,
    /// The last computer move and the number of plies in the game once it was played, so
    /// undoing it also clears the Jev panel.
    last_computer: Option<(usize, ComputerMove)>,
    /// The worker produced no move; no request is sent until space retries or the
    /// position changes.
    engine_failed: bool,
    /// Jev vs Jev is paused: no request is sent and no move is played.
    paused: bool,
    /// An answer that arrived while paused, applied when play resumes.
    held: Option<EngineOutcome>,
    step_delay: Duration,
    /// When the last Jev vs Jev move was applied; the next request is due one step
    /// delay later. `None` means due now.
    step_anchor: Option<Instant>,

    hits: HitMap,
    /// The last draw showed only [`TOO_SMALL`]; input is ignored until the next draw fits.
    too_small: bool,
    quit: bool,
}

impl App {
    /// A new app on the menu screen.
    ///
    /// `warnings` are startup notes (such as an unknown `--glyphs` value); the engine's own
    /// [`Engine::warnings`] are added here, so callers must not pass them again (duplicates
    /// are dropped). `HOME` is read once, for `~` in save paths.
    pub fn new(
        engine: Arc<dyn Engine>,
        glyphs: GlyphSet,
        truecolor: bool,
        warnings: Vec<String>,
    ) -> App {
        let engine_status = engine.status();
        let uses_jev = engine.uses_jev();
        let mut all_warnings = engine.warnings();
        for warning in warnings {
            if !all_warnings.contains(&warning) {
                all_warnings.push(warning);
            }
        }
        App {
            engine,
            engine_status,
            uses_jev,
            warnings: all_warnings,
            palette: glyphs::palette(truecolor),
            glyphs,
            pick_side: random_side,
            today,
            home: std::env::var_os("HOME").map(PathBuf::from),
            screen: Screen::Menu,
            mode: Mode::HumanVsHuman,
            game: Game::new(),
            flipped: false,
            dialogs: Vec::new(),
            menu_index: 0,
            game_over_choice: 0,
            selected: None,
            cursor: None,
            drag: None,
            command: LineEditor::new(),
            command_focused: false,
            message: None,
            move_scroll: 0,
            generation: 0,
            pending: None,
            in_flight: 0,
            last_computer: None,
            engine_failed: false,
            paused: false,
            held: None,
            step_delay: DEFAULT_STEP_DELAY,
            step_anchor: None,
            hits: HitMap::default(),
            too_small: false,
            quit: false,
        }
    }

    /// Replaces the coin flip used by "Human vs Jev: random side" (tests pass a fixed side).
    #[must_use]
    pub fn with_side_picker(mut self, pick: fn() -> Side) -> App {
        self.pick_side = pick;
        self
    }

    /// Replaces the home folder used to expand `~` in save paths (`None`: no expansion).
    #[must_use]
    pub fn with_home(mut self, home: Option<PathBuf>) -> App {
        self.home = home;
        self
    }

    /// Replaces the source of the PGN `Date` tag (default: [`today`]).
    #[must_use]
    pub fn with_date(mut self, today: fn() -> String) -> App {
        self.today = today;
        self
    }

    // ----- read accessors -----

    /// The engine, for the run loop's `worker::spawn_request`.
    pub fn engine(&self) -> &Arc<dyn Engine> {
        &self.engine
    }

    /// The engine's one-line status for the menu.
    pub fn engine_status(&self) -> &str {
        &self.engine_status
    }

    /// True when the computer's moves come from Jev ([`Engine::uses_jev`]).
    pub fn uses_jev(&self) -> bool {
        self.uses_jev
    }

    /// What the UI calls the computer player: [`JEV_NAME`] or [`LOCAL_SEARCH_NAME`].
    pub fn computer_name(&self) -> &'static str {
        if self.uses_jev {
            JEV_NAME
        } else {
            LOCAL_SEARCH_NAME
        }
    }

    /// Startup and engine warnings for the menu.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The game being shown (the last one played while on the menu).
    pub fn game(&self) -> &Game {
        &self.game
    }

    /// The current (or last) game's mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// [`Mode::label`] of the current mode, with the computer's name.
    pub fn mode_label(&self) -> String {
        self.mode.label(self.computer_name())
    }

    /// [`Mode::labels`] of the current mode, longest first, for a title that must fit.
    pub fn mode_labels(&self) -> Vec<String> {
        let short = if self.uses_jev {
            JEV_NAME
        } else {
            LOCAL_SEARCH_SHORT_NAME
        };
        self.mode.labels(self.computer_name(), short)
    }

    /// The screen under the dialogs.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// [`Screen::name`] of the current screen.
    pub fn screen_name(&self) -> &'static str {
        self.screen.name()
    }

    /// The top dialog.
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialogs.last()
    }

    /// [`Dialog::name`] of the top dialog.
    pub fn dialog_name(&self) -> Option<&'static str> {
        self.dialogs.last().map(Dialog::name)
    }

    /// The latest status message, if any.
    pub fn message(&self) -> Option<&Message> {
        self.message.as_ref()
    }

    /// The latest status message's text, or `""`.
    pub fn status_line(&self) -> &str {
        self.message.as_ref().map_or("", |m| m.text.as_str())
    }

    /// Whose turn it is, with `· Check`, or the result when the game is over:
    /// `White to move`, `Black to move (Jev) · Check`, `White to move (Local search)`,
    /// `Checkmate — Black wins (0-1)`.
    pub fn turn_text(&self) -> String {
        self.turn_line(true)
    }

    /// [`turn_text`](Self::turn_text) without the player in brackets, for a panel too
    /// narrow for it: `Black to move · Check`.
    pub fn turn_text_brief(&self) -> String {
        self.turn_line(false)
    }

    fn turn_line(&self, with_player: bool) -> String {
        if let Some(outcome) = self.game.outcome() {
            return format!("{} ({})", outcome_text(outcome), outcome.result());
        }
        let pos = self.game.position();
        let side = pos.side_to_move();
        let who = match self.mode {
            Mode::HumanVsJev { human } if with_player && human == side => " (you)".to_string(),
            Mode::HumanVsJev { .. } | Mode::JevVsJev if with_player => {
                format!(" ({})", self.computer_name())
            }
            _ => String::new(),
        };
        let check = if pos.is_check() { " · Check" } else { "" };
        format!("{side} to move{who}{check}")
    }

    /// True when Black is drawn at the bottom.
    pub fn flipped(&self) -> bool {
        self.flipped
    }

    /// The piece glyph set.
    pub fn glyphs(&self) -> GlyphSet {
        self.glyphs
    }

    /// The colours in use.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The command box text.
    pub fn command_text(&self) -> &str {
        self.command.text()
    }

    /// The command box editor (text and cursor).
    pub fn command_editor(&self) -> &LineEditor {
        &self.command
    }

    /// True when the command box is focused (it may still be covered by a dialog).
    pub fn command_focused(&self) -> bool {
        self.command_focused
    }

    /// True when typed keys go to the command box right now: it is focused, the playing
    /// screen is shown and no dialog is open. Draw the terminal cursor only then.
    pub fn command_has_focus(&self) -> bool {
        self.command_focused && self.screen == Screen::Playing && self.dialogs.is_empty()
    }

    /// The last move the computer played, for the Jev panel; `None` once it is undone.
    pub fn last_computer(&self) -> Option<&ComputerMove> {
        self.last_computer.as_ref().map(|(_, computer)| computer)
    }

    /// The piece picked up by click or Enter.
    pub fn selected(&self) -> Option<Square> {
        self.selected
    }

    /// The keyboard cursor (hidden until an arrow key or Enter is pressed on the board).
    pub fn cursor(&self) -> Option<Square> {
        self.cursor
    }

    /// Legal destinations of the selected piece, sorted.
    pub fn targets(&self) -> Vec<Square> {
        self.selected
            .map(|sq| self.targets_of(sq))
            .unwrap_or_default()
    }

    /// Board highlights for the current state.
    pub fn highlights(&self) -> Highlights {
        let pos = self.game.position();
        Highlights {
            last_move: self.game.moves().last().map(|m| (m.from(), m.to())),
            selected: self.selected,
            targets: self.targets(),
            cursor: self.cursor,
            check: pos.is_check().then(|| pos.king_square(pos.side_to_move())),
        }
    }

    /// The engine request generation; replies from older generations are discarded.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// True while an engine request for the current generation is in flight.
    pub fn is_thinking(&self) -> bool {
        self.pending.is_some()
    }

    /// Engine requests sent and not yet answered, including discarded ones still running.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// True when the engine should be asked for a move now but [`MAX_IN_FLIGHT`] earlier
    /// requests are still running.
    pub fn waiting_for_engine(&self, now: Instant) -> bool {
        self.pending.is_none() && self.engine_due(now) && self.in_flight >= MAX_IN_FLIGHT
    }

    /// True after the worker produced no move, until space retries or the position changes.
    pub fn engine_failed(&self) -> bool {
        self.engine_failed
    }

    /// Elapsed time and spinner frame while the engine is thinking.
    pub fn thinking(&self, now: Instant) -> Option<Thinking> {
        self.pending.map(|pending| {
            let elapsed = now.saturating_duration_since(pending.since);
            let frame = elapsed.as_millis() / SPINNER_FRAME_MS % SPINNER.len() as u128;
            Thinking {
                elapsed,
                spinner: SPINNER[frame as usize],
            }
        })
    }

    /// True when Jev vs Jev is paused.
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// True when the engine's answer arrived while paused and waits to be played.
    pub fn has_held_move(&self) -> bool {
        self.held.is_some()
    }

    /// The Jev vs Jev pause between moves.
    pub fn step_delay(&self) -> Duration {
        self.step_delay
    }

    /// The highlighted menu entry (index into [`MENU_ITEMS`]).
    pub fn menu_index(&self) -> usize {
        self.menu_index
    }

    /// The highlighted game-over button (index into [`GAME_OVER_BUTTONS`]).
    pub fn game_over_choice(&self) -> usize {
        self.game_over_choice
    }

    /// Move-list rows scrolled up from the latest (0 follows the latest move).
    pub fn move_scroll(&self) -> usize {
        self.move_scroll
    }

    /// PGN `White` and `Black` names: `You`, or the [`computer_name`](Self::computer_name)
    /// (`Jev` or `Local search`).
    pub fn player_names(&self) -> (&'static str, &'static str) {
        let computer = self.computer_name();
        match self.mode {
            Mode::HumanVsHuman => ("You", "You"),
            Mode::HumanVsJev { human: Side::White } => ("You", computer),
            Mode::HumanVsJev { human: Side::Black } => (computer, "You"),
            Mode::JevVsJev => (computer, computer),
        }
    }

    /// Areas recorded by the last [`render`](Self::render).
    pub fn hit_map(&self) -> &HitMap {
        &self.hits
    }

    /// True once the user has confirmed quitting.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    // ----- events -----

    /// Applies one event and returns the work the run loop must do.
    ///
    /// Key events count only when their kind is `Press`. While the last draw showed only
    /// [`TOO_SMALL`], keys, clicks and pastes are ignored (they would act on a screen nobody
    /// can see), except Ctrl+C, which then quits at once: its confirmation could not be seen
    /// either. `now` stamps engine requests and schedules Jev vs Jev steps; pass the time the
    /// event was collected.
    #[must_use = "the run loop must execute the returned actions"]
    pub fn handle(&mut self, event: AppEvent, now: Instant) -> Vec<Action> {
        match event {
            AppEvent::Term(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if !self.too_small {
                    self.on_key(key);
                } else if ctrl_char(&key) == Some('c') {
                    self.quit = true;
                }
            }
            AppEvent::Term(Event::Mouse(mouse)) if !self.too_small => self.on_mouse(mouse),
            AppEvent::Term(Event::Paste(text)) if !self.too_small => self.on_paste(&text),
            AppEvent::Term(_) | AppEvent::Tick => {}
            AppEvent::Engine(reply) => self.on_engine(reply, now),
        }
        self.release_held(now);
        self.engine_request(now)
            .map(Action::RequestEngine)
            .into_iter()
            .collect()
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::ALT {
            // In a text field an Alt chord is a readline habit (Alt+B, Alt+F, ...): splitting
            // it would clear the text with Esc and send the letter to the board as a hotkey.
            if self.typing() {
                return;
            }
            // Elsewhere it is most likely Esc typed quickly before the key: handle both.
            if let KeyCode::Char(c) = key.code {
                self.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                self.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
                return;
            }
        }
        // Raw mode delivers Ctrl+C as a key; it must work from every focus.
        if ctrl_char(&key) == Some('c') {
            if matches!(
                self.dialogs.last(),
                Some(Dialog::Confirm {
                    question: Question::Quit,
                    ..
                })
            ) {
                self.quit = true;
            } else {
                self.request_quit();
            }
            return;
        }
        if !self.dialogs.is_empty() {
            self.dialog_key(key);
            return;
        }
        if ctrl_char(&key) == Some('s') && self.screen != Screen::Menu {
            self.open_input(InputPurpose::Save(SaveKind::Pgn));
            return;
        }
        match self.screen {
            Screen::Menu => self.menu_key(key),
            Screen::GameOver => self.game_over_key(key),
            Screen::Playing if self.command_focused => self.command_key(key),
            Screen::Playing => self.board_key(key),
        }
    }

    fn menu_key(&mut self, key: KeyEvent) {
        let last = MENU_ITEMS.len() - 1;
        match key.code {
            KeyCode::Up => self.menu_index = self.menu_index.checked_sub(1).unwrap_or(last),
            KeyCode::Down => self.menu_index = (self.menu_index + 1) % MENU_ITEMS.len(),
            KeyCode::Home => self.menu_index = 0,
            KeyCode::End => self.menu_index = last,
            KeyCode::Enter => self.activate_menu(self.menu_index),
            _ => match typed_char(&key) {
                Some('k') => self.menu_index = self.menu_index.checked_sub(1).unwrap_or(last),
                Some('j') => self.menu_index = (self.menu_index + 1) % MENU_ITEMS.len(),
                Some('q') => self.quit = true,
                Some(c) => {
                    let index = c.to_digit(10).and_then(|d| (d as usize).checked_sub(1));
                    if let Some(index) = index.filter(|&i| i < MENU_ITEMS.len()) {
                        self.menu_index = index;
                        self.activate_menu(index);
                    }
                }
                None => {}
            },
        }
    }

    fn activate_menu(&mut self, index: usize) {
        match MENU_ITEMS[index] {
            MenuItem::HumanVsHuman => self.start(Mode::HumanVsHuman, Game::new()),
            MenuItem::HumanVsJev(pick) => {
                let human = match pick {
                    SidePick::White => Side::White,
                    SidePick::Black => Side::Black,
                    SidePick::Random => (self.pick_side)(),
                };
                self.start(Mode::HumanVsJev { human }, Game::new());
            }
            MenuItem::JevVsJev => self.start(Mode::JevVsJev, Game::new()),
            MenuItem::LoadFen => self.open_input(InputPurpose::LoadFen { from_menu: true }),
            MenuItem::Quit => self.quit = true,
        }
    }

    /// True when keys go to a text field: the command box or a text-input dialog.
    fn typing(&self) -> bool {
        match self.dialogs.last() {
            Some(top) => matches!(top, Dialog::Input { .. }),
            None => self.command_has_focus(),
        }
    }

    fn game_over_key(&mut self, key: KeyEvent) {
        let count = GAME_OVER_BUTTONS.len();
        match key.code {
            KeyCode::Left | KeyCode::BackTab => {
                self.game_over_choice = (self.game_over_choice + count - 1) % count;
            }
            KeyCode::Right | KeyCode::Tab => {
                self.game_over_choice = (self.game_over_choice + 1) % count;
            }
            KeyCode::Enter => self.game_over_button(GAME_OVER_BUTTONS[self.game_over_choice]),
            // Look at the final position; `u` and `n` still work from the board.
            KeyCode::Esc => self.screen = Screen::Playing,
            _ => match typed_char(&key) {
                Some('n') => self.game_over_button(Button::NewGame),
                Some('s') => self.game_over_button(Button::SavePgn),
                Some('m') => self.game_over_button(Button::Menu),
                Some('u') => self.undo(),
                Some('q') => self.request_quit(),
                _ => {}
            },
        }
    }

    fn game_over_button(&mut self, button: Button) {
        match button {
            Button::NewGame => self.ask_new_game(),
            Button::SavePgn => self.open_input(InputPurpose::Save(SaveKind::Pgn)),
            Button::Menu => self.ask_menu(),
            Button::Confirm | Button::Cancel => {}
        }
    }

    fn board_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.move_cursor(0, 1),
            KeyCode::Down => self.move_cursor(0, -1),
            KeyCode::Left => self.move_cursor(-1, 0),
            KeyCode::Right => self.move_cursor(1, 0),
            KeyCode::Enter => self.board_enter(),
            KeyCode::Esc => {
                if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.cursor = None;
                }
            }
            _ => {
                if let Some(c) = typed_char(&key) {
                    self.board_char(c);
                }
            }
        }
    }

    fn board_char(&mut self, c: char) {
        let watching = self.mode == Mode::JevVsJev;
        match c {
            'u' => self.undo(),
            'f' => self.flipped = !self.flipped,
            'n' => self.ask_new_game(),
            'g' => self.cycle_glyphs(),
            'm' => self.ask_menu(),
            '?' => self.dialogs.push(Dialog::Help),
            'q' => self.request_quit(),
            '/' => self.command_focused = true,
            ':' => {
                self.command_focused = true;
                self.command.clear();
                self.command.insert(':');
            }
            ' ' if self.engine_failed => {
                self.engine_failed = false;
                self.info(RETRYING);
            }
            // The Status panel's pace line shows the pause and the step delay.
            ' ' if watching => self.paused = !self.paused,
            '+' | '=' if watching => self.change_step_delay(1),
            '-' if watching => self.change_step_delay(-1),
            _ => {}
        }
    }

    fn command_key(&mut self, key: KeyEvent) {
        // No command or move starts with a space, so in an empty box space does what it
        // does on the board: retry a failed engine, or pause Jev vs Jev.
        if typed_char(&key) == Some(' ')
            && self.command.is_empty()
            && (self.engine_failed || self.mode == Mode::JevVsJev)
        {
            self.board_char(' ');
            return;
        }
        match key.code {
            KeyCode::Enter => self.submit_command(),
            KeyCode::Esc => {
                self.command.clear();
                self.command_focused = false;
            }
            _ => edit_key(&mut self.command, &key),
        }
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(top) = self.dialogs.last_mut() else {
            return;
        };
        match top {
            Dialog::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
                    || matches!(typed_char(&key), Some('q' | '?' | ' '))
                {
                    self.dialogs.pop();
                }
            }
            Dialog::Promotion { choice, .. } => {
                let count = PROMOTION_CHOICES.len();
                match key.code {
                    KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                        *choice = (*choice + count - 1) % count;
                    }
                    KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                        *choice = (*choice + 1) % count;
                    }
                    KeyCode::Enter => {
                        let kind = PROMOTION_CHOICES[*choice % count];
                        self.promote(kind);
                    }
                    KeyCode::Esc => {
                        self.dialogs.pop();
                    }
                    _ => {
                        let kind = typed_char(&key)
                            .and_then(PieceKind::from_char)
                            .filter(|kind| PROMOTION_CHOICES.contains(kind));
                        if let Some(kind) = kind {
                            self.promote(kind);
                        }
                    }
                }
            }
            Dialog::Input { editor, .. } => match key.code {
                KeyCode::Enter => self.submit_input(),
                KeyCode::Esc => {
                    self.dialogs.pop();
                }
                _ => edit_key(editor, &key),
            },
            Dialog::Confirm { yes, .. } => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => *yes = !*yes,
                KeyCode::Enter => {
                    let answer = *yes;
                    self.answer(answer);
                }
                KeyCode::Esc => self.answer(false),
                _ => match typed_char(&key).map(|c| c.to_ascii_lowercase()) {
                    Some('y') => self.answer(true),
                    Some('n') => self.answer(false),
                    _ => {}
                },
            },
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        let (column, row) = (mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(column, row),
            MouseEventKind::Up(MouseButton::Left) => {
                if self.dialogs.is_empty() && self.screen == Screen::Playing {
                    self.board_release(column, row);
                } else {
                    self.drag = None;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if self.hits.at(column, row) == Some(Hit::MoveList) =>
            {
                self.scroll_moves(mouse.kind == MouseEventKind::ScrollUp);
            }
            _ => {}
        }
    }

    fn mouse_down(&mut self, column: u16, row: u16) {
        let hit = self.hits.at(column, row);
        if let Some(top) = self.dialogs.last() {
            let is_help = matches!(top, Dialog::Help);
            let is_promotion = matches!(top, Dialog::Promotion { .. });
            match hit {
                _ if is_help => {
                    self.dialogs.pop();
                }
                Some(Hit::Promote(kind)) if is_promotion => self.promote(kind),
                Some(Hit::Button(button)) => self.dialog_button(button),
                _ => {}
            }
            return;
        }
        match self.screen {
            Screen::Menu => {
                if let Some(Hit::MenuItem(index)) = hit
                    && index < MENU_ITEMS.len()
                {
                    self.menu_index = index;
                    self.activate_menu(index);
                }
            }
            Screen::GameOver => {
                if let Some(Hit::Button(button)) = hit {
                    self.game_over_button(button);
                }
            }
            Screen::Playing => {
                if hit == Some(Hit::CommandBox) {
                    self.command_focused = true;
                } else {
                    // Clicking elsewhere leaves the command box but keeps its text.
                    self.command_focused = false;
                    self.board_press(column, row);
                }
            }
        }
    }

    fn dialog_button(&mut self, button: Button) {
        match (self.dialogs.last(), button) {
            (Some(Dialog::Confirm { .. }), Button::Confirm) => self.answer(true),
            (Some(Dialog::Confirm { .. }), Button::Cancel) => self.answer(false),
            (Some(Dialog::Input { .. }), Button::Confirm) => self.submit_input(),
            (Some(_), Button::Cancel) => {
                self.dialogs.pop();
            }
            _ => {}
        }
    }

    fn on_paste(&mut self, text: &str) {
        if let Some(top) = self.dialogs.last_mut() {
            if let Dialog::Input { editor, .. } = top
                && editor.insert_str(text)
            {
                self.submit_input();
            }
            return;
        }
        if self.screen != Screen::Playing {
            return;
        }
        // Pasting on the board types into the command box; a line break submits it.
        self.command_focused = true;
        if self.command.insert_str(text) {
            self.submit_command();
        }
    }

    fn on_engine(&mut self, reply: EngineReply, now: Instant) {
        // Every request is answered exactly once, whether or not it is still wanted.
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.pending.is_none() {
            log::debug!(
                "discarding engine reply for generation {}: no request pending",
                reply.generation
            );
            return;
        }
        if !is_current(&reply, self.generation, self.game.position().hash()) {
            log::debug!(
                "discarding stale engine reply (generation {} vs {})",
                reply.generation,
                self.generation
            );
            return;
        }
        self.pending = None;
        if self.paused {
            // Paused while the engine was thinking: nothing is played until space resumes.
            self.held = Some(reply.outcome);
            return;
        }
        self.apply_outcome(reply.outcome, now);
    }

    /// Plays a move the worker produced, or records why there is none.
    fn apply_outcome(&mut self, outcome: EngineOutcome, now: Instant) {
        match outcome {
            EngineOutcome::Move(computer) => {
                if let Err(error) = self.game.play(computer.mv) {
                    self.engine_failure(&format!(
                        "engine returned an illegal move {}: {error}",
                        computer.mv
                    ));
                    return;
                }
                let recovered = computer.note.as_deref() == Some(ENGINE_ERROR_NOTE);
                self.last_computer = Some((self.game.moves().len(), computer));
                self.message = None;
                self.after_engine_move(now);
                if recovered {
                    self.error(ENGINE_ERROR_NOTE);
                }
            }
            EngineOutcome::GameOver if self.game.outcome().is_some() => self.check_game_over(),
            EngineOutcome::GameOver => {
                self.engine_failure("engine found no move in a game that is not over");
            }
            EngineOutcome::Failed(reason) => self.engine_failure(&reason),
        }
    }

    /// Stops asking the engine until space retries or the position changes.
    fn engine_failure(&mut self, reason: &str) {
        log::warn!("{reason}");
        self.engine_failed = true;
        self.error(ENGINE_FAILED);
    }

    /// Plays the answer held while paused, once play has resumed.
    fn release_held(&mut self, now: Instant) {
        if !self.paused
            && let Some(outcome) = self.held.take()
        {
            self.apply_outcome(outcome, now);
        }
    }

    /// True when it is the engine's turn, nothing stops it, and in Jev vs Jev the step
    /// delay since the last move has passed.
    fn engine_due(&self, now: Instant) -> bool {
        let turn = self.screen != Screen::Menu
            && self.game.outcome().is_none()
            && self.mode.engine_plays(self.game.position().side_to_move())
            && !self.paused
            && !self.engine_failed
            && self.held.is_none();
        let step_done = self.mode != Mode::JevVsJev
            || self.step_anchor.is_none_or(|anchor| {
                anchor
                    .checked_add(self.step_delay)
                    .is_none_or(|due| now >= due)
            });
        turn && step_done
    }

    /// The request to send now: the engine is due, nothing is pending for this position,
    /// and fewer than [`MAX_IN_FLIGHT`] requests are out.
    fn engine_request(&mut self, now: Instant) -> Option<EngineRequest> {
        if self.pending.is_some() || self.in_flight >= MAX_IN_FLIGHT || !self.engine_due(now) {
            return None;
        }
        self.pending = Some(Pending { since: now });
        self.in_flight += 1;
        Some(EngineRequest::new(self.generation, self.game.clone()))
    }

    // ----- game actions -----

    /// Starts `game` in `mode` on the playing screen.
    fn start(&mut self, mode: Mode, game: Game) {
        self.mode = mode;
        self.game = game;
        self.flipped = matches!(mode, Mode::HumanVsJev { human: Side::Black });
        self.screen = Screen::Playing;
        self.dialogs.clear();
        self.message = None;
        self.last_computer = None;
        self.paused = false;
        self.step_anchor = None;
        self.cursor = None;
        self.invalidate();
        self.check_game_over();
    }

    /// True when leaving would throw away moves of an unfinished game.
    fn game_in_progress(&self) -> bool {
        self.screen != Screen::Menu
            && !self.game.moves().is_empty()
            && self.game.outcome().is_none()
    }

    /// A new game, after asking when one is in progress.
    fn ask_new_game(&mut self) {
        if self.game_in_progress() {
            self.confirm(Question::NewGame);
        } else {
            self.new_game();
        }
    }

    /// The menu, after asking when a game is in progress.
    fn ask_menu(&mut self) {
        if self.game_in_progress() {
            self.confirm(Question::Menu);
        } else {
            self.go_to_menu();
        }
    }

    /// Opens a yes/no dialog for `question`, starting on No.
    fn confirm(&mut self, question: Question) {
        self.dialogs.push(Dialog::Confirm {
            question,
            yes: false,
        });
    }

    fn new_game(&mut self) {
        if self.screen == Screen::Menu {
            return;
        }
        self.start(self.mode, Game::new());
        self.info("new game");
    }

    fn go_to_menu(&mut self) {
        self.screen = Screen::Menu;
        self.dialogs.clear();
        self.command.clear();
        self.command_focused = false;
        self.cursor = None;
        self.message = None;
        self.invalidate();
    }

    /// Forgets everything tied to the old position: in-flight requests (by bumping the
    /// generation), a held answer, an engine failure, the selection, the move-list scroll,
    /// and the Jev panel's move once it has been taken back.
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending = None;
        self.held = None;
        self.engine_failed = false;
        self.selected = None;
        self.drag = None;
        self.move_scroll = 0;
        let plies = self.game.moves().len();
        self.last_computer.take_if(|(ply, _)| *ply > plies);
    }

    /// Shows the game-over overlay when the game has ended, and leaves it when it has not.
    fn check_game_over(&mut self) {
        if self.screen == Screen::Menu {
            return;
        }
        if self.game.outcome().is_some() {
            self.screen = Screen::GameOver;
            self.game_over_choice = 0;
            self.selected = None;
            self.drag = None;
        } else if self.screen == Screen::GameOver {
            self.screen = Screen::Playing;
        }
    }

    /// Why the person may not move now.
    fn can_human_move(&self) -> Result<(), String> {
        if self.game.outcome().is_some() {
            return Err(GAME_IS_OVER.to_string());
        }
        let computer = self.computer_name();
        match self.mode {
            Mode::JevVsJev => Err(format!("{computer} plays both sides; you are watching")),
            Mode::HumanVsJev { human } if self.game.position().side_to_move() != human => {
                // Short enough for one row of the narrowest Status panel.
                Err(format!("{computer} to move"))
            }
            _ => Ok(()),
        }
    }

    fn play_human(&mut self, mv: Move) -> Result<(), String> {
        self.can_human_move()?;
        self.game
            .play(mv)
            .map_err(|_| format!("not a legal move: {mv}"))?;
        self.selected = None;
        self.drag = None;
        self.move_scroll = 0;
        self.message = None;
        self.check_game_over();
        Ok(())
    }

    /// Plays typed move text; a promotion typed without its piece opens the picker.
    fn play_text(&mut self, text: &str) -> Result<(), String> {
        self.can_human_move()?;
        match parse_move(self.game.position(), text) {
            Ok(mv) => self.play_human(mv),
            Err(MoveTextError::NeedsPromotion { from, to }) => {
                self.dialogs.push(Dialog::Promotion {
                    from,
                    to,
                    choice: 0,
                });
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn after_engine_move(&mut self, now: Instant) {
        self.selected = None;
        self.drag = None;
        self.move_scroll = 0;
        if self.mode == Mode::JevVsJev {
            self.step_anchor = Some(now);
        }
        self.check_game_over();
    }

    /// Takes back one ply, or two against Jev so it is the person's turn again (one when
    /// Jev has not replied yet). A pending resignation is withdrawn first. Jev vs Jev
    /// takes back one ply and pauses.
    fn undo(&mut self) {
        if self.screen == Screen::Menu {
            return;
        }
        if matches!(self.game.outcome(), Some(Outcome::Resignation { .. })) {
            self.game.undo();
            self.invalidate();
            self.screen = Screen::Playing;
            self.info("resignation withdrawn");
            return;
        }
        let plies = match self.mode {
            Mode::HumanVsJev { human } if self.game.position().side_to_move() == human => 2,
            _ => 1,
        };
        if self.game.moves().len() < plies {
            self.error(NOTHING_TO_UNDO);
            return;
        }
        for _ in 0..plies {
            self.game.undo();
        }
        self.invalidate();
        self.screen = Screen::Playing;
        let taken = if plies == 1 {
            "took back 1 move"
        } else {
            "took back 2 moves"
        };
        if self.mode == Mode::JevVsJev {
            // The pace line in the Status panel says so.
            self.paused = true;
        }
        self.info(taken);
    }

    fn cycle_glyphs(&mut self) {
        self.glyphs = self.glyphs.next();
        self.info(format!("glyphs: {}", self.glyphs));
    }

    fn change_step_delay(&mut self, direction: i8) {
        let current = STEP_DELAYS
            .iter()
            .position(|&d| d >= self.step_delay)
            .unwrap_or(STEP_DELAYS.len() - 1);
        let index = if direction > 0 {
            (current + 1).min(STEP_DELAYS.len() - 1)
        } else {
            current.saturating_sub(1)
        };
        // The Status panel's pace line shows the new delay.
        self.step_delay = STEP_DELAYS[index];
    }

    fn request_quit(&mut self) {
        if self.screen == Screen::Playing && self.game.outcome().is_none() {
            self.confirm(Question::Quit);
        } else {
            self.quit = true;
        }
    }

    fn open_resign(&mut self) {
        if self.mode == Mode::JevVsJev {
            self.error(NOTHING_TO_RESIGN);
        } else if self.game.outcome().is_some() {
            self.error(GAME_IS_OVER);
        } else {
            self.confirm(Question::Resign);
        }
    }

    /// The person resigns: their side against Jev, the side to move otherwise.
    fn resign(&mut self) {
        let loser = match self.mode {
            Mode::HumanVsJev { human } => human,
            Mode::HumanVsHuman | Mode::JevVsJev => self.game.position().side_to_move(),
        };
        self.game.resign(loser);
        self.invalidate();
        self.check_game_over();
    }

    fn answer(&mut self, yes: bool) {
        if !matches!(self.dialogs.last(), Some(Dialog::Confirm { .. })) {
            return;
        }
        let Some(Dialog::Confirm { question, .. }) = self.dialogs.pop() else {
            return;
        };
        match question {
            Question::Quit if yes => self.quit = true,
            Question::Resign if yes => self.resign(),
            Question::NewGame if yes => self.new_game(),
            Question::Menu if yes => self.go_to_menu(),
            Question::Overwrite { path, contents, .. } if yes => {
                match write_file(&path, &contents, true) {
                    Ok(()) => self.info(format!("saved {}", path.display())),
                    Err(error) => self.error(error.to_string()),
                }
            }
            Question::Overwrite {
                kind,
                typed,
                origin,
                ..
            } => {
                self.info(NOT_SAVED);
                self.retype_save(kind, &typed, origin);
            }
            Question::Quit | Question::Resign | Question::NewGame | Question::Menu => {}
        }
    }

    /// Gives a declined save's path back where it was typed, ready to edit.
    fn retype_save(&mut self, kind: SaveKind, typed: &str, origin: SaveOrigin) {
        match origin {
            SaveOrigin::Dialog => {
                let mut editor = LineEditor::new();
                editor.insert_str(typed);
                self.dialogs.push(Dialog::Input {
                    purpose: InputPurpose::Save(kind),
                    editor,
                    error: None,
                });
            }
            SaveOrigin::Command => {
                self.command.clear();
                self.command
                    .insert_str(&format!(":save{} {typed}", kind.extension()));
                self.command_focused = true;
            }
        }
    }

    // ----- board input -----

    fn targets_of(&self, from: Square) -> Vec<Square> {
        let mut targets: Vec<Square> = self
            .game
            .position()
            .legal_moves()
            .iter()
            .filter(|m| m.from() == from)
            .map(|m| m.to())
            .collect();
        targets.sort();
        targets.dedup();
        targets
    }

    /// Picks up the piece on `sq` if the person may move it; otherwise drops the selection.
    fn select(&mut self, sq: Square) -> bool {
        let Some(piece) = self.game.position().piece_at(sq) else {
            self.selected = None;
            return false;
        };
        if let Err(reason) = self.can_human_move() {
            self.selected = None;
            self.error(reason);
            return false;
        }
        if piece.color != self.game.position().side_to_move() {
            self.selected = None;
            return false;
        }
        self.selected = Some(sq);
        true
    }

    /// Plays `from`-`to`, asking for the piece first when it is a promotion.
    fn try_move(&mut self, from: Square, to: Square) {
        let moves: Vec<Move> = self
            .game
            .position()
            .legal_moves()
            .iter()
            .copied()
            .filter(|m| m.from() == from && m.to() == to)
            .collect();
        match moves.as_slice() {
            [] => self.selected = None,
            &[mv] => {
                if let Err(reason) = self.play_human(mv) {
                    self.error(reason);
                }
            }
            _ => self.dialogs.push(Dialog::Promotion {
                from,
                to,
                choice: 0,
            }),
        }
    }

    fn promote(&mut self, kind: PieceKind) {
        if !matches!(self.dialogs.last(), Some(Dialog::Promotion { .. })) {
            return;
        }
        let Some(Dialog::Promotion { from, to, .. }) = self.dialogs.pop() else {
            return;
        };
        let mv = self
            .game
            .position()
            .legal_moves()
            .iter()
            .copied()
            .find(|m| m.from() == from && m.to() == to && m.promotion() == Some(kind));
        let result = match mv {
            Some(mv) => self.play_human(mv),
            None => Err(format!("not a legal move: {from}{to}{}", kind.to_char())),
        };
        if let Err(reason) = result {
            self.error(reason);
        }
    }

    /// Moves the keyboard cursor `dx` columns right and `dy` rows up on screen. The first
    /// press only shows it: on the selected piece, else in front of the bottom king.
    fn move_cursor(&mut self, dx: i8, dy: i8) {
        let Some(current) = self.cursor else {
            self.show_cursor();
            return;
        };
        let (df, dr) = if self.flipped { (-dx, -dy) } else { (dx, dy) };
        let file = current.file().saturating_add_signed(df).min(7);
        let rank = current.rank().saturating_add_signed(dr).min(7);
        self.cursor = Square::from_file_rank(file, rank).or(self.cursor);
    }

    fn show_cursor(&mut self) {
        // In front of the bottom king: e2, or e7 when Black is at the bottom.
        let front = if self.flipped {
            Square::from_file_rank(4, 6)
        } else {
            Square::from_file_rank(4, 1)
        };
        self.cursor = self.selected.or(front);
    }

    fn board_enter(&mut self) {
        let Some(sq) = self.cursor else {
            self.show_cursor();
            return;
        };
        match self.selected {
            Some(from) if from == sq => self.selected = None,
            Some(from) if self.targets_of(from).contains(&sq) => self.try_move(from, sq),
            _ => {
                self.select(sq);
            }
        }
    }

    fn board_press(&mut self, column: u16, row: u16) {
        let Some(sq) = self.hits.board.and_then(|g| square_at(&g, column, row)) else {
            self.selected = None;
            return;
        };
        // The mouse is in use: hide the keyboard cursor.
        self.cursor = None;
        if let Some(from) = self.selected {
            if sq == from {
                self.drag = Some(Drag {
                    from,
                    release_deselects: true,
                });
                return;
            }
            if self.targets_of(from).contains(&sq) {
                self.try_move(from, sq);
                return;
            }
        }
        if self.select(sq) {
            self.drag = Some(Drag {
                from: sq,
                release_deselects: false,
            });
        }
    }

    fn board_release(&mut self, column: u16, row: u16) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let Some(sq) = self.hits.board.and_then(|g| square_at(&g, column, row)) else {
            return;
        };
        if sq == drag.from {
            if drag.release_deselects {
                self.selected = None;
            }
        } else if self.selected == Some(drag.from) && self.targets_of(drag.from).contains(&sq) {
            self.try_move(drag.from, sq);
        }
    }

    fn scroll_moves(&mut self, up: bool) {
        if self.screen == Screen::Menu || !self.dialogs.is_empty() {
            return;
        }
        let max = move_rows(&self.game).len().saturating_sub(1);
        self.move_scroll = if up {
            (self.move_scroll + 1).min(max)
        } else {
            self.move_scroll.saturating_sub(1)
        };
    }

    // ----- command box and text dialogs -----

    fn submit_command(&mut self) {
        let line = self.command.take();
        if let Some(purpose) = bare_command_dialog(&line) {
            self.open_input(purpose);
            return;
        }
        let result = parse_command(&line).and_then(|command| self.run_command(command));
        if let Err(reason) = result {
            // Keep the text so a typo can be fixed.
            self.command.insert_str(&line);
            self.error(reason);
        }
    }

    /// Runs a submitted command. Errors worth editing the text for are returned; other
    /// outcomes set their own message.
    fn run_command(&mut self, command: Command) -> Result<(), String> {
        match command {
            Command::Move(text) => self.play_text(&text)?,
            Command::Undo => self.undo(),
            Command::Flip => self.flipped = !self.flipped,
            Command::New => self.ask_new_game(),
            Command::Fen(fen) => self.load_fen(&fen, self.mode)?,
            Command::SaveFen(path) => self.save(SaveKind::Fen, &path, SaveOrigin::Command)?,
            Command::SavePgn(path) => self.save(SaveKind::Pgn, &path, SaveOrigin::Command)?,
            Command::Resign => self.open_resign(),
            Command::Glyphs => self.cycle_glyphs(),
            Command::Help => self.dialogs.push(Dialog::Help),
            Command::Quit => self.request_quit(),
        }
        Ok(())
    }

    fn open_input(&mut self, purpose: InputPurpose) {
        self.dialogs.push(Dialog::Input {
            purpose,
            editor: LineEditor::new(),
            error: None,
        });
    }

    fn submit_input(&mut self) {
        if !matches!(self.dialogs.last(), Some(Dialog::Input { .. })) {
            return;
        }
        let Some(Dialog::Input {
            purpose, editor, ..
        }) = self.dialogs.pop()
        else {
            return;
        };
        let result = match purpose {
            InputPurpose::LoadFen { from_menu } => {
                let mode = if from_menu {
                    Mode::HumanVsHuman
                } else {
                    self.mode
                };
                self.load_fen(editor.text(), mode)
            }
            InputPurpose::Save(kind) => self.save(kind, editor.text(), SaveOrigin::Dialog),
        };
        if let Err(error) = result {
            self.dialogs.push(Dialog::Input {
                purpose,
                editor,
                error: Some(error),
            });
        }
    }

    fn load_fen(&mut self, fen: &str, mode: Mode) -> Result<(), String> {
        let fen = fen.trim();
        if fen.is_empty() {
            return Err("type a FEN".to_string());
        }
        let game = Game::from_fen(fen).map_err(|e| fen_error(&e))?;
        self.start(mode, game);
        self.info("position loaded");
        Ok(())
    }

    /// Saves to the typed path, asking before replacing an existing file.
    fn save(&mut self, kind: SaveKind, raw: &str, origin: SaveOrigin) -> Result<(), String> {
        let path = resolve_path(raw, kind.extension(), self.home.as_deref())?;
        let contents = self.export(kind);
        match write_file(&path, &contents, false) {
            Ok(()) => {
                self.info(format!("saved {}", path.display()));
                Ok(())
            }
            Err(SaveError::Exists) => {
                self.confirm(Question::Overwrite {
                    kind,
                    path,
                    contents,
                    typed: raw.trim().to_string(),
                    origin,
                });
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn export(&self, kind: SaveKind) -> String {
        match kind {
            SaveKind::Pgn => {
                let (white, black) = self.player_names();
                pgn_export(&self.game, white, black, &(self.today)())
            }
            SaveKind::Fen => format!("{}\n", self.game.position().to_fen()),
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            is_error: false,
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            is_error: true,
        });
    }

    // ----- rendering -----

    /// Draws the current state (see [`panels::draw`]) and records the [`HitMap`] used by
    /// the next mouse event. Also clamps the move-list scroll to what the list can show,
    /// and notes whether only [`TOO_SMALL`] fitted (input is ignored until more does).
    pub fn render(&mut self, frame: &mut Frame, now: Instant) {
        self.too_small = is_too_small(frame.area());
        let drawn = panels::draw(self, frame, now);
        self.hits = drawn.hits;
        self.move_scroll = drawn.move_scroll;
    }
}

/// How a game ended, in words: `Checkmate — Black wins`, `White resigned — Black wins`,
/// `Stalemate — draw`, ...
pub fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Checkmate { winner } => format!("Checkmate — {winner} wins"),
        Outcome::Resignation { winner } => format!("{} resigned — {winner} wins", !winner),
        Outcome::Stalemate => "Stalemate — draw".to_string(),
        Outcome::FiftyMoveRule => "Fifty-move rule — draw".to_string(),
        Outcome::ThreefoldRepetition => "Threefold repetition — draw".to_string(),
        Outcome::InsufficientMaterial => "Insufficient material — draw".to_string(),
    }
}

/// Move-list rows in SAN: `1. e4 e5`, `2. Nf3`; a game from a FEN with Black to move
/// starts `12... Kd7`.
pub fn move_rows(game: &Game) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for (index, (&mv, pos)) in game.moves().iter().zip(game.positions()).enumerate() {
        let san = pos.to_san(mv);
        match pos.side_to_move() {
            Side::White => rows.push(format!("{}. {san}", pos.fullmove_number())),
            Side::Black if index == 0 => rows.push(format!("{}... {san}", pos.fullmove_number())),
            Side::Black => {
                if let Some(row) = rows.last_mut() {
                    row.push(' ');
                    row.push_str(&san);
                }
            }
        }
    }
    rows
}

/// A plain typed character: no modifier but Shift. Ctrl+Alt also counts as plain, because
/// that is how the `AltGr` key arrives on some platforms.
fn typed_char(key: &KeyEvent) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    let mods = key.modifiers.difference(KeyModifiers::SHIFT);
    (mods.is_empty() || mods == KeyModifiers::CONTROL | KeyModifiers::ALT).then_some(c)
}

/// The letter of a Ctrl+letter chord, lower-cased.
fn ctrl_char(key: &KeyEvent) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    (key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::CONTROL)
        .then(|| c.to_ascii_lowercase())
}

/// Applies an editing key to `editor`: characters, ←/→, Home/End, Backspace/Delete, and
/// the readline chords Ctrl+A/E (home/end), Ctrl+H (backspace) and Ctrl+U (clear). Other
/// keys are ignored.
fn edit_key(editor: &mut LineEditor, key: &KeyEvent) {
    match ctrl_char(key) {
        Some('a') => editor.home(),
        Some('e') => editor.end(),
        Some('h') => editor.backspace(),
        Some('u') => editor.clear(),
        Some(_) => {}
        None => match key.code {
            KeyCode::Backspace => editor.backspace(),
            KeyCode::Delete => editor.delete(),
            KeyCode::Left => editor.left(),
            KeyCode::Right => editor.right(),
            KeyCode::Home => editor.home(),
            KeyCode::End => editor.end(),
            _ => {
                if let Some(c) = typed_char(key) {
                    editor.insert(c);
                }
            }
        },
    }
}

/// The dialog a bare `:fen`, `:savefen` or `:savepgn` opens: typed without an argument,
/// these ask for it in a wider field instead of failing with a usage message.
fn bare_command_dialog(line: &str) -> Option<InputPurpose> {
    let name = line.trim().strip_prefix(':')?.trim();
    if name.eq_ignore_ascii_case("fen") {
        Some(InputPurpose::LoadFen { from_menu: false })
    } else if name.eq_ignore_ascii_case("savefen") {
        Some(InputPurpose::Save(SaveKind::Fen))
    } else if name.eq_ignore_ascii_case("savepgn") {
        Some(InputPurpose::Save(SaveKind::Pgn))
    } else {
        None
    }
}

/// `invalid FEN: <reason>`, without the FEN text core appends to the reason.
fn fen_error(error: &ChessError) -> String {
    match error {
        ChessError::InvalidFen(detail) => {
            // core formats the detail as `<reason> in "<fen>"`; the reason never contains
            // a quote, so the first ` in "` starts the echoed FEN.
            let reason = detail
                .split_once(" in \"")
                .map_or(detail.as_str(), |(r, _)| r);
            format!("invalid FEN: {reason}")
        }
        other => other.to_string(),
    }
}

/// A coin flip without a random-number dependency: std's `RandomState` is seeded from
/// the OS once per thread and stepped for each new instance.
fn random_side() -> Side {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    if bits & 1 == 0 {
        Side::White
    } else {
        Side::Black
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;

    use super::*;
    use crate::core::{Position as ChessPosition, START_FEN};
    use crate::engine::{MoveSource, analyse};
    use crate::tui::panels::HELP_LINES;
    use crate::tui::test_support::engine::{FakeEngine, REPLY_TIMEOUT, chord, key, mouse, paste};
    use crate::tui::test_support::harness::{Harness, request};
    use crate::tui::test_support::{PROMOTION_FEN, TempDir, game_from, sq};
    use crate::tui::worker::{LOCAL_SEARCH_STATUS, spawn_request};

    const FOOLS_MATE: [&str; 4] = ["f2f3", "e7e5", "g2g4", "d8h4"];
    /// Knights on b1 and f3 can both reach d2.
    const TWO_KNIGHTS_FEN: &str = "4k3/8/8/8/8/5N2/8/1N2K3 w - - 0 1";

    /// A Human vs Human game at the start position.
    fn hvh() -> Harness {
        let mut harness = Harness::new();
        harness.char('1');
        assert_eq!(harness.app.screen(), Screen::Playing);
        harness
    }

    // ----- menu -----

    #[test]
    fn menu_enter_starts_human_vs_human() {
        let mut h = Harness::new();
        assert_eq!(h.app.screen_name(), "menu");
        assert!(h.screen().contains("1. Human vs Human"));
        assert!(h.press(KeyCode::Enter).is_empty());
        assert_eq!(h.app.screen_name(), "playing");
        assert_eq!(h.app.mode(), Mode::HumanVsHuman);
        assert!(!h.app.flipped());
        assert_eq!(*h.app.game().position(), ChessPosition::startpos());
        assert!(h.app.hit_map().board.is_some());
    }

    #[test]
    fn menu_keys_wrap_and_digits_start_items() {
        let mut h = Harness::new();
        h.press(KeyCode::Up);
        assert_eq!(h.app.menu_index(), MENU_ITEMS.len() - 1);
        h.press(KeyCode::Down);
        assert_eq!(h.app.menu_index(), 0);
        h.char('j');
        h.char('j');
        h.char('k');
        assert_eq!(h.app.menu_index(), 1);
        h.press(KeyCode::End);
        assert_eq!(h.app.menu_index(), MENU_ITEMS.len() - 1);
        h.press(KeyCode::Home);
        assert_eq!(h.app.menu_index(), 0);
        h.char('9');
        h.char('0');
        assert_eq!(h.app.screen_name(), "menu", "no such items");
        assert!(h.char('2').is_empty(), "White moves first, so Jev waits");
        assert_eq!(h.app.mode(), Mode::HumanVsJev { human: Side::White });
        assert_eq!(h.app.menu_index(), 1);
        assert!(!h.app.flipped());
    }

    #[test]
    fn human_vs_jev_as_black_starts_flipped_and_asks_the_engine() {
        let mut h = Harness::new();
        let request = request(&h.char('3'));
        assert_eq!(h.app.mode(), Mode::HumanVsJev { human: Side::Black });
        assert!(h.app.flipped());
        assert_eq!(request.generation, h.app.generation());
        assert_eq!(request.hash, ChessPosition::startpos().hash());
        assert!(request.game.moves().is_empty());
        assert!(h.app.is_thinking());
        assert!(h.tick().is_empty(), "one request at a time");
    }

    #[test]
    fn random_side_uses_the_injected_picker() {
        let mut h = Harness::build(FakeEngine::local(), (80, 24), Vec::new(), |app| {
            app.with_side_picker(|| Side::Black)
        });
        assert_eq!(h.char('4').len(), 1);
        assert_eq!(h.app.mode(), Mode::HumanVsJev { human: Side::Black });
        assert!(h.app.flipped());

        let mut h = Harness::build(FakeEngine::local(), (80, 24), Vec::new(), |app| {
            app.with_side_picker(|| Side::White)
        });
        assert!(h.char('4').is_empty());
        assert_eq!(h.app.mode(), Mode::HumanVsJev { human: Side::White });
        assert!(!h.app.flipped());
    }

    #[test]
    fn menu_click_starts_jev_vs_jev() {
        let mut h = Harness::new();
        let actions = h.click_hit(Hit::MenuItem(4));
        assert_eq!(h.app.mode(), Mode::JevVsJev);
        assert_eq!(h.app.menu_index(), 4);
        assert_eq!(request(&actions).hash, ChessPosition::startpos().hash());
    }

    #[test]
    fn menu_shows_engine_status_and_warnings_once() {
        let engine = FakeEngine::local().with_warnings(&["JEV_TIMEOUT_MS ignored"]);
        let warnings = vec![
            "--glyphs: unknown glyph set".to_string(),
            "JEV_TIMEOUT_MS ignored".to_string(),
        ];
        let h = Harness::build(engine, (80, 24), warnings, |app| app);
        assert_eq!(
            h.app.warnings(),
            ["JEV_TIMEOUT_MS ignored", "--glyphs: unknown glyph set"]
        );
        assert_eq!(h.app.engine_status(), LOCAL_SEARCH_STATUS);
        let screen = h.screen();
        assert!(screen.contains(LOCAL_SEARCH_STATUS));
        assert!(screen.contains("! JEV_TIMEOUT_MS ignored"));
        assert!(screen.contains("! --glyphs: unknown glyph set"));
    }

    #[test]
    fn menu_load_fen_rejects_bad_text_without_echoing_it() {
        let mut h = Harness::new();
        h.char('6');
        assert_eq!(h.app.dialog_name(), Some("load fen"));
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { error: Some(e), .. }) if e == "type a FEN"
        ));
        h.type_text("hello world");
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { error: Some(e), .. }) if e == "invalid FEN: expected 4 to 6 fields"
        ));
        assert!(h.screen().contains("invalid FEN: expected 4 to 6 fields"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.screen_name(), "menu");

        h.char('6');
        h.type_text(PROMOTION_FEN);
        h.click_hit(Hit::Button(Button::Confirm));
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.screen_name(), "playing");
        assert_eq!(h.app.mode(), Mode::HumanVsHuman);
        assert_eq!(h.app.game().position().to_fen(), PROMOTION_FEN);
        assert_eq!(h.app.status_line(), "position loaded");
    }

    #[test]
    fn menu_quit_needs_no_confirmation() {
        let mut h = Harness::new();
        h.char('q');
        assert!(h.app.should_quit());
        let mut h = Harness::new();
        h.click_hit(Hit::MenuItem(6));
        assert!(h.app.should_quit());
    }

    // ----- board input -----

    #[test]
    fn keyboard_cursor_selects_and_moves() {
        let mut h = hvh();
        assert_eq!(h.app.cursor(), None);
        h.press(KeyCode::Up);
        assert_eq!(
            h.app.cursor(),
            Some(sq("e2")),
            "the first press only shows it"
        );
        h.press(KeyCode::Enter);
        assert_eq!(h.app.selected(), Some(sq("e2")));
        assert_eq!(h.app.targets(), [sq("e3"), sq("e4")]);
        h.press(KeyCode::Up);
        h.press(KeyCode::Up);
        assert_eq!(h.app.cursor(), Some(sq("e4")));
        h.press(KeyCode::Enter);
        assert_eq!(h.uci(), ["e2e4"]);
        assert_eq!(h.app.selected(), None);
        assert_eq!(h.app.highlights().last_move, Some((sq("e2"), sq("e4"))));

        // Enter on an empty square or an opponent piece selects nothing.
        h.press(KeyCode::Down);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.selected(), None);
        // Enter on the selected piece puts it down again; Esc drops it, then hides the cursor.
        for _ in 0..4 {
            h.press(KeyCode::Up);
        }
        assert_eq!(h.app.cursor(), Some(sq("e7")));
        h.press(KeyCode::Enter);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.selected(), None);
        h.press(KeyCode::Enter);
        h.press(KeyCode::Esc);
        assert_eq!(h.app.selected(), None);
        assert_eq!(h.app.cursor(), Some(sq("e7")));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.cursor(), None);
    }

    #[test]
    fn cursor_moves_follow_the_flip_and_stop_at_the_edge() {
        let mut h = hvh();
        h.press(KeyCode::Up);
        h.press(KeyCode::Up);
        h.press(KeyCode::Left);
        assert_eq!(h.app.cursor(), Some(sq("d3")));
        h.char('f');
        h.press(KeyCode::Up);
        h.press(KeyCode::Left);
        assert_eq!(
            h.app.cursor(),
            Some(sq("e2")),
            "screen up and left on a flipped board"
        );
        for _ in 0..10 {
            h.press(KeyCode::Down);
        }
        assert_eq!(h.app.cursor(), Some(sq("e8")));
        let mut h = Harness::new();
        h.char('3');
        h.press(KeyCode::Up);
        assert_eq!(h.app.cursor(), Some(sq("e7")), "Black at the bottom");
    }

    #[test]
    fn click_then_click_moves_a_piece() {
        let mut h = hvh();
        h.click_square(sq("g1"));
        assert_eq!(h.app.selected(), Some(sq("g1")));
        assert_eq!(h.app.targets(), [sq("f3"), sq("h3")]);
        assert_eq!(h.app.highlights().targets, [sq("f3"), sq("h3")]);
        h.click_square(sq("f3"));
        assert_eq!(h.uci(), ["g1f3"]);
        assert_eq!(h.app.selected(), None);
        h.click_square(sq("f3"));
        assert_eq!(h.app.selected(), None, "White's knight, Black's turn");
    }

    #[test]
    fn drag_and_drop_moves_a_piece() {
        let mut h = hvh();
        h.drag_square(sq("d2"), sq("d4"));
        assert_eq!(h.uci(), ["d2d4"]);
        assert_eq!(h.app.selected(), None);
    }

    #[test]
    fn second_click_deselects_and_a_bad_drop_keeps_the_selection() {
        let mut h = hvh();
        h.click_square(sq("e2"));
        h.click_square(sq("e2"));
        assert_eq!(h.app.selected(), None);
        h.drag_square(sq("e2"), sq("e5"));
        assert_eq!(h.app.selected(), Some(sq("e2")));
        assert!(h.uci().is_empty());
        h.click_square(sq("b1"));
        assert_eq!(h.app.selected(), Some(sq("b1")), "another own piece");
        h.click_square(sq("e5"));
        assert_eq!(h.app.selected(), None, "an empty non-target square");
        h.click_square(sq("b1"));
        h.click(0, 0);
        assert_eq!(h.app.selected(), None, "outside the grid");
    }

    #[test]
    fn mouse_use_hides_the_keyboard_cursor() {
        let mut h = hvh();
        h.press(KeyCode::Up);
        assert!(h.app.cursor().is_some());
        h.click_square(sq("e2"));
        assert_eq!(h.app.cursor(), None);
        assert_eq!(h.app.selected(), Some(sq("e2")));
    }

    #[test]
    fn clicks_before_the_first_draw_do_nothing() {
        let engine = Arc::new(FakeEngine::local());
        let mut app = App::new(engine, GlyphSet::Solid, true, Vec::new());
        let now = Instant::now();
        assert!(app.handle(key(KeyCode::Char('1')), now).is_empty());
        for (column, row) in [(5, 5), (20, 10), (40, 12)] {
            let down = mouse(MouseEventKind::Down(MouseButton::Left), column, row);
            assert!(app.handle(down, now).is_empty());
        }
        assert_eq!(app.selected(), None);
        assert_eq!(app.hit_map(), &HitMap::default());
    }

    #[test]
    fn promotion_picker_by_keyboard() {
        let mut h = hvh();
        h.command(&format!(":fen {PROMOTION_FEN}"));
        h.press(KeyCode::Esc);
        h.click_square(sq("e7"));
        h.click_square(sq("e8"));
        assert_eq!(h.app.dialog_name(), Some("promotion"));
        assert!(h.uci().is_empty());
        h.press(KeyCode::Right);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Promotion { choice: 1, .. })
        ));
        h.press(KeyCode::Left);
        h.press(KeyCode::Left);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Promotion { choice: 3, .. })
        ));
        h.char('k');
        assert_eq!(
            h.app.dialog_name(),
            Some("promotion"),
            "not a promotion piece"
        );
        h.char('N');
        assert_eq!(h.uci(), ["e7e8n"]);
        assert_eq!(h.app.dialog_name(), None);

        let mut h = hvh();
        h.command(&format!(":fen {PROMOTION_FEN}"));
        h.press(KeyCode::Esc);
        h.promote_e7_by_keyboard();
        h.press(KeyCode::Enter);
        assert_eq!(h.uci(), ["e7e8q"], "Enter takes the highlighted queen");
    }

    #[test]
    fn a_typed_promotion_without_its_piece_opens_the_picker() {
        let mut h = hvh();
        h.command(&format!(":fen {PROMOTION_FEN}"));
        h.command("e8");
        assert_eq!(h.app.dialog_name(), Some("promotion"));
        assert!(h.uci().is_empty());
        assert_eq!(h.app.status_line(), "position loaded", "no error");
        h.char('r');
        assert_eq!(h.uci(), ["e7e8r"]);
        assert!(h.app.command_has_focus(), "typing goes on in the box");
    }

    #[test]
    fn promotion_picker_by_mouse_and_cancel() {
        let mut h = hvh();
        h.command(&format!(":fen {PROMOTION_FEN}"));
        h.press(KeyCode::Esc);
        h.drag_square(sq("e7"), sq("e8"));
        assert_eq!(h.app.dialog_name(), Some("promotion"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.dialog_name(), None);
        assert!(h.uci().is_empty());
        assert_eq!(h.app.selected(), Some(sq("e7")));
        h.click_square(sq("e8"));
        h.click_hit(Hit::Promote(PieceKind::Rook));
        assert_eq!(h.uci(), ["e7e8r"]);
        assert_eq!(h.app.dialog_name(), None);
    }

    #[test]
    fn board_is_locked_on_jevs_turn_and_while_watching() {
        let mut h = Harness::new();
        assert_eq!(h.char('3').len(), 1);
        h.click_square(sq("e7"));
        assert_eq!(h.app.selected(), None);
        assert_eq!(h.app.status_line(), "Local search to move");
        h.command("e5");
        assert!(h.uci().is_empty());
        assert_eq!(h.app.status_line(), "Local search to move");
        assert_eq!(h.app.command_text(), "e5", "kept for after Jev's move");

        let mut h = Harness::new();
        h.char('5');
        h.click_square(sq("e2"));
        assert_eq!(h.app.selected(), None);
        assert_eq!(
            h.app.status_line(),
            "Local search plays both sides; you are watching"
        );
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut h = hvh();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('f'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        h.key(release);
        assert!(!h.app.flipped());
        h.char('f');
        assert!(h.app.flipped());
    }

    #[test]
    fn alt_key_is_escape_then_the_key_outside_text_fields() {
        // Esc typed quickly before a key arrives as Alt+key.
        let mut h = hvh();
        h.click_square(sq("e2"));
        h.alt('f');
        assert_eq!(h.app.selected(), None, "Esc dropped the piece");
        assert!(h.app.flipped(), "then f flipped the board");

        // In a yes/no question: Esc answers No, then the key reaches the board.
        h.char('q');
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.alt('f');
        assert_eq!(h.app.dialog_name(), None);
        assert!(!h.app.should_quit());
        assert!(!h.app.flipped());

        // On the menu: Esc does nothing there, then 1 starts a game.
        let mut h = Harness::new();
        h.alt('1');
        assert_eq!(h.app.mode(), Mode::HumanVsHuman);
        assert_eq!(h.app.screen_name(), "playing");
    }

    #[test]
    fn alt_chords_are_ignored_while_typing() {
        // Readline habits (Alt+b, Alt+f, Alt+d, Alt+Backspace) in a text field must neither
        // change the text nor leave the field and fire board hotkeys such as u (undo).
        let mut h = hvh();
        h.command("e4");
        h.command("e5");
        h.type_text("Nf");
        for c in ['u', 'b', 'f', 'd', 'q'] {
            assert!(h.alt(c).is_empty());
        }
        for code in [
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Enter,
            KeyCode::Esc,
        ] {
            h.send(chord(code, KeyModifiers::ALT));
        }
        assert_eq!(h.app.command_text(), "Nf");
        assert_eq!(h.app.command_editor().cursor(), 2);
        assert!(h.app.command_has_focus());
        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
        assert!(!h.app.flipped());
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.status_line(), "");
        // AltGr arrives as Ctrl+Alt on some platforms and still types.
        h.send(chord(
            KeyCode::Char('3'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        h.press(KeyCode::Enter);
        assert_eq!(h.uci(), ["e2e4", "e7e5", "g1f3"]);

        // The save and FEN dialogs stay open with their text.
        h.ctrl('s');
        h.type_text("game");
        h.alt('b');
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { purpose: InputPurpose::Save(SaveKind::Pgn), editor, .. })
                if editor.text() == "game"
        ));
        h.press(KeyCode::Esc);
        h.command(":fen");
        h.type_text("8/8");
        h.alt('u');
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { purpose: InputPurpose::LoadFen { .. }, editor, .. })
                if editor.text() == "8/8"
        ));
        assert_eq!(h.uci().len(), 3);
    }

    #[test]
    fn ctrl_s_saves_from_the_board_and_the_command_box_only() {
        let mut h = hvh();
        h.command("e4");
        h.type_text("e5");
        assert!(h.app.command_has_focus());
        h.ctrl('s');
        assert_eq!(h.app.dialog_name(), Some("save pgn"));
        // Inside a dialog it does nothing: no second dialog, no text.
        h.ctrl('s');
        assert_eq!(h.app.dialogs.len(), 1);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { editor, .. }) if editor.is_empty()
        ));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.dialog_name(), None);
        assert!(h.app.command_has_focus(), "back in the box");
        assert_eq!(h.app.command_text(), "e5", "with its text");
        h.press(KeyCode::Esc);
        h.ctrl('s');
        assert_eq!(h.app.dialog_name(), Some("save pgn"), "from the board");
        h.press(KeyCode::Esc);
        h.char('?');
        h.ctrl('s');
        assert_eq!(
            h.app.dialog_name(),
            Some("help"),
            "not from other dialogs either"
        );

        let mut h = Harness::new();
        h.ctrl('s');
        assert_eq!(h.app.dialog_name(), None, "nothing to save on the menu");
    }

    #[test]
    fn wheel_scrolls_the_move_list() {
        let mut h = Harness::sized(FakeEngine::local(), 60, 20);
        h.char('1');
        let moves = [
            "e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5", "c2c3", "g8f6", "d2d3", "d7d6", "e1g1",
            "e8g8", "h2h3", "h7h6", "a2a4", "a7a5", "f1e1", "f8e8", "b1d2", "c8e6", "c4e6", "f7e6",
            "d2f1", "d8d7", "f1g3", "a8d8", "d1e2", "g8h8", "c1e3", "c5e3", "e2e3", "b7b6",
        ];
        for mv in moves {
            h.command(mv);
        }
        assert_eq!(h.uci(), moves);
        let rows = move_rows(h.app.game());
        let list = h.app.hit_map().rect_of(Hit::MoveList).expect("move list");
        let visible = usize::from(list.height - 2);
        assert!(rows.len() > visible, "the list overflows at 60x20");
        let (first, last) = (rows[0].as_str(), rows[rows.len() - 1].as_str());
        assert_eq!(first, "1. e4 e5");
        assert!(h.screen().contains(last));
        assert!(!h.screen().contains(first), "the latest rows are shown");
        // The wheel works only over the move list.
        let board = h.app.hit_map().board.expect("board").outer;
        h.mouse(MouseEventKind::ScrollUp, board.x + 1, board.y + 1);
        assert_eq!(h.app.move_scroll(), 0, "not over the list");
        let (x, y) = (list.x + 2, list.y + 1);
        for _ in 0..rows.len() + 5 {
            h.mouse(MouseEventKind::ScrollUp, x, y);
        }
        let most = rows.len() - visible;
        assert_eq!(h.app.move_scroll(), most, "clamped to what does not fit");
        assert!(h.screen().contains(first));
        assert!(!h.screen().contains(last));
        for _ in 0..most {
            h.mouse(MouseEventKind::ScrollDown, x, y);
        }
        assert_eq!(h.app.move_scroll(), 0);
        assert!(h.screen().contains(last));
    }

    // ----- command box -----

    #[test]
    fn command_box_plays_moves_and_keeps_focus() {
        let mut h = hvh();
        h.char('/');
        assert!(h.app.command_has_focus());
        for mv in ["e4", "e5", "Nf3"] {
            h.type_text(mv);
            h.press(KeyCode::Enter);
        }
        assert_eq!(h.uci(), ["e2e4", "e7e5", "g1f3"]);
        assert!(h.app.command_has_focus());
        assert_eq!(h.app.command_text(), "");
        h.press(KeyCode::Esc);
        assert!(!h.app.command_focused());
        h.click_hit(Hit::CommandBox);
        assert!(h.app.command_focused(), "a click focuses it");
        h.type_text("Nc6");
        h.click_square(sq("e2"));
        assert!(!h.app.command_focused(), "a click elsewhere leaves it");
        assert_eq!(h.app.command_text(), "Nc6", "and keeps the text");
    }

    #[test]
    fn command_errors_keep_the_text() {
        let mut h = hvh();
        h.command("Ke2");
        assert_eq!(h.app.status_line(), "not a legal move: Ke2");
        assert!(h.app.message().is_some_and(|m| m.is_error));
        assert_eq!(h.app.command_text(), "Ke2");
        h.ctrl('u');
        h.command("e4");
        assert_eq!(h.uci(), ["e2e4"]);
        assert_eq!(h.app.status_line(), "", "a played move clears the error");
        h.press(KeyCode::Enter);
        assert_eq!(h.app.status_line(), "type a move or :help");
    }

    #[test]
    fn ambiguous_move_text_is_reported() {
        let mut h = hvh();
        h.command(&format!(":fen {TWO_KNIGHTS_FEN}"));
        h.command("Nd2");
        assert_eq!(h.app.status_line(), "ambiguous: Nbd2, Nfd2");
        assert!(h.uci().is_empty());
        h.ctrl('u');
        h.command("Nfd2");
        assert_eq!(h.uci(), ["f3d2"]);
    }

    #[test]
    fn long_text_is_never_echoed_in_full() {
        // A FEN pasted on the playing screen is move text; its error shows only the start.
        let mut h = hvh();
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        h.send(paste(&format!("{fen}\n")));
        assert_eq!(
            h.app.status_line(),
            format!("not a legal move: {}…", &fen[..24])
        );
        assert_eq!(h.app.command_text(), fen, "the box keeps it for editing");
        h.ctrl('u');
        h.command(&format!(":{}", "x".repeat(40)));
        assert_eq!(
            h.app.status_line(),
            format!("unknown command: :{}… (try :help)", "x".repeat(24))
        );
    }

    #[test]
    fn colon_starts_a_command() {
        let mut h = hvh();
        h.char(':');
        assert_eq!(h.app.command_text(), ":");
        h.type_text("flip");
        h.press(KeyCode::Enter);
        assert!(h.app.flipped());
        h.command(":glyphs");
        assert_eq!(h.app.glyphs(), GlyphSet::Outline);
        assert_eq!(h.app.status_line(), "glyphs: outline");
        h.command(":bogus");
        assert_eq!(h.app.status_line(), "unknown command: :bogus (try :help)");
        assert_eq!(h.app.command_text(), ":bogus");
        h.ctrl('u');
        h.command(":help");
        assert_eq!(h.app.dialog_name(), Some("help"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.dialog_name(), None);
        assert!(h.app.command_has_focus(), "focus returns to the box");
        h.command("e4");
        let generation = h.app.generation();
        h.command(":new");
        assert_eq!(
            h.app.dialog_name(),
            Some("new game"),
            "a game is in progress"
        );
        h.char('y');
        assert!(h.uci().is_empty());
        assert!(h.app.generation() > generation);
        assert!(
            !h.app.flipped(),
            "a new game restores the mode's orientation"
        );
        h.command(":undo");
        assert_eq!(h.app.status_line(), NOTHING_TO_UNDO);
        h.command(":quit");
        assert_eq!(h.app.dialog_name(), Some("quit"));
    }

    #[test]
    fn command_box_editing_keys() {
        let mut h = hvh();
        h.char('/');
        h.type_text("xe4");
        h.press(KeyCode::Home);
        h.press(KeyCode::Delete);
        assert_eq!(h.app.command_text(), "e4");
        h.press(KeyCode::End);
        h.ctrl('h');
        assert_eq!(h.app.command_text(), "e");
        h.ctrl('a');
        h.char('d');
        h.press(KeyCode::Right);
        h.press(KeyCode::Backspace);
        h.press(KeyCode::Left);
        assert_eq!(h.app.command_text(), "d");
        assert_eq!(h.app.command_editor().cursor(), 0);
        h.ctrl('e');
        h.type_text("4é");
        assert_eq!(h.app.command_text(), "d4é");
        h.press(KeyCode::Backspace);
        h.press(KeyCode::Enter);
        assert_eq!(h.uci(), ["d2d4"]);
    }

    #[test]
    fn hotkeys_work_only_with_board_focus() {
        let mut h = hvh();
        h.char('/');
        for c in ['f', 'u', 'q', '?', 'g'] {
            h.char(c);
        }
        assert_eq!(h.app.command_text(), "fuq?g");
        assert!(!h.app.flipped());
        assert_eq!(h.app.dialog_name(), None);
        h.press(KeyCode::Esc);
        assert_eq!(h.app.command_text(), "");
        h.char('f');
        assert!(h.app.flipped());
    }

    #[test]
    fn paste_types_into_the_command_box_and_a_newline_submits() {
        let mut h = hvh();
        h.send(paste("e2e4\r\ne7e5"));
        assert_eq!(h.uci(), ["e2e4"], "only the first line");
        assert!(h.app.command_has_focus());
        h.send(paste("e7\u{7}e"));
        assert_eq!(
            h.app.command_text(),
            "e7e",
            "control characters are stripped"
        );
        assert_eq!(h.uci(), ["e2e4"]);
        h.send(paste("5\n"));
        assert_eq!(h.uci(), ["e2e4", "e7e5"]);

        let mut h = Harness::new();
        h.send(paste("e2e4\n"));
        assert_eq!(h.app.screen_name(), "menu", "nothing to paste into");
        h.char('6');
        h.send(paste(&format!("{TWO_KNIGHTS_FEN}\n")));
        assert_eq!(h.app.game().position().to_fen(), TWO_KNIGHTS_FEN);
    }

    // ----- hotkeys and dialogs -----

    #[test]
    fn hotkeys_flip_glyphs_help_new_and_menu() {
        let mut h = hvh();
        h.char('f');
        assert!(h.app.flipped());
        h.char('g');
        assert_eq!(h.app.glyphs(), GlyphSet::Outline);
        h.char('g');
        h.char('g');
        assert_eq!(h.app.glyphs(), GlyphSet::Solid);
        h.char('?');
        assert_eq!(h.app.dialog_name(), Some("help"));
        assert!(h.screen().contains(HELP_LINES[0]));
        h.char('?');
        assert_eq!(h.app.dialog_name(), None);
        h.command("e4");
        h.press(KeyCode::Esc);
        let generation = h.app.generation();
        h.char('n');
        h.char('y');
        assert!(h.uci().is_empty());
        assert!(h.app.generation() > generation);
        assert_eq!(h.app.status_line(), "new game");
        h.char('m');
        assert_eq!(h.app.screen_name(), "menu", "no moves: nothing to lose");
    }

    #[test]
    fn new_game_and_menu_ask_while_a_game_is_in_progress() {
        let mut h = hvh();
        h.command("e4");
        h.press(KeyCode::Esc);
        // Typing lowercase SAN on the board must not throw the game away.
        h.type_text("nf3");
        assert_eq!(h.app.dialog_name(), Some("new game"));
        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), None, "Enter answers No");
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(!h.app.flipped(), "the f in nf3 went to the dialog");
        h.char('m');
        assert_eq!(h.app.dialog_name(), Some("menu"));
        assert!(h.screen().contains("Abandon the game in progress"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.screen_name(), "playing");
        h.char('m');
        h.char('y');
        assert_eq!(h.app.screen_name(), "menu");

        // A finished game is left without asking.
        let mut h = hvh();
        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
        }
        h.press(KeyCode::Esc);
        h.press(KeyCode::Esc);
        h.char('n');
        assert_eq!(h.app.dialog_name(), None);
        assert!(h.uci().is_empty());
    }

    #[test]
    fn a_typed_queen_move_on_the_board_never_quits() {
        // `q` opens the quit question on board focus; the rest of `qh5` and the Enter
        // that follows must not confirm it.
        let mut h = hvh();
        h.command("e4");
        h.command("e5");
        h.click_square(sq("d1"));
        h.click_square(sq("d1"));
        assert!(!h.app.command_focused());
        h.type_text("qh5");
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.press(KeyCode::Enter);
        assert!(!h.app.should_quit());
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
    }

    #[test]
    fn quit_asks_only_while_a_game_is_in_progress() {
        let mut h = hvh();
        h.char('q');
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.char('n');
        assert_eq!(h.app.dialog_name(), None);
        assert!(!h.app.should_quit());
        h.char('q');
        h.press(KeyCode::Enter);
        assert!(!h.app.should_quit(), "No is the default");
        h.char('q');
        h.press(KeyCode::Tab);
        h.press(KeyCode::Enter);
        assert!(h.app.should_quit(), "Tab moved the choice to Yes");

        let mut h = hvh();
        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
        }
        assert_eq!(h.app.screen_name(), "game over");
        h.char('q');
        assert!(h.app.should_quit(), "a finished game quits at once");
    }

    #[test]
    fn ctrl_c_works_from_any_focus() {
        let mut h = hvh();
        h.char('/');
        h.ctrl('c');
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.ctrl('c');
        assert!(h.app.should_quit());

        let mut h = hvh();
        h.char('?');
        h.ctrl('c');
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.dialog_name(), Some("help"));
    }

    #[test]
    fn dialogs_are_modal_for_the_mouse() {
        let mut h = hvh();
        h.char('q');
        h.click_square(sq("e2"));
        assert_eq!(h.app.selected(), None);
        assert_eq!(h.app.dialog_name(), Some("quit"));
        h.click_hit(Hit::Button(Button::Cancel));
        assert_eq!(h.app.dialog_name(), None);
        assert!(!h.app.should_quit());
        h.char('?');
        h.click(1, 1);
        assert_eq!(h.app.dialog_name(), None, "any click closes help");
        h.char('q');
        h.click_hit(Hit::Button(Button::Confirm));
        assert!(h.app.should_quit());
    }

    #[test]
    fn resign_asks_first_and_undo_withdraws_it() {
        let mut h = hvh();
        h.command("e4");
        h.command(":resign");
        assert_eq!(h.app.dialog_name(), Some("resign"));
        h.press(KeyCode::Enter);
        assert_eq!(h.app.game().outcome(), None, "No is the default");
        h.command(":resign");
        h.press(KeyCode::Esc);
        assert_eq!(h.app.game().outcome(), None);
        h.command(":resign");
        h.char('y');
        assert_eq!(h.app.screen_name(), "game over");
        assert_eq!(
            h.app.game().outcome(),
            Some(Outcome::Resignation {
                winner: Side::White
            }),
            "the side to move resigns between humans"
        );
        assert!(h.screen().contains("Black resigned — White wins"));
        h.char('u');
        assert_eq!(h.app.game().outcome(), None);
        assert_eq!(h.app.screen_name(), "playing");
        assert_eq!(h.uci(), ["e2e4"]);
        assert_eq!(h.app.status_line(), "resignation withdrawn");

        let mut h = Harness::new();
        h.char('5');
        h.command(":resign");
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.status_line(), NOTHING_TO_RESIGN);
    }

    #[test]
    fn resigning_against_jev_while_it_thinks() {
        let mut h = Harness::new();
        h.char('2');
        let late = request(&h.command("e4"));
        h.command(":resign");
        h.char('y');
        assert_eq!(
            h.app.game().outcome(),
            Some(Outcome::Resignation {
                winner: Side::Black
            })
        );
        assert!(!h.app.is_thinking());
        assert!(h.respond(&late).is_empty());
        assert_eq!(h.uci(), ["e2e4"], "the late reply is discarded");
        // Withdrawing the resignation hands the move back to Jev.
        let again = request(&h.char('u'));
        assert_eq!(again.generation, h.app.generation());
        assert_ne!(again.generation, late.generation);
    }

    // ----- undo -----

    #[test]
    fn undo_takes_back_one_ply_between_humans() {
        let mut h = hvh();
        h.command("e4");
        h.command("e5");
        h.press(KeyCode::Esc);
        let generation = h.app.generation();
        h.char('u');
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(h.app.generation() > generation);
        assert_eq!(h.app.status_line(), "took back 1 move");
        h.char('u');
        h.char('u');
        assert!(h.uci().is_empty());
        assert_eq!(h.app.status_line(), NOTHING_TO_UNDO);
    }

    #[test]
    fn undo_against_jev_takes_back_two_plies() {
        let mut h = Harness::with_engine(FakeEngine::local().playing(&["e7e5"]));
        h.char('2');
        let request = request(&h.command("e4"));
        assert!(h.respond(&request).is_empty());
        assert_eq!(h.uci(), ["e2e4", "e7e5"]);
        assert_eq!(h.app.last_computer().map(|c| c.san.as_str()), Some("e5"));
        h.press(KeyCode::Esc);
        assert!(h.char('u').is_empty(), "the person's turn again");
        assert!(h.uci().is_empty());
        assert_eq!(h.app.status_line(), "took back 2 moves");
    }

    #[test]
    fn undo_while_jev_thinks_discards_the_late_reply() {
        let mut h = Harness::with_engine(FakeEngine::local().playing(&["e7e5"]));
        h.char('2');
        let late = request(&h.command("e4"));
        h.press(KeyCode::Esc);
        assert!(h.app.is_thinking());
        assert!(h.char('u').is_empty());
        assert!(!h.app.is_thinking());
        assert!(h.uci().is_empty());
        assert_eq!(h.app.status_line(), "took back 1 move");
        assert!(h.respond(&late).is_empty());
        assert!(h.uci().is_empty(), "the stale reply is not played");
        let fresh = request(&h.command("d4"));
        assert_eq!(fresh.generation, h.app.generation());
        assert_ne!(fresh.generation, late.generation);
        h.respond(&fresh);
        assert_eq!(h.uci().len(), 2);
    }

    #[test]
    fn nothing_to_undo_when_jev_moved_first() {
        let mut h = Harness::with_engine(FakeEngine::local().playing(&["e2e4"]));
        let first = request(&h.char('3'));
        h.respond(&first);
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(h.char('u').is_empty());
        assert_eq!(h.uci(), ["e2e4"]);
        assert_eq!(h.app.status_line(), NOTHING_TO_UNDO);
    }

    // ----- engine replies -----

    #[test]
    fn a_reply_for_another_position_is_discarded() {
        let mut h = Harness::new();
        h.char('2');
        let request = request(&h.command("e4"));
        let mut reply = h.reply_for(&request);
        reply.hash = ChessPosition::startpos().hash();
        assert!(h.send(AppEvent::Engine(reply)).is_empty());
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(h.app.is_thinking(), "still waiting for the real reply");
        h.respond(&request);
        assert_eq!(h.uci().len(), 2);
        assert!(!h.app.is_thinking());
        assert_eq!(
            h.app.last_computer().map(|c| c.source.clone()),
            Some(MoveSource::Jev)
        );
    }

    #[test]
    fn a_stray_error_while_jev_thinks_does_not_outlive_its_reply() {
        // Typing a move while it is not the person's turn sets an error; once the engine
        // replies and it is the person's turn again, that stale error must not remain.
        let mut h = Harness::new();
        h.char('2');
        let request = request(&h.command("e4"));
        assert!(h.app.is_thinking());
        h.command("d4");
        assert_eq!(h.app.status_line(), "Local search to move");
        h.respond(&request);
        assert_eq!(h.app.status_line(), "");
    }

    #[test]
    fn a_move_recovered_after_an_engine_panic_is_announced() {
        // The worker ran the local search after the engine panicked; the UI only plays it.
        let mut h = Harness::new();
        h.char('2');
        let request = request(&h.command("e4"));
        let best = analyse(&request.game)[0].mv;
        let computer = ComputerMove {
            mv: best,
            san: request.game.position().to_san(best),
            source: MoveSource::Fallback,
            top: Vec::new(),
            confidence: None,
            model: None,
            latency: Duration::from_millis(300),
            input_tokens: None,
            note: Some(ENGINE_ERROR_NOTE.to_string()),
        };
        h.send(AppEvent::Engine(EngineReply {
            generation: request.generation,
            hash: request.hash,
            outcome: EngineOutcome::Move(computer),
        }));
        assert_eq!(h.app.game().moves().last(), Some(&best));
        assert_eq!(h.app.status_line(), ENGINE_ERROR_NOTE);
        assert!(h.app.message().is_some_and(|m| m.is_error));
        assert!(h.screen().contains(ENGINE_ERROR_NOTE));
        assert!(!h.app.is_thinking());
        assert!(!h.app.engine_failed());
    }

    #[test]
    fn an_engine_failure_stops_asking_until_space_retries() {
        let mut h = Harness::new();
        h.char('2');
        let first = request(&h.command("e4"));
        h.press(KeyCode::Esc);
        let failed = EngineOutcome::Failed("engine panicked: boom; local search panicked".into());
        assert!(
            h.answer(&first, failed).is_empty(),
            "nothing is computed here"
        );
        assert_eq!(h.uci(), ["e2e4"], "no move is played");
        assert!(h.app.engine_failed());
        assert!(!h.app.is_thinking());
        assert_eq!(h.app.status_line(), ENGINE_FAILED);
        h.at_ms(10_000);
        assert!(h.tick().is_empty(), "no new request while failed");
        let retry = request(&h.char(' '));
        assert_eq!(retry.hash, first.hash);
        assert!(!h.app.engine_failed());
        h.respond(&retry);
        assert_eq!(h.uci().len(), 2);
    }

    #[test]
    fn space_in_an_empty_command_box_retries_a_failed_engine() {
        // After a typed move the box keeps focus, so space must retry from there too.
        let mut h = Harness::new();
        h.char('2');
        let first = request(&h.command("e4"));
        assert!(h.app.command_has_focus());
        h.answer(&first, EngineOutcome::Failed("boom".into()));
        assert_eq!(h.app.status_line(), ENGINE_FAILED);
        // While typing, space is just a space.
        h.type_text("Nf");
        assert!(h.char(' ').is_empty());
        assert_eq!(h.app.command_text(), "Nf ");
        assert!(h.app.engine_failed());
        h.ctrl('u');
        let retry = request(&h.char(' '));
        assert_eq!(retry.hash, first.hash);
        assert!(!h.app.engine_failed());
        assert_eq!(h.app.command_text(), "", "no space was typed");
        assert!(h.app.command_has_focus());

        // Watching, space in an empty box pauses like on the board.
        let mut h = Harness::new();
        h.char('5');
        h.char('/');
        h.char(' ');
        assert!(h.app.paused());
        assert_eq!(h.app.command_text(), "");
    }

    #[test]
    fn undo_or_a_new_position_clears_an_engine_failure() {
        let mut h = Harness::new();
        h.char('2');
        let first = request(&h.command("e4"));
        h.press(KeyCode::Esc);
        h.answer(&first, EngineOutcome::Failed("boom".into()));
        assert!(h.char('u').is_empty(), "the person's turn again");
        assert!(!h.app.engine_failed());
        assert_eq!(request(&h.command("d4")).game.moves().len(), 1);
    }

    #[test]
    fn a_game_over_reply_for_a_live_game_is_a_failure() {
        let mut h = Harness::new();
        h.char('2');
        let request = request(&h.command("e4"));
        assert!(h.answer(&request, EngineOutcome::GameOver).is_empty());
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(h.app.engine_failed());
        assert_eq!(h.app.status_line(), ENGINE_FAILED);
    }

    #[test]
    fn an_illegal_move_from_the_engine_is_a_failure() {
        let mut h = Harness::new();
        h.char('2');
        let request = request(&h.command("e4"));
        // A legal move of the start position, which is not the position Jev was asked.
        let wrong = ChessPosition::startpos()
            .parse_uci("d2d4")
            .expect("legal at the start");
        let mut reply = h.reply_for(&request);
        if let EngineOutcome::Move(computer) = &mut reply.outcome {
            computer.mv = wrong;
        }
        h.send(AppEvent::Engine(reply));
        assert_eq!(h.uci(), ["e2e4"]);
        assert!(h.app.engine_failed());
    }

    #[test]
    fn at_most_two_engine_requests_are_out_at_once() {
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        assert_eq!(h.app.in_flight(), 1);
        // No moves yet, so `n` starts over at once; the first request is still running.
        let second = request(&h.char('n'));
        assert_eq!(h.app.in_flight(), 2);
        for _ in 0..10 {
            assert!(h.char('n').is_empty(), "capped");
        }
        assert!(h.app.waiting_for_engine(h.now));
        assert!(h.screen().contains(WAITING_FOR_ENGINE));
        assert!(!h.app.is_thinking());
        // A discarded answer frees a slot for the current position.
        let third = request(&h.respond(&first));
        assert_eq!(third.generation, h.app.generation());
        assert_eq!(h.app.in_flight(), 2);
        assert!(
            h.respond(&second).is_empty(),
            "stale, and the live one is pending"
        );
        assert_eq!(h.app.in_flight(), 1);
        h.respond(&third);
        assert_eq!(h.uci().len(), 1);
        assert_eq!(h.app.in_flight(), 0);
    }

    #[test]
    fn pausing_while_jev_thinks_holds_its_move() {
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        h.char(' ');
        assert!(h.app.paused());
        assert!(h.respond(&first).is_empty());
        assert!(h.uci().is_empty(), "paused: nothing is played");
        assert!(h.app.has_held_move());
        h.at_ms(5000);
        assert!(h.tick().is_empty());
        h.at_ms(6000);
        assert!(h.char(' ').is_empty(), "resumed: the held move is played");
        assert_eq!(h.uci().len(), 1);
        assert!(!h.app.has_held_move());
        h.at_ms(6999);
        assert!(h.tick().is_empty(), "the next move waits a step from now");
        h.at_ms(7000);
        assert_eq!(h.tick().len(), 1);

        // Undo while holding throws the held move away.
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        h.respond(&first);
        h.at_ms(1000);
        let second = request(&h.tick());
        h.char(' ');
        h.respond(&second);
        assert!(h.app.has_held_move());
        h.char('u');
        assert!(!h.app.has_held_move());
        assert!(h.uci().is_empty());
        assert_eq!(
            h.char(' ').len(),
            1,
            "resumed: asked again for the first move"
        );
        assert!(h.uci().is_empty());
    }

    #[test]
    fn undoing_the_computer_move_clears_the_jev_panel() {
        let mut h = Harness::with_engine(FakeEngine::jev().playing(&["e7e5"]));
        h.char('2');
        let request = request(&h.command("e4"));
        h.respond(&request);
        assert!(h.app.last_computer().is_some());
        assert!(h.screen().contains("played e5"));
        h.press(KeyCode::Esc);
        h.char('u');
        assert!(h.app.last_computer().is_none());
        assert!(!h.screen().contains("played e5"));
    }

    #[test]
    fn replies_arrive_through_a_worker_thread() {
        let mut h = Harness::with_engine(FakeEngine::jev().playing(&["c7c5"]));
        h.char('2');
        let request = request(&h.command("e4"));
        let (tx, rx) = mpsc::channel();
        spawn_request(Arc::clone(h.app.engine()), request, tx).expect("spawn engine thread");
        let reply = rx.recv_timeout(REPLY_TIMEOUT).expect("engine reply");
        h.send(AppEvent::Engine(reply));
        assert_eq!(h.uci(), ["e2e4", "c7c5"]);
        let computer = h.app.last_computer().expect("computer move");
        assert_eq!(computer.model.as_deref(), Some("jev-test"));
        assert!(h.screen().contains("played c5 · Jev"));
    }

    #[test]
    fn thinking_shows_elapsed_time_and_a_spinner() {
        let mut h = Harness::with_engine(FakeEngine::jev());
        h.char('2');
        h.command("e4");
        assert_eq!(
            h.app.thinking(h.t0 + Duration::from_millis(250)),
            Some(Thinking {
                elapsed: Duration::from_millis(250),
                spinner: "-",
            })
        );
        h.at_ms(1300);
        h.tick();
        assert!(h.screen().contains("/ Jev thinking... 1.3s"));
        assert_eq!(h.app.turn_text(), "Black to move (Jev)");
    }

    // ----- Jev vs Jev -----

    #[test]
    fn jev_vs_jev_waits_the_step_delay() {
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        h.at_ms(100);
        assert!(h.respond(&first).is_empty(), "the next move waits");
        assert_eq!(h.uci().len(), 1);
        h.at_ms(1099);
        assert!(h.tick().is_empty());
        h.at_ms(1100);
        let second = request(&h.tick());
        assert_eq!(second.game.moves().len(), 1);
        assert_eq!(second.hash, h.app.game().position().hash());
    }

    #[test]
    fn jev_vs_jev_pause_and_step_delay_limits() {
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        h.respond(&first);
        h.char(' ');
        assert!(h.app.paused());
        assert_eq!(h.app.status_line(), "", "the pace line shows it");
        assert!(h.screen().contains("paused · space resumes"));
        h.at_ms(2000);
        assert!(h.tick().is_empty());
        for _ in 0..20 {
            h.char('-');
        }
        assert_eq!(h.app.step_delay(), Duration::from_millis(200));
        for _ in 0..20 {
            h.char('+');
        }
        assert_eq!(h.app.step_delay(), Duration::from_secs(5));
        assert_eq!(h.app.status_line(), "");
        h.char('-');
        h.char('=');
        assert_eq!(h.app.step_delay(), Duration::from_secs(5), "= works as +");
        assert!(h.char(' ').is_empty(), "resumed, but 5 s have not passed");
        assert!(!h.app.paused());
        assert!(h.screen().contains("step 5.0 s · space pauses"));
        h.at_ms(4999);
        assert!(h.tick().is_empty());
        h.at_ms(5000);
        assert_eq!(h.tick().len(), 1);
    }

    #[test]
    fn jev_vs_jev_stops_at_game_over() {
        let mut h = Harness::with_engine(FakeEngine::local().playing(&FOOLS_MATE));
        let mut actions = h.char('5');
        for step in 1..=4 {
            let request = request(&actions);
            h.respond(&request);
            h.at_ms(step * 1000);
            actions = h.tick();
        }
        assert!(actions.is_empty());
        assert_eq!(h.uci(), FOOLS_MATE);
        assert_eq!(h.app.screen_name(), "game over");
        h.at_ms(60_000);
        assert!(h.tick().is_empty());
    }

    #[test]
    fn jev_vs_jev_undo_pauses() {
        let mut h = Harness::new();
        let first = request(&h.char('5'));
        h.respond(&first);
        assert!(h.char('u').is_empty());
        assert!(h.uci().is_empty());
        assert!(h.app.paused());
        assert_eq!(h.app.status_line(), "took back 1 move");
        assert!(h.screen().contains("paused · space resumes"));
        h.at_ms(5000);
        assert!(h.tick().is_empty());
        assert_eq!(h.char(' ').len(), 1);
    }

    // ----- saving -----

    #[test]
    fn save_pgn_dialog_writes_and_asks_before_overwriting() {
        let dir = TempDir::new("save-pgn");
        let typed = dir.path().join("game");
        let typed = typed.to_str().expect("utf-8 temp path");
        let path = dir.path().join("game.pgn");
        let mut h = hvh();
        h.command("e4");
        h.press(KeyCode::Esc);
        h.ctrl('s');
        assert_eq!(h.app.dialog_name(), Some("save pgn"));
        h.type_text(typed);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.status_line(), format!("saved {}", path.display()));
        let saved = fs::read_to_string(&path).expect("saved file");
        assert!(saved.contains("[Date \"2026.09.27\"]"));
        assert!(saved.contains("[White \"You\"]"));
        assert!(saved.contains("[Black \"You\"]"));
        assert!(saved.contains("\n1. e4 *\n"));

        h.command("e5");
        h.press(KeyCode::Esc);
        h.ctrl('s');
        h.type_text(typed);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), Some("overwrite"));
        assert!(h.screen().contains("exists. Overwrite it?"));
        h.press(KeyCode::Enter);
        assert_eq!(h.app.status_line(), NOT_SAVED, "No is the default");
        assert_eq!(fs::read_to_string(&path).expect("file"), saved);
        // No gives the typed path back, ready to change or save again.
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { purpose: InputPurpose::Save(SaveKind::Pgn), editor, error: None })
                if editor.text() == typed && editor.cursor() == typed.chars().count()
        ));

        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), Some("overwrite"));
        h.char('y');
        assert_eq!(h.app.status_line(), format!("saved {}", path.display()));
        assert!(
            fs::read_to_string(&path)
                .expect("file")
                .contains("1. e4 e5 *")
        );
    }

    #[test]
    fn savefen_command_adds_the_extension_and_a_newline() {
        let dir = TempDir::new("save-fen");
        let base = dir.path().join("position");
        let path = dir.path().join("position.fen");
        let mut h = hvh();
        h.command("e4");
        h.command(&format!(":savefen {}", base.display()));
        let expected = format!("{}\n", h.app.game().position().to_fen());
        assert_eq!(fs::read_to_string(&path).expect("saved file"), expected);
        assert_eq!(h.app.status_line(), format!("saved {}", path.display()));
        assert_eq!(h.app.command_text(), "");
        h.command(&format!(":savefen {}", base.display()));
        assert_eq!(h.app.dialog_name(), Some("overwrite"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.status_line(), NOT_SAVED);
        assert_eq!(
            h.app.command_text(),
            format!(":savefen {}", base.display()),
            "No puts the command back"
        );
        assert!(h.app.command_has_focus());
        h.ctrl('u');
        h.command(&format!(":savepgn {}", base.display()));
        assert!(
            dir.path().join("position.pgn").exists(),
            "a different extension"
        );
    }

    #[test]
    fn save_errors_are_reported_where_the_path_was_typed() {
        let dir = TempDir::new("save-errors");
        let missing = dir.path().join("missing").join("game");
        let mut h = hvh();
        h.command(&format!(":savepgn {}", missing.display()));
        assert!(h.app.status_line().starts_with("cannot save "));
        assert!(h.app.status_line().ends_with(": folder does not exist"));
        assert!(
            h.app.command_text().starts_with(":savepgn "),
            "kept for fixing"
        );
        h.press(KeyCode::Esc);

        h.ctrl('s');
        h.type_text(missing.to_str().expect("utf-8 temp path"));
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { error: Some(e), .. }) if e.ends_with(": folder does not exist")
        ));
        h.ctrl('u');
        h.press(KeyCode::Enter);
        assert!(matches!(
            h.app.dialog(),
            Some(Dialog::Input { error: Some(e), .. }) if e == "type a file name"
        ));
        h.click_hit(Hit::Button(Button::Cancel));
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(fs::read_dir(dir.path()).expect("temp dir").count(), 0);
    }

    #[test]
    fn save_paths_expand_the_home_folder() {
        let dir = TempDir::new("save-home");
        let home = dir.path().to_path_buf();
        let mut h = Harness::build(FakeEngine::local(), (80, 24), Vec::new(), move |app| {
            app.with_home(Some(home))
        });
        h.char('1');
        h.command(":savepgn ~/tilde");
        assert!(
            dir.path().join("tilde.pgn").exists(),
            "{}",
            h.app.status_line()
        );

        let mut h = hvh();
        h.command(":savepgn ~/tilde");
        assert_eq!(
            h.app.status_line(),
            "cannot expand ~ in ~/tilde: HOME is not set"
        );
    }

    #[test]
    fn without_a_jev_key_the_computer_is_called_local_search() {
        // The only "Jev" left on screen is the variable name in the engine status.
        let without_key = |text: String| text.replace("JEV_API_KEY", "");
        let mut h = Harness::sized(FakeEngine::local(), 120, 40);
        let menu = h.screen();
        assert!(!without_key(menu.clone()).contains("Jev"), "{menu}");
        assert!(menu.contains("Human vs Local search: play White"));
        for _ in 0..MENU_ITEMS.len() {
            h.press(KeyCode::Down);
            let screen = h.screen();
            let index = h.app.menu_index();
            assert!(
                !without_key(screen.clone()).contains("Jev"),
                "{index}: {screen}"
            );
            if MENU_ITEMS[index] == MenuItem::HumanVsJev(SidePick::Black) {
                assert!(screen.contains("Local search plays White and opens"));
            }
        }

        h.char('3');
        assert_eq!(h.app.mode_label(), "You (Black) vs Local search");
        assert_eq!(h.app.turn_text(), "White to move (Local search)");
        let screen = h.screen();
        assert!(screen.contains("You (Black) vs Local search"), "{screen}");
        assert!(screen.contains("Local search thinking... 0.0s"), "{screen}");
        assert!(!without_key(screen.clone()).contains("Jev"), "{screen}");
        h.click_square(sq("e7"));
        assert_eq!(h.app.status_line(), "Local search to move");
        h.char('?');
        assert!(!h.screen().contains("Jev"), "help");

        let mut h = Harness::new();
        h.char('5');
        assert_eq!(h.app.mode_label(), "Local search vs Local search");
        h.click_square(sq("e2"));
        assert_eq!(
            h.app.status_line(),
            "Local search plays both sides; you are watching"
        );

        // With a key it is Jev.
        let mut h = Harness::with_engine(FakeEngine::jev());
        assert!(h.screen().contains("Human vs Jev: play White"));
        h.char('3');
        assert_eq!(h.app.mode_label(), "You (Black) vs Jev");
        assert_eq!(h.app.turn_text(), "White to move (Jev)");
        h.click_square(sq("e7"));
        assert_eq!(h.app.status_line(), "Jev to move");
    }

    #[test]
    fn mode_labels_shorten_step_by_step() {
        let white = Mode::HumanVsJev { human: Side::White };
        let black = Mode::HumanVsJev { human: Side::Black };
        let local = |mode: Mode| mode.labels(LOCAL_SEARCH_NAME, LOCAL_SEARCH_SHORT_NAME);
        let jev = |mode: Mode| mode.labels(JEV_NAME, JEV_NAME);
        assert_eq!(
            local(white),
            [
                "You (White) vs Local search",
                "You (W) vs Local",
                "W You · B Local"
            ]
        );
        assert_eq!(
            local(black),
            [
                "You (Black) vs Local search",
                "You (B) vs Local",
                "W Local · B You"
            ]
        );
        assert_eq!(
            local(Mode::JevVsJev),
            ["Local search vs Local search", "Local vs Local"]
        );
        assert_eq!(
            jev(white),
            ["You (White) vs Jev", "You (W) vs Jev", "W You · B Jev"]
        );
        assert_eq!(jev(Mode::JevVsJev), ["Jev vs Jev"]);
        assert_eq!(local(Mode::HumanVsHuman), ["Human vs Human"]);

        let mut h = Harness::new();
        h.char('3');
        assert_eq!(h.app.mode_labels(), local(black));
        let mut h = Harness::with_engine(FakeEngine::jev());
        h.char('5');
        assert_eq!(h.app.mode_labels(), jev(Mode::JevVsJev));
    }

    #[test]
    fn the_wait_message_fits_one_row_at_the_minimum_size() {
        for (engine, expected) in [
            (FakeEngine::local(), "Local search to move"),
            (FakeEngine::jev(), "Jev to move"),
        ] {
            let mut h = Harness::sized(engine, MIN_WIDTH, MIN_HEIGHT);
            h.char('3');
            h.click_square(sq("e7"));
            assert_eq!(h.app.status_line(), expected);
            let screen = h.screen();
            assert!(screen.contains(&format!("│ {expected} ")), "{screen}");
        }
    }

    #[test]
    fn pgn_player_names_follow_the_mode_and_engine() {
        let mut h = Harness::new();
        let mut names = Vec::new();
        for digit in ['1', '2', '3', '5'] {
            h.char(digit);
            names.push(h.app.player_names());
            h.char('m');
        }
        assert_eq!(
            names,
            [
                ("You", "You"),
                ("You", "Local search"),
                ("Local search", "You"),
                ("Local search", "Local search"),
            ]
        );
        let mut h = Harness::with_engine(FakeEngine::jev());
        h.char('3');
        assert_eq!(h.app.player_names(), ("Jev", "You"));
        let pgn = h.app.export(SaveKind::Pgn);
        assert!(pgn.contains("[White \"Jev\"]") && pgn.contains("[Black \"You\"]"));
    }

    // ----- game over -----

    #[test]
    fn game_over_overlay_offers_new_game_save_and_menu() {
        let mut h = hvh();
        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
        }
        assert_eq!(h.app.screen_name(), "game over");
        assert_eq!(h.app.turn_text(), "Checkmate — Black wins (0-1)");
        assert!(h.screen().contains("Checkmate — Black wins"));
        assert!(h.screen().contains("[ New game ]"));
        h.press(KeyCode::Right);
        assert_eq!(h.app.game_over_choice(), 1);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), Some("save pgn"));
        h.press(KeyCode::Esc);
        assert_eq!(h.app.screen_name(), "game over");
        h.click_hit(Hit::Button(Button::NewGame));
        assert_eq!(h.app.screen_name(), "playing");
        assert!(h.uci().is_empty());

        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
        }
        h.click_hit(Hit::Button(Button::Menu));
        assert_eq!(h.app.screen_name(), "menu");
        assert!(!h.app.command_focused());
    }

    #[test]
    fn dismissed_game_over_shows_the_final_position() {
        let mut h = hvh();
        for mv in ["f3", "e5", "g4", "Qh4#"] {
            h.command(mv);
        }
        h.press(KeyCode::Esc);
        assert_eq!(h.app.screen_name(), "playing");
        assert!(h.app.game().outcome().is_some());
        assert_eq!(h.app.highlights().check, Some(sq("e1")));
        h.command("a3");
        assert_eq!(h.app.status_line(), GAME_IS_OVER);
        h.press(KeyCode::Esc);
        h.char('u');
        assert_eq!(h.app.game().outcome(), None);
        assert_eq!(h.uci(), ["f2f3", "e7e5", "g2g4"]);
        assert_eq!(h.app.screen_name(), "playing");
    }

    #[test]
    fn bare_file_commands_open_their_dialogs() {
        let mut h = Harness::new();
        h.char('2');
        h.command(":fen");
        assert_eq!(h.app.dialog_name(), Some("load fen"));
        assert_eq!(h.app.command_text(), "");
        h.type_text(TWO_KNIGHTS_FEN);
        h.press(KeyCode::Enter);
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.game().position().to_fen(), TWO_KNIGHTS_FEN);
        assert_eq!(
            h.app.mode(),
            Mode::HumanVsJev { human: Side::White },
            "loaded in the current mode"
        );
        h.command(": SavePGN ");
        assert_eq!(h.app.dialog_name(), Some("save pgn"));
        h.press(KeyCode::Esc);
        h.command(":savefen");
        assert_eq!(h.app.dialog_name(), Some("save fen"));
        h.press(KeyCode::Esc);
        h.command(":fen x");
        assert_eq!(h.app.dialog_name(), None);
        assert_eq!(h.app.status_line(), "invalid FEN: expected 4 to 6 fields");
    }

    #[test]
    fn new_game_and_menu_while_jev_thinks_discard_the_late_reply() {
        let mut h = Harness::new();
        h.char('2');
        let late = request(&h.command("e4"));
        h.press(KeyCode::Esc);
        h.char('n');
        h.char('y');
        assert!(!h.app.is_thinking());
        assert!(h.respond(&late).is_empty());
        assert!(h.uci().is_empty());
        let late = request(&h.command("d4"));
        h.press(KeyCode::Esc);
        h.char('m');
        h.char('y');
        assert!(h.respond(&late).is_empty());
        assert_eq!(h.app.screen_name(), "menu");
        assert_eq!(
            h.uci(),
            ["d2d4"],
            "the menu keeps showing the last game untouched"
        );
    }

    #[test]
    fn a_finished_position_loads_straight_into_game_over() {
        let mut h = hvh();
        h.command(":fen 7k/5Q2/6K1/8/8/8/8/8 b - - 0 1");
        assert_eq!(h.app.screen_name(), "game over");
        assert_eq!(h.app.turn_text(), "Stalemate — draw (1/2-1/2)");
    }

    // ----- rendering helpers -----

    #[test]
    fn too_small_terminal_shows_only_a_message_and_ignores_input() {
        let mut h = Harness::sized(FakeEngine::local(), 59, 20);
        assert!(h.screen().contains(TOO_SMALL));
        h.char('1');
        assert_eq!(
            h.app.screen_name(),
            "menu",
            "keys do not act on a hidden screen"
        );
        h.send(paste("e4\n"));
        h.mouse(MouseEventKind::ScrollUp, 10, 10);
        h.click(10, 10);
        assert_eq!(h.app.hit_map(), &HitMap::default());
        assert!(h.screen().contains(TOO_SMALL));
        h.ctrl('c');
        assert!(
            h.app.should_quit(),
            "Ctrl+C quits at once: no question can be seen"
        );

        // A game that was running when the terminal shrank keeps its state.
        let mut h = hvh();
        h.command("e4");
        h.terminal.backend_mut().resize(40, 10);
        h.draw();
        h.command("e5");
        h.char('n');
        assert_eq!(h.uci(), ["e2e4"]);
        assert_eq!(h.app.dialog_name(), None);
        h.terminal.backend_mut().resize(80, 24);
        h.draw();
        h.command("e5");
        assert_eq!(h.uci(), ["e2e4", "e7e5"]);

        let mut h = Harness::sized(FakeEngine::local(), 60, 20);
        h.char('1');
        let board = h.app.hit_map().board.expect("board fits at 60x20");
        assert_eq!((board.square_w, board.square_h), (3, 1));
    }

    #[test]
    fn move_rows_pair_moves_by_number() {
        let game = game_from(START_FEN, &["e2e4", "e7e5", "g1f3"]);
        assert_eq!(move_rows(&game), ["1. e4 e5", "2. Nf3"]);
        let game = game_from("4k3/8/8/8/8/8/4P3/4K3 b - - 0 12", &["e8d7", "e2e4"]);
        assert_eq!(move_rows(&game), ["12... Kd7", "13. e4"]);
        assert!(move_rows(&Game::new()).is_empty());
    }

    #[test]
    fn fen_errors_never_echo_the_fen() {
        let error = Game::from_fen("8/8/8/8/8/8/8/8 w - - 0 1").expect_err("no kings");
        assert_eq!(
            fen_error(&error),
            "invalid FEN: each side needs exactly one king"
        );
        let error = Game::from_fen("4k3/8/8/8/8/8/8/4R1K1 w - - 0 1").expect_err("check");
        assert_eq!(
            fen_error(&error),
            "invalid FEN: side not to move is in check"
        );
    }

    #[test]
    fn outcome_texts() {
        assert_eq!(
            outcome_text(Outcome::Checkmate {
                winner: Side::White
            }),
            "Checkmate — White wins"
        );
        assert_eq!(
            outcome_text(Outcome::Resignation {
                winner: Side::Black
            }),
            "White resigned — Black wins"
        );
        assert_eq!(
            outcome_text(Outcome::ThreefoldRepetition),
            "Threefold repetition — draw"
        );
    }
}
