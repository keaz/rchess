//! Terminal setup and teardown (spec 6.6).
//!
//! [`enter`] puts the terminal in raw mode on the alternate screen, runs the
//! caller's start-up step (the graphics query), then turns on click-and-drag
//! mouse reporting and bracketed paste. [`leave`] undoes all of it
//! and is reached on every exit path: a normal return or `?` error (through
//! [`Guard`]), a panic on the UI thread (through the panic hook) and SIGINT,
//! SIGTERM or SIGHUP (through the flag from [`register_signals`], which the main
//! loop checks every tick; the program then ends by that signal with
//! [`exit_by_signal`]). If the loop does not react within [`STUCK_GRACE`], the
//! signal thread restores the terminal itself and ends the process. That is also
//! how a hangup ends when the UI is idle: crossterm keeps polling a hung-up tty
//! without ever returning an error.
//!
//! Nothing here may panic while restoring. After a hangup every write to the tty
//! fails, and `eprintln!` panics on a failed write, so a restore that printed its
//! error (as `ratatui::restore` and ratatui's panic hook do) would panic inside the
//! panic hook and abort the process. [`leave`] ignores every error instead, and the
//! panic hook calls the standard hook, not ratatui's.

use std::fmt;
use std::io::{self, stdout};
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
#[cfg(unix)]
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::is_raw_mode_enabled;
use ratatui::crossterm::{Command, execute};

/// True between a successful [`enter`] and the first [`leave`], which makes
/// `leave` idempotent and a no-op when the terminal was never set up.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The only thread whose panic may touch the terminal: the one running the UI.
const UI_THREAD: &str = "main";

/// How long the UI loop has to act on a quit signal (it checks every 50 ms tick) before
/// the signal thread decides it is stuck and restores the terminal itself.
pub const STUCK_GRACE: Duration = Duration::from_secs(1);

/// How long a forced restore may take; if the stuck UI thread holds the terminal, the
/// process ends without it rather than hang.
#[cfg(unix)]
const RESTORE_DEADLINE: Duration = Duration::from_millis(500);

/// Turns on mouse reporting for presses, releases, drags and the wheel, with SGR
/// coordinates: `?1000h` (press/release), `?1002h` (motion while a button is
/// held) and `?1006h` (SGR encoding). Unlike crossterm's `EnableMouseCapture`
/// it omits `?1003h`, which would report every mouse movement, and the legacy
/// `?1015h` encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnableClickMouse;

impl Command for EnableClickMouse {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1000h\x1b[?1002h\x1b[?1006h")
    }

    /// The Windows console has a single mouse-input switch.
    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        ratatui::crossterm::event::EnableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

/// Turns off what [`EnableClickMouse`] turned on, in reverse order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisableClickMouse;

impl Command for DisableClickMouse {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1006l\x1b[?1002l\x1b[?1000l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        ratatui::crossterm::event::DisableMouseCapture.execute_winapi()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

/// Sets up the terminal: raw mode and the alternate screen (`ratatui::try_init`),
/// then `before_input`, then click-and-drag mouse reporting and bracketed paste,
/// then a thread-aware panic hook. Returns the terminal with what `before_input`
/// returned. Call it once, from the main thread, and hold a [`Guard`] for as long
/// as the terminal is in use.
///
/// `before_input` is for the graphics query (spec 9.3): it runs in raw mode, so
/// the terminal's answers are not echoed, on the alternate screen, so nothing it
/// writes stays on the main screen, and before any mouse or paste report can mix
/// into the answers.
///
/// The panic hook replaces the one `try_init` installs (ratatui's prints when its
/// restore fails, which panics on a hung-up tty). A panic on the "main" thread
/// runs [`leave`], then the hook that was installed before `enter` (normally the
/// standard one, which prints the message); a panic on any other thread (the
/// engine's is caught by `catch_unwind`) leaves the running UI alone.
///
/// Do not let the returned terminal drop while the process may still be running
/// on a dead tty: its `Drop` shows the cursor and prints when that fails. Keep it
/// in a `ManuallyDrop`; [`leave`] shows the cursor instead.
///
/// # Errors
///
/// When there is no usable terminal (for example no controlling tty) or it
/// rejects the setup sequences. Whatever was set up is restored first, and the
/// panic hook is put back as it was. `before_input` does not run when raw mode or
/// the alternate screen could not be entered.
pub fn enter<T>(before_input: impl FnOnce() -> T) -> io::Result<(DefaultTerminal, T)> {
    // Taken before `try_init` wraps it in ratatui's hook, which is then dropped.
    let original = panic::take_hook();
    let terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(err) => {
            undo_partial_init();
            panic::set_hook(original);
            return Err(err);
        }
    };
    ACTIVE.store(true, Ordering::SeqCst);
    let value = before_input();
    finish_enter(
        original,
        || execute!(stdout(), EnableClickMouse, EnableBracketedPaste),
        leave,
    )?;
    Ok((terminal, value))
}

/// The rest of [`enter`] once `try_init` succeeded: `enable` turns on mouse
/// reporting and bracketed paste. If it fails, `undo` restores the terminal and
/// `original` becomes the panic hook again (dropping ratatui's); otherwise the
/// thread-aware hook replaces ratatui's. Until then ratatui's hook is in place,
/// which restores the terminal too (only without the care for a hung-up tty);
/// the start-up step that runs in between does not panic.
fn finish_enter(
    original: PanicHook,
    enable: impl FnOnce() -> io::Result<()>,
    undo: impl FnOnce(),
) -> io::Result<()> {
    if let Err(err) = enable() {
        undo();
        panic::set_hook(original);
        return Err(err);
    }
    install_panic_hook(original);
    Ok(())
}

/// Restores the terminal: turns off mouse reporting and bracketed paste, then
/// raw mode and the alternate screen (`ratatui::try_restore`), then shows the
/// cursor. Every error is ignored and nothing is printed, so this never panics,
/// even on a hung-up tty. Only the first call after [`enter`] does anything, so
/// every exit path may call it.
pub fn leave() {
    leave_once(&ACTIVE, || {
        let _ = execute!(stdout(), DisableClickMouse, DisableBracketedPaste);
        let _ = ratatui::try_restore();
        let _ = execute!(stdout(), Show);
    });
}

/// Calls [`leave`] when dropped. Bind it to a named variable for the lifetime of
/// the UI: `let _guard = Guard;` (`let _ = Guard;` would drop it immediately).
#[derive(Debug)]
#[must_use = "the terminal is restored when the guard is dropped"]
pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        leave();
    }
}

/// Watches SIGINT, SIGTERM and SIGHUP. The returned flag holds the number of the first
/// one to arrive (0 until then); the main loop checks it every tick, leaves through the
/// normal restore path and then calls [`exit_by_signal`].
///
/// A thread named "signals" receives them. After the first signal it waits
/// [`STUCK_GRACE`]; if the process is still running by then, the UI loop is stuck, so
/// the thread restores the terminal (giving up after a short deadline if the UI thread
/// holds it) and ends the process by that signal. Further signals change nothing, so two
/// in quick succession never skip the restore. On non-Unix platforms the flag never
/// changes.
///
/// # Errors
///
/// When the signal handlers or the thread cannot be set up.
pub fn register_signals() -> io::Result<Arc<AtomicI32>> {
    let received = Arc::new(AtomicI32::new(0));
    #[cfg(unix)]
    watch_signals(
        &QUIT_SIGNALS,
        Arc::clone(&received),
        STUCK_GRACE,
        force_quit,
    )?;
    Ok(received)
}

/// Ends the process the way `signal`'s default action does, so the parent (a shell or
/// a supervisor) sees it was interrupted: SIGINT, SIGTERM and SIGHUP terminate it by
/// that signal. Call it only once the terminal is restored. If the signal cannot be
/// re-raised, exits with status 128 + `signal`, the shell convention.
pub fn exit_by_signal(signal: i32) -> ! {
    #[cfg(unix)]
    {
        let _ = signal_hook::low_level::emulate_default_handler(signal);
    }
    std::process::exit(128_i32.saturating_add(signal))
}

/// Signals that ask the app to quit. In raw mode Ctrl+C arrives as a key event,
/// so SIGINT only comes from outside (`kill -INT`).
#[cfg(unix)]
const QUIT_SIGNALS: [std::ffi::c_int; 3] = {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
    [SIGINT, SIGTERM, SIGHUP]
};

/// Starts the "signals" thread: the first of `signals` is stored in `received`, and
/// `on_stuck` runs with it `grace` later (in production the process has ended by then,
/// unless the UI is stuck).
#[cfg(unix)]
fn watch_signals(
    signals: &[std::ffi::c_int],
    received: Arc<AtomicI32>,
    grace: Duration,
    on_stuck: fn(std::ffi::c_int),
) -> io::Result<()> {
    let mut incoming = signal_hook::iterator::Signals::new(signals)?;
    thread::Builder::new()
        .name("signals".to_string())
        .spawn(move || {
            let Some(signal) = incoming.forever().next() else {
                return;
            };
            received.store(signal, Ordering::SeqCst);
            thread::sleep(grace);
            on_stuck(signal);
        })
        .map(drop)
}

/// The UI loop did not act on `signal` in time: restore the terminal from here and end
/// the process by the signal. Raw mode goes first: turning it off writes nothing to the
/// terminal, so it works even when the terminal has stopped reading output and the
/// escape-sequence writes of [`leave`] block (or a stuck UI thread is blocked in them).
#[cfg(unix)]
fn force_quit(signal: std::ffi::c_int) {
    forced_restore(
        || {
            let _ = disable_raw_mode();
        },
        leave,
        RESTORE_DEADLINE,
    );
    exit_by_signal(signal);
}

/// Runs `raw_off` here, then `restore` on a helper thread, waiting at most `deadline` for
/// it: a restore blocked writing to the terminal cannot keep the process alive.
#[cfg(unix)]
fn forced_restore(
    raw_off: impl FnOnce(),
    restore: impl FnOnce() + Send + 'static,
    deadline: Duration,
) {
    raw_off();
    let (done_tx, done) = std::sync::mpsc::channel();
    let helper = thread::Builder::new()
        .name("restore".to_string())
        .spawn(move || {
            restore();
            let _ = done_tx.send(());
        });
    if helper.is_ok() {
        let _ = done.recv_timeout(deadline);
    }
}

/// `ratatui::try_init` failed part-way. If raw mode is on it got past
/// `enable_raw_mode`, so the alternate screen may be active as well: undo both
/// (errors ignored). Otherwise nothing changed and nothing is written.
fn undo_partial_init() {
    if is_raw_mode_enabled().unwrap_or(true) {
        let _ = ratatui::try_restore();
    }
}

/// Runs `restore` if `active` was set, clearing it.
fn leave_once(active: &AtomicBool, restore: impl FnOnce()) {
    if active.swap(false, Ordering::SeqCst) {
        restore();
    }
}

/// The panic hook signature `std::panic::take_hook` returns.
type PanicHook = Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send + 'static>;

/// Replaces the current hook (ratatui's, installed by `try_init`) with one that,
/// for a panic on the UI thread, restores the terminal and then calls `original`
/// (the hook from before `try_init`), so the message is printed on the main
/// screen. For other threads it only logs.
fn install_panic_hook(original: PanicHook) {
    panic::set_hook(Box::new(move |info| {
        if !is_ui_thread(thread::current().name()) {
            // Restoring here would pull the terminal out from under the running
            // UI, and printing would scribble over it. The engine thread's panic
            // is caught and reported to the UI as a failed reply.
            log::error!("{info}");
            return;
        }
        leave();
        original(info);
    }));
}

fn is_ui_thread(name: Option<&str>) -> bool {
    name == Some(UI_THREAD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn ansi(command: impl Command) -> String {
        let mut out = String::new();
        command.write_ansi(&mut out).unwrap();
        out
    }

    #[test]
    fn enable_click_mouse_writes_exact_sequence() {
        assert_eq!(ansi(EnableClickMouse), "\x1b[?1000h\x1b[?1002h\x1b[?1006h");
    }

    #[test]
    fn disable_click_mouse_writes_exact_sequence() {
        assert_eq!(ansi(DisableClickMouse), "\x1b[?1006l\x1b[?1002l\x1b[?1000l");
    }

    #[test]
    fn click_mouse_never_asks_for_motion_reports() {
        for text in [ansi(EnableClickMouse), ansi(DisableClickMouse)] {
            assert!(!text.contains("1003"), "{text:?}");
            assert!(!text.contains("1015"), "{text:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn setup_and_teardown_byte_streams() {
        let mut setup = Vec::new();
        execute!(setup, EnableClickMouse, EnableBracketedPaste).unwrap();
        assert_eq!(
            String::from_utf8(setup).unwrap(),
            "\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?2004h"
        );

        let mut teardown = Vec::new();
        execute!(teardown, DisableClickMouse, DisableBracketedPaste).unwrap();
        assert_eq!(
            String::from_utf8(teardown).unwrap(),
            "\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?2004l"
        );
    }

    #[test]
    fn leave_once_restores_only_once() {
        let active = AtomicBool::new(true);
        let calls = Cell::new(0);
        leave_once(&active, || calls.set(calls.get() + 1));
        leave_once(&active, || calls.set(calls.get() + 1));
        assert_eq!(calls.get(), 1);
        assert!(!active.load(Ordering::SeqCst));
    }

    #[test]
    fn leave_once_does_nothing_when_never_entered() {
        let active = AtomicBool::new(false);
        leave_once(&active, || panic!("must not restore"));
    }

    #[test]
    fn leave_and_guard_are_no_ops_without_enter() {
        // No terminal was set up in this process, so neither may write anything.
        assert!(!ACTIVE.load(Ordering::SeqCst));
        leave();
        leave();
        drop(Guard);
        assert!(!ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn a_failed_enable_puts_the_original_panic_hook_back() {
        // The panic hook is global: this is the only test that changes it, and it puts
        // back the one it found. A panic on this test thread (not "main") reaches the
        // original hook only when that hook is installed again; the thread-aware hook
        // would just log it.
        static PROBES: AtomicI32 = AtomicI32::new(0);
        const PROBE: &str = "terminal hook probe";
        fn probe() -> PanicHook {
            Box::new(|info| {
                if info.payload().downcast_ref::<&str>() == Some(&PROBE) {
                    PROBES.fetch_add(1, Ordering::SeqCst);
                }
            })
        }
        let reaches_original = || {
            let before = PROBES.load(Ordering::SeqCst);
            assert!(panic::catch_unwind(|| panic::panic_any(PROBE)).is_err());
            PROBES.load(Ordering::SeqCst) > before
        };
        let found = panic::take_hook();

        let undone = Cell::new(false);
        let result = finish_enter(
            probe(),
            || Err(io::Error::other("no mouse")),
            || undone.set(true),
        );
        assert_eq!(result.unwrap_err().to_string(), "no mouse");
        assert!(undone.get(), "the terminal is restored");
        let failed_enable_restores = reaches_original();

        let result = finish_enter(probe(), || Ok(()), || panic!("must not restore"));
        assert!(result.is_ok());
        let success_installs_thread_hook = !reaches_original();

        panic::set_hook(found);
        assert!(failed_enable_restores, "original hook put back");
        assert!(success_installs_thread_hook, "thread-aware hook installed");
    }

    #[test]
    fn only_the_main_thread_restores_on_panic() {
        assert!(is_ui_thread(Some("main")));
        assert!(!is_ui_thread(Some("engine")));
        assert!(!is_ui_thread(Some("tui::terminal::tests::some_test")));
        assert!(!is_ui_thread(None));
    }

    #[cfg(unix)]
    #[test]
    fn quits_on_interrupt_terminate_and_hangup() {
        use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
        assert_eq!(QUIT_SIGNALS, [SIGINT, SIGTERM, SIGHUP]);
    }

    #[cfg(unix)]
    #[test]
    fn a_forced_restore_turns_raw_mode_off_even_when_the_writes_block() {
        // A terminal that stopped reading output blocks the escape-sequence writes;
        // raw mode must be off before they start, and the wait must end at the deadline.
        use std::sync::Mutex;
        use std::sync::mpsc;
        use std::time::Instant;
        let steps = Arc::new(Mutex::new(Vec::new()));
        let (unblock, blocked) = mpsc::channel::<()>();
        let started = Instant::now();
        let log = Arc::clone(&steps);
        forced_restore(
            || steps.lock().unwrap().push("raw mode off"),
            move || {
                log.lock().unwrap().push("writes started");
                let _ = blocked.recv();
            },
            Duration::from_millis(50),
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline holds"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while steps.lock().unwrap().len() < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*steps.lock().unwrap(), ["raw mode off", "writes started"]);
        drop(unblock);
    }

    #[cfg(unix)]
    #[test]
    fn the_first_signal_is_recorded_and_a_stuck_ui_is_handled_once() {
        // SIGUSR2 stands in for the quit signals so the test binary keeps its own
        // Ctrl+C behaviour; `on_stuck` records instead of ending the process.
        use signal_hook::consts::signal::SIGUSR2;
        use std::time::Instant;
        static STUCK: AtomicI32 = AtomicI32::new(0);
        static STUCK_CALLS: AtomicI32 = AtomicI32::new(0);
        fn record(signal: std::ffi::c_int) {
            STUCK.store(signal, Ordering::SeqCst);
            STUCK_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        let wait_for = |what: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !what() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            what()
        };

        let received = Arc::new(AtomicI32::new(0));
        watch_signals(
            &[SIGUSR2],
            Arc::clone(&received),
            Duration::from_millis(50),
            record,
        )
        .unwrap();
        assert_eq!(received.load(Ordering::SeqCst), 0);

        signal_hook::low_level::raise(SIGUSR2).unwrap();
        assert!(wait_for(&|| received.load(Ordering::SeqCst) == SIGUSR2));
        // A second signal neither ends the process nor calls `on_stuck` again.
        signal_hook::low_level::raise(SIGUSR2).unwrap();
        assert!(wait_for(&|| STUCK.load(Ordering::SeqCst) == SIGUSR2));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(STUCK_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(received.load(Ordering::SeqCst), SIGUSR2);
    }
}
