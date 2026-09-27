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

use std::io::{self, IsTerminal, Write};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::backend::Backend;
use ratatui::crossterm::event::Event;

use crate::core::Game;
use crate::engine::{ComputerMove, ComputerPlayer, EngineConfig};

use self::app::{Action, App};
use self::debug::DebugLog;
use self::event::AppEvent;
use self::graphics::{Graphics, LateAnswers};
use self::worker::{Engine, EngineOutcome, EngineReply};

/// How long one batch waits for terminal input before its `Tick` (spec 6.5).
const TICK: Duration = Duration::from_millis(50);

/// How long a UI that failed waits for a quit signal before returning its error. A
/// terminal that hangs up fails the next write at about the moment its SIGHUP
/// arrives, and the process should then end by that signal rather than by the error.
const SIGNAL_GRACE: Duration = Duration::from_millis(100);

/// How long quitting waits for the debug log to write the exchanges still queued.
const LOG_GRACE: Duration = Duration::from_millis(500);

/// The `--help` text.
const USAGE: &str = concat!(
    "rchess: chess in the terminal, against a person or the Jev computer player\n",
    "\n",
    "Usage: ",
    env!("CARGO_PKG_NAME"),
    " [--glyphs image|solid|outline|ascii] [--debug]\n",
    "\n",
    "Options:\n",
    "  --glyphs <set>     how pieces look: image (pictures; the default when the\n",
    "                     terminal can show them), solid (otherwise the default),\n",
    "                     outline or ascii; `g` cycles them during a game\n",
    "  --debug            debug mode: keep every Jev request and answer, show them\n",
    "                     with `d` during a game and append them to the debug log\n",
    "  -h, --help         show this help and exit\n",
    "\n",
    "Environment:\n",
    "  JEV_API_KEY        key for the Jev computer player (TYPESAFE_API_KEY also\n",
    "                     works); without one the computer uses local search\n",
    "  JEV_MODEL          Jev model (default jev-latest)\n",
    "  JEV_MAX_OPTIONS    moves offered to Jev per turn, 1-255 (default 40)\n",
    "  JEV_FILTER_LOSING  keep losing moves off Jev's shortlist (default true)\n",
    "  RCHESS_GLYPHS      glyph set when --glyphs is not given\n",
    "  RCHESS_IMAGES      off: no piece pictures, and no graphics query at start\n",
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

/// Runs the terminal UI until the user quits or SIGINT, SIGTERM or SIGHUP
/// arrives. `args` are the command-line arguments without the program name.
///
/// `--help` prints the usage and returns without touching the terminal.
/// Unknown arguments and a missing or invalid `--glyphs` value do not stop the
/// program: they are listed as warnings on the menu, like invalid engine
/// settings, and so is a graphics query that failed. The computer player comes
/// from `EngineConfig::from_env()`; with no `JEV_API_KEY` it plays by local
/// search and never uses the network. In debug mode (`--debug` or `RCHESS_DEBUG`)
/// it records its exchanges with Jev (`EngineConfig::trace`).
///
/// Unless images are off ([`glyphs::images_wanted`]), the terminal is asked about
/// graphics right after it is set up, which takes up to [`graphics::QUERY_TIMEOUT`]
/// when it does not answer.
///
/// Call it from the main thread (the panic hook restores the terminal only for
/// a panic on the thread named "main"). The terminal is restored on every exit:
/// normal return, error return, panic, or signal. After a signal it does not
/// return: once the terminal is restored, the process ends by that signal
/// ([`terminal::exit_by_signal`]), so the shell sees it was interrupted.
///
/// # Errors
///
/// When stdin or stdout is not a terminal, when signal handlers cannot be installed, or
/// when the terminal cannot be set up, read or drawn. Writing the `--help` text
/// can fail too.
pub fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let options = match parse_args(args) {
        Cli::Help => return io::stdout().lock().write_all(USAGE.as_bytes()),
        Cli::Play(options) => options,
    };
    // Before touching the terminal: without a terminal on stdin the key reader fails only
    // after the menu has been drawn on the alternate screen.
    if let Some(problem) = terminal_problem(io::stdin().is_terminal(), io::stdout().is_terminal()) {
        return Err(io::Error::other(problem));
    }

    let env = |name: &str| std::env::var(name).ok();
    let images = glyphs::images_wanted(options.glyphs.as_deref(), env);
    let fault = injected_fault(env("RCHESS_FAULT").as_deref());
    let config = engine_config(EngineConfig::from_env(), debug::enabled(options.debug, env));
    let mut engine: Arc<dyn Engine> = Arc::new(ComputerPlayer::from_config(config));
    if fault == Some(Fault::EnginePanic) {
        engine = Arc::new(PanickingEngine(engine));
    }

    // Before `enter`, so a signal that arrives during setup still ends in a
    // clean restore instead of killing the process with the terminal in raw mode.
    let quit = terminal::register_signals()?;
    let result = play(&quit, images, fault, |graphics| {
        build_app(options, engine, graphics, env)
    });
    let signal = if result.is_err() {
        signal_after(&quit, SIGNAL_GRACE)
    } else {
        quit.load(Ordering::SeqCst)
    };
    if signal != 0 {
        // The terminal is restored; end the way the signal asked for.
        terminal::exit_by_signal(signal);
    }
    result
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
    engine: Arc<dyn Engine>,
    graphics: Graphics,
    get: impl Fn(&str) -> Option<String>,
) -> App {
    let (glyph_set, glyph_warnings) =
        glyphs::initial_glyphs(options.glyphs.as_deref(), &get, graphics.support());
    let mut warnings = options.warnings;
    warnings.extend(glyph_warnings);
    warnings.extend(graphics.warning);
    let mut app = App::new(engine, glyph_set, glyphs::detect_truecolor(&get), warnings)
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
    // A terminal that does not know a probe may have printed it, and the first draw
    // writes only the cells that are not blank. (`Terminal::clear` would also ask
    // for the cursor position, another answer to wait for.)
    screen.backend_mut().clear()?;
    let mut late = LateAnswers::after(&graphics, Instant::now());
    let mut app = build(graphics);
    let result = run_loop(
        &mut app,
        quit,
        |app| {
            screen
                .draw(|frame| app.render(frame, Instant::now()))
                .map(drop)
        },
        |replies| {
            let mut batch = event::collect(replies, TICK)?;
            drop_late_answers(&mut batch, &mut late, Instant::now());
            if let Some(fault) = fault {
                inject_ui_fault(fault, &batch);
            }
            Ok(batch)
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

    fn uses_jev(&self) -> bool {
        self.0.uses_jev()
    }

    fn warnings(&self) -> Vec<String> {
        self.0.warnings()
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cli {
    /// `-h` or `--help` appeared anywhere: print the usage and exit.
    Help,
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

/// Reads `--glyphs <set>`, `--glyphs=<set>`, `--debug`, `-h` and `--help`. The last
/// `--glyphs` wins. A `--glyphs` followed by nothing or by another option has no
/// value; that and any other argument become warnings rather than errors.
fn parse_args(args: impl IntoIterator<Item = String>) -> Cli {
    let mut options = Options::default();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Cli::Help,
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
/// Drawing comes before each batch so that mouse events are hit-tested against
/// what is on screen. Every batch ends with a `Tick`, so the screen is redrawn at
/// least once per batch (the spinner and the Jev vs Jev delay depend on it);
/// ratatui writes only the cells that changed. Once the app quits, the rest of
/// the batch is dropped.
///
/// `draw` and `next_batch` are the terminal in production and a `TestBackend`
/// with scripted batches in tests. Engine replies travel over a channel created
/// here: `next_batch` receives its end, the worker threads send on the other.
///
/// # Errors
///
/// The first error from `draw` or `next_batch`.
fn run_loop(
    app: &mut App,
    quit: &AtomicI32,
    mut draw: impl FnMut(&mut App) -> io::Result<()>,
    mut next_batch: impl FnMut(&Receiver<EngineReply>) -> io::Result<Vec<AppEvent>>,
) -> io::Result<()> {
    let (replies_tx, replies) = mpsc::channel();
    while quit.load(Ordering::SeqCst) == 0 && !app.should_quit() {
        draw(app)?;
        let batch = next_batch(&replies)?;
        let now = Instant::now();
        for event in batch {
            for action in app.handle(event, now) {
                perform(action, app.engine(), &replies_tx);
            }
            if app.should_quit() {
                break;
            }
        }
    }
    Ok(())
}

/// Carries out one [`Action`] for the app.
fn perform(action: Action, engine: &Arc<dyn Engine>, replies: &Sender<EngineReply>) {
    match action {
        Action::RequestEngine(request) => {
            let (generation, hash) = (request.generation, request.hash);
            if let Err(error) = worker::spawn_request(Arc::clone(engine), request, replies.clone())
            {
                // No thread means no reply. Answer for it, so the app stops
                // waiting: it shows the failure and lets the person retry.
                let failed = EngineReply {
                    generation,
                    hash,
                    outcome: EngineOutcome::Failed(format!(
                        "cannot start the engine thread: {error}"
                    )),
                    exchange: None,
                };
                // The receiver lives in `run_loop`, which is still running.
                let _ = replies.send(failed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
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

    fn play(glyphs: Option<&str>, warnings: &[&str]) -> Cli {
        Cli::Play(Options {
            glyphs: glyphs.map(str::to_string),
            warnings: warnings.iter().map(|w| (*w).to_string()).collect(),
            ..Options::default()
        })
    }

    // ----- command line -----

    #[test]
    fn no_arguments_start_with_defaults() {
        assert_eq!(parse_args(args(&[])), play(None, &[]));
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
            play(Some("ascii"), &[])
        );
        assert_eq!(
            parse_args(args(&["--glyphs=outline"])),
            play(Some("outline"), &[])
        );
        assert_eq!(
            parse_args(args(&["--glyphs", "ascii", "--glyphs=solid"])),
            play(Some("solid"), &[])
        );
    }

    #[test]
    fn glyph_values_are_passed_on_unchecked() {
        // `glyphs::initial_glyphs` validates the value and words the warning.
        assert_eq!(
            parse_args(args(&["--glyphs", "fancy"])),
            play(Some("fancy"), &[])
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
            play(Some("image"), &[])
        );
    }

    #[test]
    fn debug_mode_can_be_asked_for() {
        let debug = |list: &[&str]| match parse_args(args(list)) {
            Cli::Play(options) => options,
            Cli::Help => panic!("expected Play"),
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
        assert_eq!(parse_args(args(&["--glyphs"])), play(None, &[missing]));
        assert_eq!(
            parse_args(args(&["--glyphs", "--glyphs=ascii"])),
            play(Some("ascii"), &[missing])
        );
    }

    #[test]
    fn unknown_arguments_are_warnings_in_order() {
        assert_eq!(
            parse_args(args(&["ascii", "--glyph", ""])),
            play(
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
            "JEV_API_KEY",
            "TYPESAFE_API_KEY",
            "JEV_MODEL",
            "JEV_MAX_OPTIONS",
            "JEV_FILTER_LOSING",
            "RCHESS_GLYPHS",
            "RCHESS_IMAGES",
            "RCHESS_DEBUG",
            "RCHESS_DEBUG_LOG",
            "NO_COLOR",
            "COLORTERM",
        ] {
            assert!(USAGE.contains(name), "{name}");
        }
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

    fn options(glyphs: Option<&str>, warnings: &[&str]) -> Options {
        let Cli::Play(options) = play(glyphs, warnings) else {
            unreachable!("play() builds Cli::Play");
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
        let app = build_app(options(None, &[]), local_engine(), graphics, |_| None);
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
        let app = build_app(options(None, &[]), local_engine(), Graphics::off(cell), get);
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
        let app = build_app(options(None, &[]), local_engine(), off.clone(), vars("0"));
        assert!(!app.debug_mode());
        let mut app = build_app(options(None, &[]), local_engine(), off.clone(), vars("1"));
        assert!(app.debug_mode());
        assert!(app.exchanges().is_some_and(debug::History::is_empty));
        app.close_debug_log(Duration::from_secs(10));
        assert!(!path.exists(), "no exchange, no file");
        let flagged = Options {
            debug: true,
            ..Options::default()
        };
        let app = build_app(flagged, local_engine(), off, vars(""));
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

    fn new_app() -> App {
        App::new(local_engine(), GlyphSet::Solid, true, Vec::new()).with_home(None)
    }

    /// What a scripted run of [`run_loop`] did.
    struct Run {
        result: io::Result<()>,
        draws: usize,
        unused_steps: usize,
        /// [`App::in_flight`] at every draw, so before every batch.
        in_flight: Vec<usize>,
    }

    /// Runs [`run_loop`] on an 80×24 `TestBackend`, serving `steps` as batches. Running
    /// out of steps before the loop ends is an error, so a loop that fails to quit fails
    /// the test instead of hanging it.
    fn drive(app: &mut App, quit: &AtomicI32, steps: Vec<Step>) -> Run {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let mut steps = VecDeque::from(steps);
        let mut draws = 0;
        let mut in_flight = Vec::new();
        let result = run_loop(
            app,
            quit,
            |app| {
                draws += 1;
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
        );
        Run {
            result,
            draws,
            unused_steps: steps.len(),
            in_flight,
        }
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
        assert_eq!(app.mode(), Mode::HumanVsJev { human: Side::Black });

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
        let mut app = App::new(engine, GlyphSet::Solid, true, Vec::new())
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
        assert_eq!(run.draws, 5, "one draw before every batch");
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
        assert_eq!(app.mode(), Mode::HumanVsJev { human: Side::Black });
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
        perform(
            Action::RequestEngine(worker::EngineRequest::new(1, Game::new())),
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
    fn engine_requests_are_answered_on_the_reply_channel() {
        let engine = local_engine();
        let (tx, rx) = mpsc::channel();
        let request = worker::EngineRequest::new(7, crate::core::Game::new());
        let hash = request.hash;

        perform(Action::RequestEngine(request), &engine, &tx);

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

        fn uses_jev(&self) -> bool {
            self.inner.uses_jev()
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
        let mut app = App::new(engine, GlyphSet::Solid, true, Vec::new()).with_home(None);
        let quit = AtomicI32::new(0);
        let started = Instant::now();
        let run = drive(
            &mut app,
            &quit,
            vec![
                Step::Events(chars("3")),
                Step::Events(chars("q")),
                Step::Events(chars("y")),
            ],
        );
        let took = started.elapsed();
        run.result.expect("loop ends cleanly");
        assert!(app.should_quit());
        assert!(app.is_thinking(), "the engine never answered");
        assert_eq!(app.in_flight(), 1);
        assert!(took < Duration::from_secs(5), "quit took {took:?}");
        let _ = release.send(());
    }
}
