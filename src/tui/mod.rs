//! Terminal user interface (spec section 6): ratatui widgets, input
//! parsing, file export, the engine worker and terminal setup, built only on
//! `crate::core` and `crate::engine`.
//!
//! [`run`] is the whole program: it reads the command line and the environment,
//! builds the computer player, sets up the terminal, asks it about graphics
//! ([`graphics::detect`]) and runs the main loop until the user quits or a signal
//! asks it to stop. In debug mode ([`debug::enabled`]) the computer player records
//! its exchanges with Jev, and the app keeps them and writes the debug log.

pub mod app;
pub mod board;
pub mod debug;
pub mod event;
pub mod files;
pub mod glyphs;
pub mod graphics;
pub mod input;
pub mod movetext;
pub mod panels;
pub mod pieces;
pub mod terminal;
#[cfg(test)]
mod test_support;
pub mod worker;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::crossterm::event::{Event, KeyEventKind, MouseEventKind};

use crate::core::Game;
use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig, Provider};

use self::app::{Action, App};
use self::board::CellSize;
use self::debug::DebugLog;
use self::event::AppEvent;
use self::graphics::{FontMeter, Graphics, LateAnswers};
use self::worker::{Engine, EngineOutcome, EngineReply, EngineRequest};

/// How long one batch waits for terminal input before its `Tick` (spec 6.5).
const TICK: Duration = Duration::from_millis(50);

/// How long a UI that failed waits for a quit signal before returning its error. A
/// terminal that hangs up fails the next write at about the moment its SIGHUP
/// arrives, and the process should then end by that signal rather than by the error.
const SIGNAL_GRACE: Duration = Duration::from_millis(100);

/// The number of SIGHUP, with which a UI that failed on a closed terminal ends.
#[cfg(unix)]
const HANGUP: i32 = signal_hook::consts::SIGHUP;
/// SIGHUP's number on POSIX systems; unused in practice here, as stdin never looks
/// closed ([`terminal::stdin_looks_closed`]).
#[cfg(not(unix))]
const HANGUP: i32 = 1;

/// How long quitting waits for the debug log to write the exchanges still queued.
const LOG_GRACE: Duration = Duration::from_millis(500);

/// The `--help` text.
const USAGE: &str = concat!(
    "rchess: chess in the terminal, against a person or the Jev and Laya computer\n",
    "players\n",
    "\n",
    "Usage: ",
    env!("CARGO_PKG_NAME"),
    " [--glyphs image|solid|outline|ascii] [--debug]\n",
    "\n",
    "Options:\n",
    "  --glyphs <set>     how pieces look: image (pictures; the default when the\n",
    "                     terminal can show them), solid (otherwise the default),\n",
    "                     outline or ascii; `g` cycles them during a game\n",
    "  --debug            debug mode: keep every Jev and Laya request and answer,\n",
    "                     show them with `d` during a game and append them to the\n",
    "                     debug log\n",
    "  -h, --help         show this help and exit\n",
    "  -V, --version      show the version and exit\n",
    "\n",
    "Environment:\n",
    "  JEV_API_KEY        key for the Jev computer player (TYPESAFE_API_KEY also\n",
    "                     works); without one the computer uses local search\n",
    "  JEV_MODEL          Jev model (default jev-latest)\n",
    "  JEV_MAX_OPTIONS    moves offered to Jev per turn, 1-255 (default 40)\n",
    "  JEV_FILTER_LOSING  keep losing moves off Jev's shortlist (default true)\n",
    "  LAYA_URL           laya-serve endpoint for the Laya computer player, e.g.\n",
    "                     http://127.0.0.1:8000/v1/systemone; without it Laya is\n",
    "                     local search\n",
    "  LAYA_API_KEY       key laya-serve asks for, if any\n",
    "  LAYA_MODEL         model sent to laya-serve (default laya)\n",
    "  LAYA_MAX_OPTIONS   moves offered to Laya per turn, 1-255 (default 40)\n",
    "  LAYA_FILTER_LOSING keep losing moves off Laya's shortlist (default true)\n",
    "  RCHESS_GLYPHS      glyph set when --glyphs is not given\n",
    "  RCHESS_IMAGES      off, 0, false or no: no piece pictures, and no graphics\n",
    "                     query at start\n",
    "  RCHESS_DEBUG       debug mode as with --debug, unless empty or 0\n",
    "  RCHESS_DEBUG_LOG   the debug log file; the default is\n",
    "                     $XDG_STATE_HOME/rchess/jev-debug.jsonl, else\n",
    "                     ~/.local/state/rchess/jev-debug.jsonl\n",
    "  NO_COLOR           no colours: start with outline glyphs unless a set is\n",
    "                     chosen, mark board highlights with text, no pictures\n",
    "  COLORTERM          truecolor or 24bit selects 24-bit colours\n",
    "\n",
    "In the game press ? for help. Moves can be typed after / (e4, Nf3, e2e4).\n",
);

/// The `--version` text.
const VERSION: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"), "\n");

/// Writes `text` to `out`. A reader that has gone away (a closed pipe, as in
/// `chess --help | true`) is not an error: nobody is left to read it, as with other
/// command-line tools.
fn print_quietly(out: &mut impl Write, text: &str) -> io::Result<()> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

/// Runs the terminal UI until the user quits or SIGINT, SIGTERM or SIGHUP
/// arrives. `args` are the command-line arguments without the program name.
///
/// `--help` prints the usage and `--version` the version (`chess 0.1.0`), and both
/// return without touching the terminal; when stdout is a pipe that is already
/// closed (`chess --help | true`), they return quietly. Unknown arguments and a
/// missing or invalid `--glyphs` value do not stop the program: they are listed
/// as warnings on the menu, like invalid engine settings, and so is a graphics
/// query that failed. The computer players come from `EngineConfig::from_env()`
/// (Jev) and `EngineConfig::laya_from_env()` (Laya); with no `JEV_API_KEY` or
/// `LAYA_URL` that player plays by local search and never uses the network. In
/// debug mode (`--debug` or `RCHESS_DEBUG`) they record their exchanges with the
/// model (`EngineConfig::trace`).
///
/// Unless images are off ([`glyphs::images_wanted`]), the terminal is asked about
/// graphics right after it is set up, which takes up to [`graphics::QUERY_TIMEOUT`]
/// when it does not answer.
///
/// Call it from the main thread (the panic hook restores the terminal only for
/// a panic on the thread named "main"). The terminal is restored on every exit:
/// normal return, error return, panic, or signal. After a signal it does not
/// return: once the terminal is restored, the process ends by that signal
/// ([`terminal::exit_by_signal`]), so the shell sees it was interrupted. An error on a
/// terminal that closed without sending SIGHUP (stdin looks closed) ends the same way,
/// by SIGHUP.
///
/// # Errors
///
/// When stdin or stdout is not a terminal, when signal handlers cannot be installed, or
/// when the terminal cannot be set up, read or drawn. Writing the `--help` or
/// `--version` text can fail too (not for a closed pipe).
pub fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let mut options = match parse_args(args) {
        Cli::Help => return print_quietly(&mut io::stdout().lock(), USAGE),
        Cli::Version => return print_quietly(&mut io::stdout().lock(), VERSION),
        Cli::Play(options) => options,
    };
    // Before touching the terminal: without a terminal on stdin the key reader fails only
    // after the menu has been drawn on the alternate screen.
    if let Some(problem) = terminal_problem(io::stdin().is_terminal(), io::stdout().is_terminal()) {
        return Err(io::Error::other(problem));
    }

    let env = |name: &str| std::env::var(name).ok();
    let debug = debug::enabled(options.debug, env);
    options
        .warnings
        .extend(env_warnings(debug, |name| std::env::var_os(name)));
    let images = glyphs::images_wanted(options.glyphs.as_deref(), env);
    let fault = injected_fault(env("RCHESS_FAULT").as_deref());
    let jev_config = engine_config(EngineConfig::from_env(), debug);
    let laya_config = engine_config(EngineConfig::laya_from_env(), debug);
    let mut jev: Arc<dyn Engine> = Arc::new(ComputerPlayer::from_config(jev_config));
    let mut laya: Arc<dyn Engine> = Arc::new(ComputerPlayer::from_config(laya_config));
    if fault == Some(Fault::EnginePanic) {
        jev = Arc::new(PanickingEngine(jev));
        laya = Arc::new(PanickingEngine(laya));
    }

    // Before `enter`, so a signal that arrives during setup still ends in a
    // clean restore instead of killing the process with the terminal in raw mode.
    let quit = terminal::register_signals()?;
    let result = play(&quit, images, fault, |graphics| {
        build_app(options, jev, laya, graphics, env)
    });
    let signal = if result.is_err() {
        signal_after_error(&quit, SIGNAL_GRACE, terminal::stdin_looks_closed)
    } else {
        quit.load(Ordering::SeqCst)
    };
    if signal != 0 {
        // The terminal is restored; end the way the signal asked for.
        terminal::exit_by_signal(signal);
    }
    result
}

/// Menu warnings for the variables whose value is not valid UTF-8, which are read as
/// unset: `RCHESS_GLYPHS`, `RCHESS_IMAGES` and `NO_COLOR` (crossterm reads it the same
/// way, so colours stay on), and in debug mode the three that give the debug log's path
/// (`RCHESS_DEBUG_LOG`, `XDG_STATE_HOME`, `HOME`; saving still takes `HOME` as it is).
///
/// `get_os` reads an environment variable; pass `|k| std::env::var_os(k)`.
fn env_warnings(debug: bool, get_os: impl Fn(&str) -> Option<OsString>) -> Vec<String> {
    let mut checked = [glyphs::GLYPHS_ENV, glyphs::IMAGES_ENV, "NO_COLOR"]
        .map(|name| (name, "ignored"))
        .to_vec();
    if debug {
        checked.extend(
            [debug::DEBUG_LOG_ENV, "XDG_STATE_HOME", "HOME"]
                .map(|name| (name, "the debug log does not use it")),
        );
    }
    checked
        .into_iter()
        .filter(|(name, _)| get_os(name).is_some_and(|value| value.to_str().is_none()))
        .map(|(name, effect)| format!("{name} is not valid UTF-8; {effect}"))
        .collect()
}

/// `config` with the exchange recording debug mode needs (`trace`) on when `debug` is.
fn engine_config(config: EngineConfig, debug: bool) -> EngineConfig {
    EngineConfig {
        trace: debug,
        ..config
    }
}

/// Why the UI cannot run, given whether stdin and stdout are terminals: it reads keys
/// from one and draws on the other.
fn terminal_problem(stdin_is_terminal: bool, stdout_is_terminal: bool) -> Option<&'static str> {
    if !stdout_is_terminal {
        Some("stdout is not a terminal; run it in a terminal (see --help)")
    } else if !stdin_is_terminal {
        Some("stdin is not a terminal; run it in a terminal (see --help)")
    } else {
        None
    }
}

/// The app for the options and environment (`get`), once the terminal has been
/// asked about graphics: the starting glyph set follows [`Graphics::support`], and
/// the menu lists the command-line warnings, then the glyph warnings, then the
/// graphics query's (after the engine's own, which `App::new` adds). In debug mode
/// it starts the debug log at [`debug::log_path`].
fn build_app(
    options: Options,
    jev: Arc<dyn Engine>,
    laya: Arc<dyn Engine>,
    graphics: Graphics,
    get: impl Fn(&str) -> Option<String>,
) -> App {
    let (glyph_set, glyph_warnings) =
        glyphs::initial_glyphs(options.glyphs.as_deref(), &get, graphics.support());
    let mut warnings = options.warnings;
    warnings.extend(glyph_warnings);
    warnings.extend(graphics.warning);
    let mut app = App::new(
        jev,
        laya,
        glyph_set,
        glyphs::detect_truecolor(&get),
        warnings,
    )
    .with_no_color(glyphs::no_color(&get))
    .with_picker(graphics.picker);
    if debug::enabled(options.debug, &get) {
        app = app.with_debug(DebugLog::open(debug::log_path(&get)));
    }
    app.set_cell_size(graphics.cell_size);
    app
}

/// Sets up the terminal, asks it about graphics when `images` is true, builds the
/// app from the result, runs the main loop, gives the debug log a moment to finish,
/// and restores the terminal (also on an error) before returning.
fn play(
    quit: &AtomicI32,
    images: bool,
    fault: Option<Fault>,
    build: impl FnOnce(Graphics) -> App,
) -> io::Result<()> {
    let (screen, graphics) = terminal::enter(|| {
        if images {
            graphics::detect(|| quit.load(Ordering::SeqCst) != 0)
        } else {
            graphics::without_images()
        }
    })?;
    // Never dropped: `Terminal`'s `Drop` shows the cursor and prints when that
    // fails, which panics on a hung-up tty. `terminal::leave` shows the cursor
    // instead, ignoring errors. The process ends soon after, so nothing leaks for
    // long.
    let mut screen = ManuallyDrop::new(screen);
    let _guard = terminal::Guard;
    // A terminal that closes without sending SIGHUP still ends the program by it.
    terminal::watch_hangup()?;
    // A terminal that does not know a probe may have printed it, and the first draw
    // writes only the cells that are not blank. (`Terminal::clear` would also ask
    // for the cursor position, another answer to wait for.)
    screen.backend_mut().clear()?;
    let mut late = LateAnswers::after(&graphics, Instant::now());
    let mut meter = FontMeter::default();
    let mut app = build(graphics);
    let result = run_loop(
        &mut app,
        quit,
        |app| {
            // Kitty keeps pictures after the program ends unless they are deleted: the
            // board builds each with an id from `terminal::next_kitty_id`. Those dropped
            // by a font change (between frames) are deleted before the next frame, those
            // dropped by a new square size (found while drawing) right after it;
            // `terminal::leave` deletes the rest.
            terminal::delete_dropped_kitty_pictures(&mut io::stdout())?;
            screen.draw(|frame| app.render(frame, Instant::now()))?;
            terminal::delete_dropped_kitty_pictures(&mut io::stdout())?;
            Ok(())
        },
        |replies| {
            let mut batch = event::collect(replies, TICK)?;
            drop_late_answers(&mut batch, &mut late, Instant::now());
            if let Some(fault) = fault {
                inject_ui_fault(fault, &batch);
            }
            Ok(batch)
        },
        |app| {
            let is_tmux = app.picker().is_some_and(|picker| picker.tmux_detected());
            meter.measure(is_tmux, || quit.load(Ordering::SeqCst) != 0)
        },
    );
    app.close_debug_log(LOG_GRACE);
    result
}

/// Removes from `batch` the key presses of a graphics answer that came after the
/// query stopped waiting ([`LateAnswers`]), so they do not act as keys.
fn drop_late_answers(batch: &mut Vec<AppEvent>, late: &mut LateAnswers, now: Instant) {
    batch.retain(|event| match event {
        AppEvent::Term(term) => late.keep(term, now),
        _ => true,
    });
}

/// The quit signal recorded in `quit`, waiting up to `grace` for one to arrive; 0 if
/// none does.
fn signal_after(quit: &AtomicI32, grace: Duration) -> i32 {
    let deadline = Instant::now() + grace;
    loop {
        let signal = quit.load(Ordering::SeqCst);
        if signal != 0 || Instant::now() >= deadline {
            return signal;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// The signal a UI that failed should end by: the quit signal that arrives within
/// `grace` ([`signal_after`]), else SIGHUP when stdin is closed (`stdin_closed`, asked
/// only then), else 0 (return the error). A terminal can close without its SIGHUP
/// reaching the program; the "hangup" thread would raise one within its next look at
/// stdin, which can come just after `grace`, so a failure on a closed terminal is taken
/// as that hangup here rather than ending with status 1.
fn signal_after_error(
    quit: &AtomicI32,
    grace: Duration,
    stdin_closed: impl FnOnce() -> bool,
) -> i32 {
    match signal_after(quit, grace) {
        0 if stdin_closed() => HANGUP,
        signal => signal,
    }
}

/// A failure `tests/pty_smoke.py` injects into a debug build through
/// `RCHESS_FAULT`: a running binary has no other way into its panic and
/// stuck-loop paths. Release builds ignore the variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// `engine-panic`: every engine call panics, so the worker must answer with
    /// the local search move.
    EnginePanic,
    /// `ui-panic`: the UI thread panics at the first key press, so the panic hook
    /// must restore the terminal.
    UiPanic,
    /// `ui-hang`: the UI thread hangs at the first key press, so a quit signal
    /// must restore the terminal without it.
    UiHang,
}

/// The fault `RCHESS_FAULT` asks for; always `None` in release builds.
fn injected_fault(value: Option<&str>) -> Option<Fault> {
    if !cfg!(debug_assertions) {
        return None;
    }
    match value? {
        "engine-panic" => Some(Fault::EnginePanic),
        "ui-panic" => Some(Fault::UiPanic),
        "ui-hang" => Some(Fault::UiHang),
        _ => None,
    }
}

/// Triggers a UI fault once `batch` holds a key press.
fn inject_ui_fault(fault: Fault, batch: &[AppEvent]) {
    if !batch
        .iter()
        .any(|event| matches!(event, AppEvent::Term(Event::Key(_))))
    {
        return;
    }
    match fault {
        Fault::UiPanic => panic!("RCHESS_FAULT=ui-panic: injected panic on the UI thread"),
        Fault::UiHang => loop {
            thread::sleep(Duration::from_secs(60));
        },
        Fault::EnginePanic => {}
    }
}

/// An engine whose every move panics (`RCHESS_FAULT=engine-panic`).
struct PanickingEngine(Arc<dyn Engine>);

impl Engine for PanickingEngine {
    fn choose(&self, _game: &Game) -> Option<ComputerMove> {
        panic!("RCHESS_FAULT=engine-panic: injected engine panic")
    }

    fn status(&self) -> String {
        self.0.status()
    }

    fn provider(&self) -> Provider {
        self.0.provider()
    }

    fn enabled(&self) -> bool {
        self.0.enabled()
    }

    fn warnings(&self) -> Vec<String> {
        self.0.warnings()
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cli {
    /// `-h` or `--help` appeared (before any `-V` or `--version`): print the usage
    /// and exit.
    Help,
    /// `-V` or `--version` appeared (before any `-h` or `--help`): print the version
    /// and exit.
    Version,
    /// Start the UI.
    Play(Options),
}

/// Settings from the command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Options {
    /// The last `--glyphs` value, still unchecked: `glyphs::initial_glyphs`
    /// validates it and warns about a bad one.
    glyphs: Option<String>,
    /// `--debug` was given ([`debug::enabled`] also reads `RCHESS_DEBUG`).
    debug: bool,
    /// Notes about ignored arguments, shown on the menu.
    warnings: Vec<String>,
}

/// Reads `--glyphs <set>`, `--glyphs=<set>`, `--debug`, `-h`, `--help`, `-V` and
/// `--version`. The last `--glyphs` wins; the first of help and version wins over
/// everything. A `--glyphs` followed by nothing or by another option has no
/// value; that and any other argument become warnings rather than errors.
fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
    let mut options = Options::default();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Cli::Help,
            "-V" | "--version" => return Cli::Version,
            "--debug" => options.debug = true,
            "--glyphs" => match args.next_if(|value| !value.starts_with('-')) {
                Some(value) => options.glyphs = Some(value),
                None => options
                    .warnings
                    .push("--glyphs needs a value: image, solid, outline or ascii".to_string()),
            },
            _ => match arg.strip_prefix("--glyphs=") {
                Some(value) => options.glyphs = Some(value.to_string()),
                None => options.warnings.push(format!(
                    "ignored unknown argument {:?} (see --help)",
                    glyphs::shorten(&arg)
                )),
            },
        }
    }
    Cli::Play(options)
}

/// The main loop (spec 6.5): draw, collect a batch of events, hand each to the
/// app and carry out the actions it returns, until the app quits or `quit`
/// (the number of a quit signal, 0 until one arrives) is set. Both are checked
/// before every draw, so a signal is seen within one batch timeout.
///
/// Engine requests start at once. A font measurement ([`Action::MeasureFont`]) waits
/// for the end of the batch, so that it sees the terminal after every resize in it,
/// and runs once however many resizes asked: `measure_font` asks the terminal
/// ([`FontMeter::measure`] in production, up to [`graphics::QUERY_TIMEOUT`] when it
/// does not answer) and its result goes to
/// [`App::font_measured`] before the next draw. A draw inside the batch (below) that
/// finds a measurement asked for runs it first, so that no picture is encoded for the
/// old font.
///
/// Drawing comes before each batch so that mouse events are hit-tested against
/// what is on screen. An event that reads what the last draw found ([`reads_layout`]:
/// the mouse's hit map, and for keys and pastes too whether the terminal was too small
/// and where the board was) and follows, in the same batch, an event that may have
/// changed the screen or its layout ([`changes_layout`]) waits for another draw, so it
/// is handled against the new one; ticks and engine replies need no draw. Every batch
/// ends with a `Tick`, so the screen is redrawn at least once per batch (the spinner
/// and the Jev vs Jev delay depend on it); ratatui writes only the cells that changed.
/// Once the app quits, the rest of the batch is dropped.
///
/// `draw`, `next_batch` and `measure_font` are the terminal in production and a
/// `TestBackend` with scripted batches and font sizes in tests. Engine replies
/// travel over a channel created here: `next_batch` receives its end, the worker
/// threads send on the other.
///
/// # Errors
///
/// The first error from `draw` or `next_batch`.
fn run_loop(
    app: &mut App,
    quit: &AtomicI32,
    mut draw: impl FnMut(&mut App) -> io::Result<()>,
    mut next_batch: impl FnMut(&Receiver<EngineReply>) -> io::Result<Vec<AppEvent>>,
    mut measure_font: impl FnMut(&App) -> Option<CellSize>,
) -> io::Result<()> {
    let (replies_tx, replies) = mpsc::channel();
    while quit.load(Ordering::SeqCst) == 0 && !app.should_quit() {
        draw(app)?;
        let batch = next_batch(&replies)?;
        let now = Instant::now();
        let mut measure = false;
        let mut stale = false;
        for event in batch {
            if stale && reads_layout(&event) {
                if measure {
                    // The draw would encode pictures for the old font: measure first. A
                    // later resize in the batch asks again.
                    measure = false;
                    let measured = measure_font(app);
                    app.font_measured(measured);
                }
                draw(app)?;
                stale = false;
            }
            stale |= changes_layout(&event);
            for action in app.handle(event, now) {
                match action {
                    Action::RequestEngine(request) => {
                        let engine = app.engine(request.provider).clone();
                        request_engine(request, &engine, &replies_tx);
                    }
                    Action::MeasureFont => measure = true,
                }
            }
            if app.should_quit() {
                break;
            }
        }
        if measure && !app.should_quit() {
            let measured = measure_font(app);
            app.font_measured(measured);
        }
    }
    Ok(())
}

/// Whether handling `event` reads what the last draw found: the hit map (every mouse
/// event), or whether the terminal was too small and where the board was (a key press
/// or a paste, which the app ignores while the terminal is too small).
fn reads_layout(event: &AppEvent) -> bool {
    match event {
        AppEvent::Term(Event::Key(key)) => key.kind == KeyEventKind::Press,
        AppEvent::Term(Event::Mouse(_) | Event::Paste(_)) => true,
        AppEvent::Term(_) | AppEvent::Engine(_) | AppEvent::Tick => false,
    }
}

/// Whether `event` may change what is on screen and where: a key press, a click or
/// release, a paste, a resize or an engine reply may; a tick, a wheel turn (which
/// scrolls a list in place) and a drag or move of the pointer do not.
fn changes_layout(event: &AppEvent) -> bool {
    match event {
        AppEvent::Term(Event::Key(key)) => key.kind == KeyEventKind::Press,
        AppEvent::Term(Event::Mouse(mouse)) => {
            matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_))
        }
        AppEvent::Term(Event::Paste(_) | Event::Resize(..)) | AppEvent::Engine(_) => true,
        AppEvent::Term(_) | AppEvent::Tick => false,
    }
}

/// Runs `request` on an engine thread, whose reply arrives on `replies`.
fn request_engine(request: EngineRequest, engine: &Arc<dyn Engine>, replies: &Sender<EngineReply>) {
    request_engine_with(request, engine, replies, worker::spawn_request);
}

/// [`request_engine`] with the thread started by `spawn` ([`worker::spawn_request`] in
/// production). The thread is detached: its handle is dropped.
fn request_engine_with(
    request: EngineRequest,
    engine: &Arc<dyn Engine>,
    replies: &Sender<EngineReply>,
    spawn: impl FnOnce(
        Arc<dyn Engine>,
        EngineRequest,
        Sender<EngineReply>,
    ) -> io::Result<thread::JoinHandle<()>>,
) {
    let (generation, hash) = (request.generation, request.hash);
    if let Err(error) = spawn(Arc::clone(engine), request, replies.clone()) {
        // No thread means no reply. Answer for it, so the app stops
        // waiting: it shows the failure and lets the person retry.
        let failed = EngineReply {
            generation,
            hash,
            outcome: EngineOutcome::Failed(format!("cannot start the engine thread: {error}")),
            exchange: None,
        };
        // The receiver lives in `run_loop`, which is still running.
        let _ = replies.send(failed);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::convert::Infallible;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, MouseButton, MouseEventKind};

    use ratatui_image::picker::ProtocolType;

    use super::app::{Hit, Mode, Screen};
    use super::board::CellSize;
    use super::glyphs::{GlyphSet, ImageSupport};
    use super::graphics::{Graphics, LateAnswers, picker_for};
    use super::test_support::TempDir;
    use super::test_support::engine::{FakeEngine, REPLY_TIMEOUT, Turn, chars, key, mouse};
    use super::test_support::{late_kitty_answer, uci_moves};
    use super::*;
    use crate::core::Color as Side;
    use crate::engine::{JevClient, MoveSource};
    /// Stands in for the number of a quit signal.
    const SIGNAL: i32 = 15;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_string()).collect()
    }

    /// What `parse_args` returns for starting the UI with `glyphs` and `warnings`.
    fn cli_play(glyphs: Option<&str>, warnings: &[&str]) -> Cli {
        Cli::Play(Options {
            glyphs: glyphs.map(str::to_string),
            warnings: warnings.iter().map(|w| (*w).to_string()).collect(),
            ..Options::default()
        })
    }

    // ----- command line -----

    #[test]
    fn no_arguments_start_with_defaults() {
        assert_eq!(parse_args(args(&[])), cli_play(None, &[]));
    }

    #[test]
    fn help_wins_wherever_it_appears() {
        for list in [
            &["--help"][..],
            &["-h"],
            &["--glyphs", "ascii", "--help"],
            &["--bogus", "-h"],
            &["--glyphs", "--help"],
        ] {
            assert_eq!(parse_args(args(list)), Cli::Help, "{list:?}");
        }
    }

    #[test]
    fn glyphs_takes_a_separate_or_attached_value_and_the_last_wins() {
        assert_eq!(
            parse_args(args(&["--glyphs", "ascii"])),
            cli_play(Some("ascii"), &[])
        );
        assert_eq!(
            parse_args(args(&["--glyphs=outline"])),
            cli_play(Some("outline"), &[])
        );
        assert_eq!(
            parse_args(args(&["--glyphs", "ascii", "--glyphs=solid"])),
            cli_play(Some("solid"), &[])
        );
    }

    #[test]
    fn glyph_values_are_passed_on_unchecked() {
        // `glyphs::initial_glyphs` validates the value and words the warning.
        assert_eq!(
            parse_args(args(&["--glyphs", "fancy"])),
            cli_play(Some("fancy"), &[])
        );
        let Cli::Play(options) = parse_args(args(&["--glyphs", "fancy"])) else {
            panic!("expected Play");
        };
        let (set, warnings) =
            glyphs::initial_glyphs(options.glyphs.as_deref(), |_| None, ImageSupport::Off);
        assert_eq!(set, GlyphSet::Solid);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn the_image_style_can_be_asked_for() {
        assert_eq!(
            parse_args(args(&["--glyphs", "image"])),
            cli_play(Some("image"), &[])
        );
    }

    #[test]
    fn debug_mode_can_be_asked_for() {
        let debug = |list: &[&str]| match parse_args(args(list)) {
            Cli::Play(options) => options,
            other => panic!("expected Play, got {other:?}"),
        };
        assert!(!debug(&[]).debug);
        assert_eq!(
            debug(&["--debug", "--glyphs", "ascii"]),
            Options {
                glyphs: Some("ascii".to_string()),
                debug: true,
                warnings: Vec::new(),
            }
        );
        assert_eq!(
            debug(&["--glyphs", "--debug"]),
            Options {
                glyphs: None,
                debug: true,
                warnings: vec!["--glyphs needs a value: image, solid, outline or ascii".into()],
            },
            "--debug is not a glyph set"
        );
        assert_eq!(
            debug(&["--debug=1"]).warnings,
            [r#"ignored unknown argument "--debug=1" (see --help)"#]
        );
    }

    #[test]
    fn debug_mode_turns_the_engine_trace_on() {
        assert!(engine_config(EngineConfig::default(), true).trace);
        let config = EngineConfig {
            model: "jev-x".to_string(),
            trace: true,
            ..EngineConfig::default()
        };
        let config = engine_config(config, false);
        assert!(!config.trace);
        assert_eq!(config.model, "jev-x", "nothing else changes");
    }

    #[test]
    fn a_missing_glyphs_value_is_a_warning() {
        let missing = "--glyphs needs a value: image, solid, outline or ascii";
        assert_eq!(parse_args(args(&["--glyphs"])), cli_play(None, &[missing]));
        assert_eq!(
            parse_args(args(&["--glyphs", "--glyphs=ascii"])),
            cli_play(Some("ascii"), &[missing])
        );
    }

    #[test]
    fn unknown_arguments_are_warnings_in_order() {
        assert_eq!(
            parse_args(args(&["ascii", "--glyph", ""])),
            cli_play(
                None,
                &[
                    r#"ignored unknown argument "ascii" (see --help)"#,
                    r#"ignored unknown argument "--glyph" (see --help)"#,
                    r#"ignored unknown argument "" (see --help)"#,
                ]
            )
        );
    }

    #[test]
    fn echoed_arguments_are_shortened_and_escaped() {
        let long = "x".repeat(30);
        let Cli::Play(options) = parse_args(vec![long, "\u{1b}[2J".to_string()]) else {
            panic!("expected Play");
        };
        assert_eq!(
            options.warnings,
            [
                format!(
                    "ignored unknown argument {:?} (see --help)",
                    format!("{}…", "x".repeat(24))
                ),
                r#"ignored unknown argument "\u{1b}[2J" (see --help)"#.to_string(),
            ]
        );
    }

    #[test]
    fn usage_names_every_option_and_variable() {
        for name in [
            "--glyphs",
            "--debug",
            "--help",
            "--version",
            "-V",
            "JEV_API_KEY",
            "TYPESAFE_API_KEY",
            "JEV_MODEL",
            "JEV_MAX_OPTIONS",
            "JEV_FILTER_LOSING",
            "LAYA_URL",
            "LAYA_API_KEY",
            "LAYA_MODEL",
            "LAYA_MAX_OPTIONS",
            "LAYA_FILTER_LOSING",
            "RCHESS_GLYPHS",
            "RCHESS_IMAGES",
            "RCHESS_DEBUG",
            "RCHESS_DEBUG_LOG",
            "NO_COLOR",
            "COLORTERM",
        ] {
            assert!(USAGE.contains(name), "{name}");
        }
        assert!(
            USAGE.contains("off, 0, false or no"),
            "RCHESS_IMAGES values"
        );
        assert!(USAGE.contains("Usage: chess "));
        assert!(USAGE.contains("[--glyphs image|solid|outline|ascii] [--debug]"));
        assert!(USAGE.lines().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn both_stdin_and_stdout_must_be_terminals() {
        assert_eq!(terminal_problem(true, true), None);
        assert_eq!(
            terminal_problem(true, false),
            Some("stdout is not a terminal; run it in a terminal (see --help)")
        );
        assert_eq!(
            terminal_problem(false, true),
            Some("stdin is not a terminal; run it in a terminal (see --help)")
        );
        assert!(terminal_problem(false, false).is_some());
    }

    // ----- start-up -----

    #[cfg(unix)]
    #[test]
    fn variables_that_are_not_utf_8_are_named_in_warnings() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let bad = || OsString::from_vec(b"/home/\xffana".to_vec());
        let all_bad = |_: &str| Some(bad());
        assert_eq!(
            env_warnings(true, all_bad),
            [
                "RCHESS_GLYPHS is not valid UTF-8; ignored",
                "RCHESS_IMAGES is not valid UTF-8; ignored",
                "NO_COLOR is not valid UTF-8; ignored",
                "RCHESS_DEBUG_LOG is not valid UTF-8; the debug log does not use it",
                "XDG_STATE_HOME is not valid UTF-8; the debug log does not use it",
                "HOME is not valid UTF-8; the debug log does not use it",
            ]
        );
        assert_eq!(
            env_warnings(false, all_bad),
            [
                "RCHESS_GLYPHS is not valid UTF-8; ignored",
                "RCHESS_IMAGES is not valid UTF-8; ignored",
                "NO_COLOR is not valid UTF-8; ignored",
            ],
            "the log's variables only matter in debug mode"
        );
        let fine = |name: &str| (name != "RCHESS_IMAGES").then(|| OsString::from("/home/ana"));
        assert!(env_warnings(true, fine).is_empty());
        assert!(env_warnings(true, |_| None).is_empty());
        let only_home = |name: &str| (name == "HOME").then(bad);
        assert_eq!(
            env_warnings(true, only_home),
            ["HOME is not valid UTF-8; the debug log does not use it"]
        );
        // NO_COLOR read as unset leaves the colours on, so it is named too.
        let only_no_color = |name: &str| (name == "NO_COLOR").then(bad);
        assert_eq!(
            env_warnings(false, only_no_color),
            ["NO_COLOR is not valid UTF-8; ignored"]
        );
    }

    fn options(glyphs: Option<&str>, warnings: &[&str]) -> Options {
        let Cli::Play(options) = cli_play(glyphs, warnings) else {
            unreachable!("cli_play() builds Cli::Play");
        };
        options
    }

    #[test]
    fn a_graphics_protocol_starts_the_app_with_piece_images() {
        let cell = CellSize::new(9, 18);
        let graphics = Graphics {
            picker: Some(picker_for(ProtocolType::Kitty, cell)),
            cell_size: cell,
            warning: None,
            answers_pending: false,
        };
        let app = build_app(
            options(None, &[]),
            local_engine(),
            local_laya_engine(),
            graphics,
            |_| None,
        );
        assert_eq!(app.glyphs(), GlyphSet::Image);
        assert!(app.images_available());
        assert_eq!(
            app.picker().map(|picker| picker.protocol_type()),
            Some(ProtocolType::Kitty)
        );
        assert_eq!(app.cell_size(), cell);
        assert!(app.warnings().is_empty(), "{:?}", app.warnings());
    }

    #[test]
    fn a_failed_query_starts_solid_and_warns_after_the_other_notes() {
        let warning = "graphics query: no answer within 1 s; images use half-blocks";
        let graphics = Graphics {
            picker: Some(picker_for(ProtocolType::Halfblocks, CellSize::DEFAULT)),
            cell_size: CellSize::DEFAULT,
            warning: Some(warning.to_string()),
            answers_pending: true,
        };
        let unknown = r#"ignored unknown argument "--frob" (see --help)"#;
        let app = build_app(
            options(Some("fancy"), &[unknown]),
            local_engine(),
            local_laya_engine(),
            graphics,
            |_| None,
        );
        assert_eq!(app.glyphs(), GlyphSet::Solid);
        assert!(app.images_available(), "Image stays in the cycle");
        assert_eq!(
            app.warnings(),
            [
                unknown,
                "--glyphs: unknown glyph set \"fancy\" (expected image, solid, outline or \
                 ascii); using solid",
                warning,
            ]
        );
    }

    #[test]
    fn without_images_the_app_has_no_picker() {
        let get = |name: &str| (name == "NO_COLOR").then(|| "1".to_string());
        let cell = CellSize::new(8, 16);
        let app = build_app(
            options(None, &[]),
            local_engine(),
            local_laya_engine(),
            Graphics::off(cell),
            get,
        );
        assert_eq!(app.glyphs(), GlyphSet::Outline);
        assert!(!app.images_available());
        assert!(app.picker().is_none());
        assert!(app.no_color());
        assert_eq!(
            app.cell_size(),
            cell,
            "the font size still shapes the squares"
        );
    }

    #[test]
    fn debug_mode_starts_the_log_where_the_environment_says() {
        let dir = TempDir::new("debug-start");
        let path = dir.join("log").join("jev.jsonl");
        let log = path.display().to_string();
        let vars = |debug: &str| {
            let (debug, log) = (debug.to_string(), log.clone());
            move |name: &str| match name {
                "RCHESS_DEBUG" => Some(debug.clone()),
                "RCHESS_DEBUG_LOG" => Some(log.clone()),
                _ => None,
            }
        };
        let off = Graphics::off(CellSize::DEFAULT);
        let app = build_app(
            options(None, &[]),
            local_engine(),
            local_laya_engine(),
            off.clone(),
            vars("0"),
        );
        assert!(!app.debug_mode());
        let mut app = build_app(
            options(None, &[]),
            local_engine(),
            local_laya_engine(),
            off.clone(),
            vars("1"),
        );
        assert!(app.debug_mode());
        assert!(app.exchanges().is_some_and(debug::History::is_empty));
        app.close_debug_log(Duration::from_secs(10));
        assert!(!path.exists(), "no exchange, no file");
        let flagged = Options {
            debug: true,
            ..Options::default()
        };
        let app = build_app(flagged, local_engine(), local_laya_engine(), off, vars(""));
        assert!(app.debug_mode(), "--debug alone");
    }

    // ----- main loop -----

    /// One scripted `next_batch` result.
    enum Step {
        /// These events, then a `Tick`, as `event::collect` would return them.
        Events(Vec<AppEvent>),
        /// Blocks until an engine reply arrives, then returns it and a `Tick`.
        AwaitEngine,
        /// Raises the quit flag as a signal handler would, then returns a `Tick`.
        Signal,
    }

    /// An offline player: no Jev client, so every move comes from local search.
    fn local_engine() -> Arc<dyn Engine> {
        Arc::new(ComputerPlayer::<JevClient>::new(
            None,
            EngineConfig::default(),
        ))
    }

    /// An offline Laya player: no `LAYA_URL`, so every move comes from local search.
    fn local_laya_engine() -> Arc<dyn Engine> {
        Arc::new(ComputerPlayer::<JevClient>::new(
            None,
            EngineConfig::laya_from_vars(|_| None),
        ))
    }

    fn new_app() -> App {
        App::new(
            local_engine(),
            local_laya_engine(),
            GlyphSet::Solid,
            true,
            Vec::new(),
        )
        .with_home(None)
    }

    /// What a scripted run of [`run_loop`] did.
    struct Run {
        result: io::Result<()>,
        draws: usize,
        unused_steps: usize,
        /// [`App::in_flight`] at every draw, so before every batch.
        in_flight: Vec<usize>,
        /// The draw each font measurement came after (1 for the first draw).
        measured_after: Vec<usize>,
    }

    /// Runs [`run_loop`] on an 80×24 `TestBackend`, serving `steps` as batches. Running
    /// out of steps before the loop ends is an error, so a loop that fails to quit fails
    /// the test instead of hanging it. Font measurements find nothing.
    fn drive(app: &mut App, quit: &AtomicI32, steps: Vec<Step>) -> Run {
        drive_with_font(app, quit, steps, None)
    }

    /// [`drive`], with every font measurement giving `font`.
    fn drive_with_font(
        app: &mut App,
        quit: &AtomicI32,
        steps: Vec<Step>,
        font: Option<CellSize>,
    ) -> Run {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let mut steps = VecDeque::from(steps);
        let draws = std::cell::Cell::new(0);
        let mut in_flight = Vec::new();
        let mut measured_after = Vec::new();
        let result = run_loop(
            app,
            quit,
            |app| {
                draws.set(draws.get() + 1);
                in_flight.push(app.in_flight());
                terminal
                    .draw(|frame| app.render(frame, Instant::now()))
                    .map(drop)
                    .map_err(|never: Infallible| match never {})
            },
            |replies| {
                let mut batch = match steps.pop_front() {
                    Some(Step::Events(events)) => events,
                    Some(Step::AwaitEngine) => match replies.recv_timeout(REPLY_TIMEOUT) {
                        Ok(reply) => vec![AppEvent::Engine(reply)],
                        Err(error) => return Err(io::Error::other(error)),
                    },
                    Some(Step::Signal) => {
                        quit.store(SIGNAL, Ordering::SeqCst);
                        Vec::new()
                    }
                    None => return Err(io::Error::other("script ended before the loop")),
                };
                batch.push(AppEvent::Tick);
                Ok(batch)
            },
            |_| {
                measured_after.push(draws.get());
                font
            },
        );
        Run {
            result,
            draws: draws.get(),
            unused_steps: steps.len(),
            in_flight,
            measured_after,
        }
    }

    /// A game on an app whose picker draws with `protocol` at 10×20.
    fn picture_game(protocol: ProtocolType) -> App {
        let mut app = new_app().with_picker(Some(picker_for(protocol, CellSize::DEFAULT)));
        app.set_cell_size(CellSize::DEFAULT);
        let _ = app.handle(key(KeyCode::Char('1')), Instant::now());
        app
    }

    fn resizes(sizes: &[(u16, u16)]) -> Step {
        Step::Events(
            sizes
                .iter()
                .map(|&(columns, rows)| AppEvent::Term(Event::Resize(columns, rows)))
                .collect(),
        )
    }

    #[test]
    fn a_batch_of_resizes_measures_the_font_once_after_the_batch() {
        let mut app = picture_game(ProtocolType::Sixel);
        let run = drive_with_font(
            &mut app,
            &AtomicI32::new(0),
            vec![
                resizes(&[(100, 30), (90, 26), (100, 30)]),
                Step::Events(Vec::new()),
                resizes(&[(80, 24)]),
                Step::Signal,
            ],
            Some(CellSize::new(8, 16)),
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(run.measured_after, [1, 3], "once per batch of resizes");
        assert_eq!(app.cell_size(), CellSize::new(8, 16));
        let font = app.picker().map(|picker| picker.font_size());
        assert_eq!(font.map(|font| (font.width, font.height)), Some((8, 16)));

        // Nothing measured keeps the font.
        let mut app = picture_game(ProtocolType::Iterm2);
        let run = drive(
            &mut app,
            &AtomicI32::new(0),
            vec![resizes(&[(100, 30)]), Step::Signal],
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(run.measured_after, [1]);
        assert_eq!(app.cell_size(), CellSize::DEFAULT);
    }

    #[test]
    fn a_draw_inside_a_batch_measures_the_font_first() {
        // A key after a resize waits for a redraw (it may read the new layout), and that
        // draw must not encode pictures for the old font: the measurement the resize
        // asked for comes before it.
        let mut app = picture_game(ProtocolType::Sixel);
        let run = drive_with_font(
            &mut app,
            &AtomicI32::new(0),
            vec![
                Step::Events(vec![
                    AppEvent::Term(Event::Resize(100, 30)),
                    key(KeyCode::Char('f')),
                ]),
                Step::Signal,
            ],
            Some(CellSize::new(8, 16)),
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(run.draws, 3, "before each batch, and before the key");
        assert_eq!(run.measured_after, [1], "before the draw for the key");
        assert_eq!(app.cell_size(), CellSize::new(8, 16));
        assert!(app.flipped(), "the key was handled");
    }

    #[test]
    fn kitty_pictures_follow_a_font_zoom() {
        // Kitty and Ghostty size a placeholder picture from its pixel size and the
        // current cell size, so a picture made for the old font would be cropped or
        // shrunk after a zoom.
        let mut app = picture_game(ProtocolType::Kitty);
        let run = drive_with_font(
            &mut app,
            &AtomicI32::new(0),
            vec![resizes(&[(100, 30), (80, 24)]), Step::Signal],
            Some(CellSize::new(8, 16)),
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(run.measured_after, [1]);
        assert_eq!(app.cell_size(), CellSize::new(8, 16));
        let picker = app.picker().expect("images stay on");
        assert_eq!(picker.protocol_type(), ProtocolType::Kitty);
        let font = picker.font_size();
        assert_eq!((font.width, font.height), (8, 16));
    }

    #[test]
    fn half_block_pictures_are_never_measured() {
        let mut app = picture_game(ProtocolType::Halfblocks);
        let run = drive_with_font(
            &mut app,
            &AtomicI32::new(0),
            vec![resizes(&[(100, 30), (80, 24)]), Step::Signal],
            Some(CellSize::new(8, 16)),
        );
        run.result.expect("loop ends cleanly");
        assert!(run.measured_after.is_empty());
        assert_eq!(app.cell_size(), CellSize::DEFAULT);
    }

    #[test]
    fn a_late_graphics_answer_does_not_start_a_game_from_the_menu() {
        let answer: Vec<AppEvent> = late_kitty_answer()
            .into_iter()
            .map(AppEvent::Term)
            .collect();

        // Typed as keys, the `3` in the answer starts Human vs Jev as Black.
        let mut app = new_app();
        let run = drive(
            &mut app,
            &AtomicI32::new(0),
            vec![Step::Events(answer.clone()), Step::Signal],
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(
            app.mode(),
            Mode::HumanVsComputer {
                human: Side::Black,
                computer: Provider::Jev
            }
        );

        let graphics = Graphics {
            answers_pending: true,
            ..Graphics::off(CellSize::DEFAULT)
        };
        let now = Instant::now();
        let mut late = LateAnswers::after(&graphics, now);
        let mut batch = answer;
        drop_late_answers(&mut batch, &mut late, now);
        let mut app = new_app();
        let run = drive(
            &mut app,
            &AtomicI32::new(0),
            vec![Step::Events(batch), Step::Events(chars("q"))],
        );
        run.result.expect("q on the menu quits at once");
        assert_eq!(run.unused_steps, 0);
        assert!(app.should_quit());
        assert_eq!(app.screen_name(), "menu");
        assert!(uci_moves(app.game()).is_empty());
    }

    #[test]
    fn a_traced_move_reaches_the_exchange_view_and_the_log() {
        // The worker thread moves the fake engine's exchange into its reply; the app keeps
        // it for `d` and the log thread appends it.
        let dir = TempDir::new("debug-loop");
        let path = dir.join("jev.jsonl");
        let engine = Arc::new(FakeEngine::jev().scripted([Turn::Traced("e2e4")]));
        let mut app = App::new(
            engine,
            local_laya_engine(),
            GlyphSet::Solid,
            true,
            Vec::new(),
        )
        .with_home(None)
        .with_debug(DebugLog::start(path.clone()));
        let quit = AtomicI32::new(0);
        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("3")),
                Step::AwaitEngine,
                Step::Events(chars("d")),
                Step::Events(vec![key(KeyCode::End)]),
                Step::Signal,
            ],
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(uci_moves(app.game()), ["e2e4"]);
        let view = app.exchange_view().expect("the view is open");
        assert_eq!(view.shown, Some(1));
        assert_eq!(view.scroll, view.max_scroll);
        assert!(view.max_scroll > 0);
        app.close_debug_log(Duration::from_secs(10));
        let log = std::fs::read_to_string(&path).expect("log written");
        let line: serde_json::Value = serde_json::from_str(log.trim_end()).expect("one line");
        assert_eq!(line["played"], "e4");
        assert_eq!(line["ply"], 0);
        assert_eq!(line["stale"], false);
    }

    #[test]
    fn debug_mode_from_the_environment_logs_an_exchange_privately() {
        // What `run` does with `RCHESS_DEBUG=1`: `build_app` finds the log path in the
        // environment, and the first traced move creates the folders and the file. The
        // binary cannot make a Jev request offline, so a fake engine records one.
        let dir = TempDir::new("debug-env");
        let (state, home) = (dir.join("state"), dir.join("home"));
        let explicit = state.join("explicit").join("jev.jsonl");
        let default = state.join("rchess").join("jev-debug.jsonl");
        for (log_var, expected) in [(Some(&explicit), &explicit), (None, &default)] {
            let mut vars = vec![
                ("RCHESS_DEBUG", "1".to_string()),
                ("XDG_STATE_HOME", state.display().to_string()),
                ("HOME", home.display().to_string()),
            ];
            if let Some(path) = log_var {
                vars.push(("RCHESS_DEBUG_LOG", path.display().to_string()));
            }
            let get = |name: &str| {
                vars.iter()
                    .find(|(var, _)| *var == name)
                    .map(|(_, value)| value.clone())
            };
            let engine = Arc::new(FakeEngine::jev().scripted([Turn::Traced("e2e4")]));
            let mut app = build_app(
                Options::default(),
                engine,
                local_laya_engine(),
                Graphics::off(CellSize::DEFAULT),
                get,
            );
            assert!(app.debug_mode(), "RCHESS_DEBUG=1 without --debug");
            let run = drive(
                &mut app,
                &AtomicI32::new(0),
                vec![Step::Events(chars("3")), Step::AwaitEngine, Step::Signal],
            );
            run.result.expect("loop ends cleanly");
            assert_eq!(uci_moves(app.game()), ["e2e4"]);
            app.close_debug_log(Duration::from_secs(10));

            let log = std::fs::read_to_string(expected).expect("log written");
            let lines: Vec<&str> = log.lines().collect();
            assert_eq!(lines.len(), 1, "{log}");
            let line: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON line");
            assert_eq!(line["played"], "e4");
            assert_eq!(line["stale"], false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = |path: &std::path::Path| {
                    std::fs::metadata(path)
                        .expect("exists")
                        .permissions()
                        .mode()
                        & 0o777
                };
                assert_eq!(mode(expected), 0o600, "{}", expected.display());
                assert_eq!(mode(expected.parent().expect("folder")), 0o700);
                assert_eq!(mode(&state), 0o700, "missing folders are made private");
            }
        }
        assert!(!home.exists(), "HOME is the last resort");
    }

    #[test]
    fn a_human_vs_human_session_plays_e4_and_quits_after_confirmation() {
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let mut command = vec![key(KeyCode::Char('/'))];
        command.extend(chars("e4"));
        command.push(key(KeyCode::Enter));

        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("1")),
                Step::Events(command),
                Step::Events(vec![key(KeyCode::Esc)]),
                Step::Events(chars("q")),
                Step::Events(chars("y")),
            ],
        );

        run.result.expect("loop ends cleanly");
        assert_eq!(run.unused_steps, 0);
        // One draw before every batch, and one before each key that follows another key
        // in its batch (`e`, `4` and Enter after `/`).
        assert_eq!(run.draws, 8);
        assert!(app.should_quit());
        assert_eq!(app.mode(), Mode::HumanVsHuman);
        assert_eq!(uci_moves(app.game()), ["e2e4"]);
        assert_eq!(app.game().position().side_to_move(), Side::Black);
    }

    #[test]
    fn a_click_on_the_first_frame_hits_the_menu() {
        // The loop draws before it reads input, so the very first click is hit-tested
        // against a menu that is on screen. Find where the first item is drawn.
        let mut probe = new_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        terminal
            .draw(|frame| probe.render(frame, Instant::now()))
            .expect("probe draw");
        let item = probe
            .hit_map()
            .rect_of(Hit::MenuItem(0))
            .expect("the menu is drawn at 80x24");

        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let (x, y) = (item.x + 1, item.y);
        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(vec![
                    mouse(MouseEventKind::Down(MouseButton::Left), x, y),
                    mouse(MouseEventKind::Up(MouseButton::Left), x, y),
                ]),
                Step::Signal,
            ],
        );

        run.result.expect("loop ends cleanly");
        assert_eq!(app.screen(), Screen::Playing);
        assert_eq!(app.mode(), Mode::HumanVsHuman);
    }

    #[test]
    fn a_click_after_a_key_in_the_same_batch_hits_the_new_layout() {
        // `1` leaves the menu for the board; clicks that came in with it must be
        // hit-tested against the board, not against the menu drawn before the batch.
        let mut probe = new_app();
        let _ = probe.handle(key(KeyCode::Char('1')), Instant::now());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        terminal
            .draw(|frame| probe.render(frame, Instant::now()))
            .expect("probe draw");
        let board = probe.hit_map().board.expect("the board is drawn at 80x24");
        let middle = |square: &str| {
            let rect = board::square_rect(&board, square.parse().expect("square"));
            (rect.x + rect.width / 2, rect.y + rect.height / 2)
        };
        let (e2, e4) = (middle("e2"), middle("e4"));
        let click = |(x, y): (u16, u16)| {
            [
                mouse(MouseEventKind::Down(MouseButton::Left), x, y),
                mouse(MouseEventKind::Up(MouseButton::Left), x, y),
            ]
        };
        let mut batch = chars("1");
        batch.extend(click(e2));
        batch.extend(click(e4));

        let mut app = new_app();
        let run = drive(
            &mut app,
            &AtomicI32::new(0),
            vec![Step::Events(batch), Step::Signal],
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(app.mode(), Mode::HumanVsHuman);
        assert_eq!(uci_moves(app.game()), ["e2e4"]);
        // The draw before the batch, one before each mouse event (the key changed the
        // screen, and each press or release may), and the one before the next batch.
        assert_eq!(run.draws, 6);
    }

    #[test]
    fn a_key_after_a_resize_in_the_same_batch_sees_the_new_size() {
        // The terminal is too small at the first draw, then grows; the resize and the key
        // come in one batch. The key must be handled as the new size allows, not ignored
        // as the draw before the batch left it.
        let mut app = new_app();
        let mut terminal = Terminal::new(TestBackend::new(50, 15)).expect("test terminal");
        let mut draws = 0;
        let mut batches = VecDeque::from([
            vec![
                AppEvent::Term(Event::Resize(80, 24)),
                key(KeyCode::Char('1')),
            ],
            vec![
                AppEvent::Term(Event::Resize(50, 15)),
                key(KeyCode::Char('f')),
            ],
        ]);
        let result = run_loop(
            &mut app,
            &AtomicI32::new(0),
            |app| {
                draws += 1;
                if draws == 2 {
                    terminal.backend_mut().resize(80, 24);
                } else if draws == 4 {
                    terminal.backend_mut().resize(50, 15);
                }
                terminal
                    .draw(|frame| app.render(frame, Instant::now()))
                    .map(drop)
                    .map_err(|never: Infallible| match never {})
            },
            |_| match batches.pop_front() {
                Some(batch) => Ok(batch),
                None => Err(io::Error::other("script ended")),
            },
            |_| None,
        );
        assert_eq!(
            result.expect_err("script ended").to_string(),
            "script ended"
        );
        assert_eq!(app.screen(), Screen::Playing, "1 started a game at 80x24");
        assert!(!app.flipped(), "f was ignored at 50x15");
        // Before each batch, and before each key after its resize.
        assert_eq!(draws, 5);
    }

    #[test]
    fn computer_moves_arrive_through_the_worker_thread() {
        // Human vs Jev as Black: the computer (local search here, never the network)
        // moves first, on an "engine" thread, and its reply comes back over the channel.
        let mut app = new_app();
        let quit = AtomicI32::new(0);

        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("3")),
                Step::AwaitEngine,
                Step::Events(chars("q")),
                Step::Events(chars("y")),
            ],
        );

        run.result.expect("loop ends cleanly");
        assert_eq!(run.unused_steps, 0);
        assert_eq!(
            app.mode(),
            Mode::HumanVsComputer {
                human: Side::Black,
                computer: Provider::Jev
            }
        );
        assert!(app.flipped(), "playing Black starts flipped");
        assert_eq!(
            app.game().moves().len(),
            1,
            "the computer played White's move"
        );
        assert_eq!(app.game().position().side_to_move(), Side::Black);
        let computer = app
            .last_computer()
            .expect("the move is shown in the Jev panel");
        assert_eq!(computer.source, MoveSource::Fallback);
        assert!(!app.is_thinking());
    }

    #[test]
    fn a_signal_ends_the_loop_before_the_next_draw() {
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let mut command = vec![key(KeyCode::Char('/'))];
        command.extend(chars("e4"));
        command.push(key(KeyCode::Enter));

        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("1")),
                Step::Signal,
                Step::Events(command),
            ],
        );

        run.result.expect("a signal is a normal exit");
        assert_eq!(run.draws, 2);
        assert_eq!(run.unused_steps, 1, "nothing is read after the signal");
        assert!(app.game().moves().is_empty());
        assert!(!app.should_quit(), "the app itself was not asked to quit");
    }

    #[test]
    fn a_signal_before_the_first_draw_returns_at_once() {
        let mut app = new_app();
        let quit = AtomicI32::new(SIGNAL);
        let run = drive(&mut app, &quit, vec![Step::Events(chars("1"))]);
        run.result.expect("a signal is a normal exit");
        assert_eq!(run.draws, 0);
        assert_eq!(run.unused_steps, 1);
    }

    #[test]
    fn events_after_quitting_are_dropped() {
        // On the menu `q` quits without asking; the `1` behind it in the same batch
        // must not start a game (or an engine request) on the way out.
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let run = drive(&mut app, &quit, vec![Step::Events(chars("q1"))]);
        run.result.expect("loop ends cleanly");
        assert!(app.should_quit());
        assert_eq!(app.screen(), Screen::Menu);
    }

    #[test]
    fn draw_errors_end_the_loop() {
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let result = run_loop(
            &mut app,
            &quit,
            |_| Err(io::Error::other("tty gone")),
            |_| panic!("no batch after a failed draw"),
            |_| panic!("no font measurement"),
        );
        assert_eq!(result.expect_err("draw failed").to_string(), "tty gone");
    }

    #[test]
    fn event_errors_end_the_loop() {
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let mut draws = 0;
        let result = run_loop(
            &mut app,
            &quit,
            |_| {
                draws += 1;
                Ok(())
            },
            |_| Err(io::Error::other("read failed")),
            |_| panic!("no font measurement"),
        );
        assert_eq!(result.expect_err("read failed").to_string(), "read failed");
        assert_eq!(draws, 1);
    }

    #[test]
    fn an_error_exit_waits_briefly_for_the_signal_behind_it() {
        // A hangup fails the next terminal write at about the moment its SIGHUP arrives.
        let quit = Arc::new(AtomicI32::new(0));
        let flag = Arc::clone(&quit);
        let raiser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            flag.store(SIGNAL, Ordering::SeqCst);
        });
        assert_eq!(signal_after(&quit, Duration::from_secs(10)), SIGNAL);
        raiser.join().expect("raiser thread");

        let started = Instant::now();
        assert_eq!(
            signal_after(&AtomicI32::new(0), Duration::from_millis(30)),
            0
        );
        assert!(started.elapsed() >= Duration::from_millis(30));
        let started = Instant::now();
        assert_eq!(
            signal_after(&AtomicI32::new(SIGNAL), Duration::from_secs(10)),
            SIGNAL
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "no wait once set"
        );
    }

    #[test]
    fn an_error_exit_with_stdin_closed_ends_as_a_hangup() {
        // A terminal closed without SIGHUP: the write fails, no signal comes, and
        // stdin is closed, so the program ends by SIGHUP rather than with status 1.
        let looked = Cell::new(0);
        let closed = || {
            looked.set(looked.get() + 1);
            true
        };
        let quit = AtomicI32::new(0);
        assert_eq!(
            signal_after_error(&quit, Duration::from_millis(10), closed),
            HANGUP
        );
        assert_eq!(looked.get(), 1);
        // With stdin open the error is returned as it is.
        assert_eq!(
            signal_after_error(&quit, Duration::from_millis(10), || false),
            0
        );
        // A signal that arrived is kept, and stdin is not looked at.
        let quit = AtomicI32::new(SIGNAL);
        assert_eq!(
            signal_after_error(&quit, Duration::from_secs(10), || panic!("not asked")),
            SIGNAL
        );
    }

    #[test]
    fn faults_are_injected_only_by_name_and_only_in_debug_builds() {
        let expect = |fault| cfg!(debug_assertions).then_some(fault);
        assert_eq!(
            injected_fault(Some("engine-panic")),
            expect(Fault::EnginePanic)
        );
        assert_eq!(injected_fault(Some("ui-panic")), expect(Fault::UiPanic));
        assert_eq!(injected_fault(Some("ui-hang")), expect(Fault::UiHang));
        for value in [None, Some(""), Some("UI-PANIC"), Some("panic")] {
            assert_eq!(injected_fault(value), None, "{value:?}");
        }
    }

    #[test]
    fn a_ui_fault_waits_for_a_key_press() {
        // Without a key the batch passes; with one, the injected panic fires.
        inject_ui_fault(Fault::UiPanic, &[AppEvent::Tick]);
        inject_ui_fault(Fault::EnginePanic, &chars("1"));
        let panic = std::panic::catch_unwind(|| inject_ui_fault(Fault::UiPanic, &chars("1")));
        assert!(panic.is_err());
    }

    #[test]
    fn a_panicking_engine_still_gets_a_move_through_the_worker() {
        let engine: Arc<dyn Engine> = Arc::new(PanickingEngine(local_engine()));
        assert_eq!(engine.status(), "No JEV_API_KEY — local search");
        let (tx, rx) = mpsc::channel();
        request_engine(
            worker::EngineRequest::new(1, Game::new(), Provider::Jev),
            &engine,
            &tx,
        );
        let reply = rx.recv_timeout(REPLY_TIMEOUT).expect("a reply arrives");
        let EngineOutcome::Move(computer) = reply.outcome else {
            panic!("expected the fallback move, got {:?}", reply.outcome);
        };
        assert_eq!(computer.note.as_deref(), Some(worker::ENGINE_ERROR_NOTE));
    }

    #[test]
    fn an_engine_thread_that_cannot_start_is_answered_with_a_failure() {
        // Human vs the computer as Black: the computer is asked to move at once.
        let mut app = new_app();
        let actions = app.handle(key(KeyCode::Char('3')), Instant::now());
        let Some(Action::RequestEngine(request)) = actions.into_iter().next() else {
            panic!("the computer is asked to move");
        };
        let (generation, hash) = (request.generation, request.hash);
        let (tx, rx) = mpsc::channel();
        request_engine_with(request, app.engine(Provider::Jev), &tx, |_, _, _| {
            Err(io::Error::other("out of threads"))
        });
        let reply = rx.try_recv().expect("answered at once, without a thread");
        assert!(worker::is_current(&reply, generation, hash));
        assert_eq!(
            reply.outcome,
            EngineOutcome::Failed("cannot start the engine thread: out of threads".to_string())
        );
        assert_eq!(reply.exchange, None);
        // The app stops waiting, says so, and space asks again.
        assert_eq!(app.in_flight(), 1);
        let _ = app.handle(AppEvent::Engine(reply), Instant::now());
        assert_eq!(app.in_flight(), 0);
        assert!(!app.is_thinking());
        assert_eq!(app.status_line(), app::ENGINE_FAILED);
        let retry = app.handle(key(KeyCode::Char(' ')), Instant::now());
        assert!(
            matches!(retry.as_slice(), [Action::RequestEngine(_)]),
            "{retry:?}"
        );
    }

    #[test]
    fn engine_requests_are_answered_on_the_reply_channel() {
        let engine = local_engine();
        let (tx, rx) = mpsc::channel();
        let request = worker::EngineRequest::new(7, crate::core::Game::new(), Provider::Jev);
        let hash = request.hash;

        request_engine(request, &engine, &tx);

        let reply = rx.recv_timeout(REPLY_TIMEOUT).expect("a reply arrives");
        assert!(worker::is_current(&reply, 7, hash));
        assert!(matches!(reply.outcome, EngineOutcome::Move(_)), "{reply:?}");
    }

    #[test]
    fn the_run_loop_counts_every_request_until_its_answer() {
        // Human vs Jev as Black: the app asks, the worker answers, the count drops.
        let mut app = new_app();
        let quit = AtomicI32::new(0);
        let run = drive(
            &mut app,
            &quit,
            vec![Step::Events(chars("3")), Step::AwaitEngine, Step::Signal],
        );
        run.result.expect("loop ends cleanly");
        assert_eq!(app.game().moves().len(), 1);
        assert_eq!(
            run.in_flight,
            [0, 1, 0],
            "counted while outstanding, released by the answer"
        );
        assert_eq!(app.in_flight(), 0);
    }

    /// An engine that answers only once released, like a slow Jev call.
    struct BlockedEngine {
        release: std::sync::Mutex<Receiver<()>>,
        inner: Arc<dyn Engine>,
    }

    impl Engine for BlockedEngine {
        fn choose(&self, game: &Game) -> Option<ComputerMove> {
            // A closed channel releases too.
            let _ = self.release.lock().expect("release lock").recv();
            self.inner.choose(game)
        }

        fn status(&self) -> String {
            self.inner.status()
        }

        fn provider(&self) -> Provider {
            self.inner.provider()
        }

        fn enabled(&self) -> bool {
            self.inner.enabled()
        }

        fn warnings(&self) -> Vec<String> {
            self.inner.warnings()
        }
    }

    #[test]
    fn quitting_does_not_wait_for_a_slow_engine() {
        // Engine threads cannot be cancelled (spec 6.5), so quitting must not join them.
        let (release, released) = mpsc::channel();
        // Should the loop wait anyway, the engine is let go after 10 s and the timing
        // assertion below fails instead of the test hanging.
        let late = release.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(10));
            let _ = late.send(());
        });
        let engine = Arc::new(BlockedEngine {
            release: std::sync::Mutex::new(released),
            inner: local_engine(),
        });
        let mut app = App::new(
            engine,
            local_laya_engine(),
            GlyphSet::Solid,
            true,
            Vec::new(),
        )
        .with_home(None);
        let quit = AtomicI32::new(0);
        // Human vs Jev as White: after e4 the engine is asked, and never answers. With a
        // move played, `q` asks first, and `y` quits.
        let mut command = vec![key(KeyCode::Char('/'))];
        command.extend(chars("e4"));
        command.push(key(KeyCode::Enter));
        let started = Instant::now();
        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("2")),
                Step::Events(command),
                Step::Events(vec![key(KeyCode::Esc)]),
                Step::Events(chars("q")),
                Step::Events(chars("y")),
            ],
        );
        let took = started.elapsed();
        run.result.expect("loop ends cleanly");
        assert_eq!(run.unused_steps, 0, "y answered the confirmation");
        assert_eq!(uci_moves(app.game()), ["e2e4"]);
        assert!(app.should_quit());
        assert!(app.is_thinking(), "the engine never answered");
        assert_eq!(app.in_flight(), 1);
        assert!(took < Duration::from_secs(5), "quit took {took:?}");
        let _ = release.send(());
    }
}
