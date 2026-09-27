//! Test scaffolding that needs the app: a `TestBackend` harness that drives an [`App`] the
//! way the run loop does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
};

use super::engine::{FakeEngine, chord, jev_exchange, jev_move, key, mouse};
use super::{TEST_DATE, buffer_text, uci_moves};
use crate::core::{Color as Side, Square};
use crate::engine::ComputerMove;
use crate::tui::app::{Action, App, Hit};
use crate::tui::board::square_rect;
use crate::tui::event::AppEvent;
use crate::tui::glyphs::GlyphSet;
use crate::tui::worker::{Engine, EngineOutcome, EngineReply, EngineRequest};

/// Drives an [`App`] like the run loop does: every event is followed by a draw into a
/// fixed-size `TestBackend`, at an instant the test controls (`t0` until moved on).
///
/// The app has no home folder, dates saved games [`TEST_DATE`] and gives White to "random
/// side". Engine requests are not run: the test answers them ([`Harness::respond`],
/// [`Harness::reply`]) or hands them to a worker itself.
pub(crate) struct Harness {
    /// The app under test.
    pub(crate) app: App,
    /// The engine the app was built with.
    pub(crate) engine: Arc<FakeEngine>,
    /// What the app drew last.
    pub(crate) terminal: Terminal<TestBackend>,
    /// The time the harness started.
    pub(crate) t0: Instant,
    /// The time events are stamped with.
    pub(crate) now: Instant,
    /// The latest engine request the app asked for, until [`Harness::reply`] answers it.
    pub(crate) request: Option<EngineRequest>,
}

impl Harness {
    /// An 80×24 app on the menu, playing without a Jev key.
    pub(crate) fn new() -> Harness {
        Harness::with_engine(FakeEngine::local())
    }

    /// An 80×24 app on the menu with `engine`.
    pub(crate) fn with_engine(engine: FakeEngine) -> Harness {
        Harness::sized(engine, 80, 24)
    }

    /// A `width`×`height` app on the menu with `engine`.
    pub(crate) fn sized(engine: FakeEngine, width: u16, height: u16) -> Harness {
        Harness::build(engine, (width, height), Vec::new(), |app| app)
    }

    /// A `width`×`height` app with `engine` and startup `warnings`, changed by `configure`
    /// before the first draw.
    pub(crate) fn build(
        engine: FakeEngine,
        (width, height): (u16, u16),
        warnings: Vec<String>,
        configure: impl FnOnce(App) -> App,
    ) -> Harness {
        let engine = Arc::new(engine);
        let app = App::new(engine.clone(), GlyphSet::Solid, true, warnings)
            .with_home(None)
            .with_date(|| TEST_DATE.to_string())
            .with_side_picker(|| Side::White);
        let t0 = Instant::now();
        let mut harness = Harness {
            app: configure(app),
            engine,
            terminal: Terminal::new(TestBackend::new(width, height)).expect("test terminal"),
            t0,
            now: t0,
            request: None,
        };
        harness.draw();
        harness
    }

    /// Draws the app at [`Harness::now`].
    pub(crate) fn draw(&mut self) {
        let now = self.now;
        self.terminal
            .draw(|frame| self.app.render(frame, now))
            .expect("draw");
    }

    /// Hands `event` to the app, draws, and returns the actions (remembering the request).
    pub(crate) fn send(&mut self, event: AppEvent) -> Vec<Action> {
        let actions = self.app.handle(event, self.now);
        for action in &actions {
            if let Action::RequestEngine(request) = action {
                self.request = Some(request.clone());
            }
        }
        self.draw();
        actions
    }

    /// Sends `key`.
    pub(crate) fn key(&mut self, key: KeyEvent) -> Vec<Action> {
        self.send(AppEvent::Term(Event::Key(key)))
    }

    /// Presses `code` without modifiers.
    pub(crate) fn press(&mut self, code: KeyCode) -> Vec<Action> {
        self.send(key(code))
    }

    /// Types `c`.
    pub(crate) fn char(&mut self, c: char) -> Vec<Action> {
        self.press(KeyCode::Char(c))
    }

    /// Presses Ctrl+`c`.
    pub(crate) fn ctrl(&mut self, c: char) -> Vec<Action> {
        self.send(chord(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    /// Presses Alt+`c`.
    pub(crate) fn alt(&mut self, c: char) -> Vec<Action> {
        self.send(chord(KeyCode::Char(c), KeyModifiers::ALT))
    }

    /// Types every character of `text`.
    pub(crate) fn type_text(&mut self, text: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        for c in text.chars() {
            actions.extend(self.char(c));
        }
        actions
    }

    /// Types `line` into the command box (focusing it first) and submits it.
    pub(crate) fn command(&mut self, line: &str) -> Vec<Action> {
        if !self.app.command_focused() {
            self.char('/');
        }
        let mut actions = self.type_text(line);
        actions.extend(self.press(KeyCode::Enter));
        actions
    }

    /// Plays each move through the command box, then gives the board focus back (a finished
    /// game has already left the box).
    pub(crate) fn moves(&mut self, moves: &[&str]) {
        for mv in moves {
            self.command(mv);
        }
        if self.app.command_has_focus() {
            self.press(KeyCode::Esc);
        }
    }

    /// Sends a mouse event at cell (`column`, `row`).
    pub(crate) fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) -> Vec<Action> {
        self.send(mouse(kind, column, row))
    }

    /// Presses and releases the left button at cell (`column`, `row`).
    pub(crate) fn click(&mut self, column: u16, row: u16) -> Vec<Action> {
        let mut actions = self.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        actions.extend(self.mouse(MouseEventKind::Up(MouseButton::Left), column, row));
        actions
    }

    /// The middle cell of `square` as last drawn.
    pub(crate) fn square_cell(&self, square: Square) -> (u16, u16) {
        let geometry = self.app.hit_map().board.expect("board drawn");
        let rect = square_rect(&geometry, square);
        (rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    /// Clicks the middle of `square`.
    pub(crate) fn click_square(&mut self, square: Square) -> Vec<Action> {
        let (column, row) = self.square_cell(square);
        self.click(column, row)
    }

    /// Drags from the middle of `from` to the middle of `to`.
    pub(crate) fn drag_square(&mut self, from: Square, to: Square) -> Vec<Action> {
        let (fx, fy) = self.square_cell(from);
        let (tx, ty) = self.square_cell(to);
        let mut actions = self.mouse(MouseEventKind::Down(MouseButton::Left), fx, fy);
        actions.extend(self.mouse(MouseEventKind::Drag(MouseButton::Left), tx, ty));
        actions.extend(self.mouse(MouseEventKind::Up(MouseButton::Left), tx, ty));
        actions
    }

    /// Clicks the top-left cell of `hit` as last drawn.
    pub(crate) fn click_hit(&mut self, hit: Hit) -> Vec<Action> {
        let rect = self
            .app
            .hit_map()
            .rect_of(hit)
            .unwrap_or_else(|| panic!("{hit:?} not drawn"));
        self.click(rect.x, rect.y)
    }

    /// From board focus with the keyboard cursor hidden and White's pawn on e7 (see
    /// [`PROMOTION_FEN`](super::PROMOTION_FEN)): shows the cursor on e2, walks it to e7,
    /// picks the pawn up and puts it down on e8, which opens the promotion picker.
    pub(crate) fn promote_e7_by_keyboard(&mut self) {
        for _ in 0..6 {
            self.press(KeyCode::Up);
        }
        self.press(KeyCode::Enter);
        self.press(KeyCode::Up);
        self.press(KeyCode::Enter);
    }

    /// Moves the clock to `ms` milliseconds after `t0`.
    pub(crate) fn at_ms(&mut self, ms: u64) {
        self.now = self.t0 + Duration::from_millis(ms);
    }

    /// Sends a `Tick`.
    pub(crate) fn tick(&mut self) -> Vec<Action> {
        self.send(AppEvent::Tick)
    }

    /// What the fake engine answers to `request`, computed on this thread.
    pub(crate) fn reply_for(&self, request: &EngineRequest) -> EngineReply {
        let outcome = match self.engine.choose(&request.game) {
            Some(computer) => EngineOutcome::Move(computer),
            None => EngineOutcome::GameOver,
        };
        EngineReply::new(request, outcome)
    }

    /// Answers `request` the way the fake engine would.
    pub(crate) fn respond(&mut self, request: &EngineRequest) -> Vec<Action> {
        let reply = self.reply_for(request);
        self.send(AppEvent::Engine(reply))
    }

    /// Answers `request` with `outcome`, as the worker would (a traced move's exchange
    /// travels beside it, see [`EngineReply::new`]).
    pub(crate) fn answer(
        &mut self,
        request: &EngineRequest,
        outcome: EngineOutcome,
    ) -> Vec<Action> {
        self.send(AppEvent::Engine(EngineReply::new(request, outcome)))
    }

    /// Answers the latest engine request with Jev playing `uci`.
    pub(crate) fn reply(&mut self, uci: &str) -> Vec<Action> {
        self.reply_with(uci, |_| {})
    }

    /// Answers the latest engine request with Jev playing `uci`, its exchange with Jev
    /// recorded ([`jev_exchange`]), as in debug mode.
    pub(crate) fn reply_traced(&mut self, uci: &str) -> Vec<Action> {
        self.reply_with(uci, |computer| {
            computer.exchange = Some(Box::new(jev_exchange()));
        })
    }

    /// Answers the latest engine request with Jev playing `uci`, after `adjust` changes how
    /// the move was chosen.
    pub(crate) fn reply_with(
        &mut self,
        uci: &str,
        adjust: impl FnOnce(&mut ComputerMove),
    ) -> Vec<Action> {
        let request = self.request.take().expect("an engine request");
        let mut computer = jev_move(request.game.position(), uci);
        adjust(&mut computer);
        self.answer(&request, EngineOutcome::Move(computer))
    }

    /// The game's moves in UCI.
    pub(crate) fn uci(&self) -> Vec<String> {
        uci_moves(self.app.game())
    }

    /// What the app drew last.
    pub(crate) fn buffer(&self) -> &Buffer {
        self.terminal.backend().buffer()
    }

    /// The screen's symbols, one line per row.
    pub(crate) fn screen(&self) -> String {
        buffer_text(self.buffer())
    }
}

/// The single engine request in `actions`.
pub(crate) fn request(actions: &[Action]) -> EngineRequest {
    match actions {
        [Action::RequestEngine(request)] => request.clone(),
        other => panic!("expected one engine request, got {other:?}"),
    }
}
