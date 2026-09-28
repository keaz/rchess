//! Terminal setup and teardown (spec 6.6).
//!
//! [`enter`] puts the terminal in raw mode on the alternate screen, runs the
//! caller's start-up step (the graphics query), then turns on click-and-drag
//! mouse reporting and bracketed paste. [`leave`] undoes all of it (deleting any
//! Kitty pictures first) and is reached on every exit path: a normal return or `?`
//! error (through [`Guard`]), a panic on the UI thread (through the panic hook) and
//! SIGINT, SIGTERM or SIGHUP (through the flag from [`register_signals`], which the
//! main loop checks every tick; the program then ends by that signal with
//! [`exit_by_signal`]). If the loop does not react within [`STUCK_GRACE`], the
//! signal thread restores the terminal itself and ends the process. That is also
//! how a hangup ends when the UI is idle: crossterm keeps polling a hung-up tty
//! without ever returning an error.
//!
//! A terminal can also close without a SIGHUP reaching the program (it is not in the
//! tty's session, or a supervisor keeps the signal from it). crossterm then spins on
//! stdin, which is at end of file or fails with EIO. [`watch_hangup`] looks at stdin
//! from a thread of its own and raises SIGHUP when it finds it closed, so that ends
//! the same way.
//!
//! Nothing here may panic while restoring. After a hangup every write to the tty
//! fails, and `eprintln!` panics on a failed write, so a restore that printed its
//! error (as `ratatui::restore` and ratatui's panic hook do) would panic inside the
//! panic hook and abort the process. [`leave`] ignores every error instead, runs every
//! step even when an earlier one failed, and the panic hook calls the standard hook,
//! not ratatui's.

use std::fmt;
use std::io::{self, stdout};
use std::panic;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::cursor::Show;
use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use ratatui::crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode, is_raw_mode_enabled};
use ratatui::crossterm::{Command, execute};
use ratatui_image::picker::cap_parser::Parser;

/// Where the terminal is between [`enter`] and [`leave`]: [`ACTIVE`] after a
/// successful `enter`, [`RESTORING`] while the first `leave` runs, [`DONE`] before
/// `enter` and once that `leave` has finished. It makes `leave` run once, a no-op when
/// the terminal was never set up, and makes a second caller wait for the first.
static STATE: AtomicU8 = AtomicU8::new(DONE);

/// [`STATE`]: set up, nothing restored yet.
const ACTIVE: u8 = 1;
/// [`STATE`]: a [`leave`] is restoring the terminal.
const RESTORING: u8 = 2;
/// [`STATE`]: nothing to restore (never set up, or restored).
const DONE: u8 = 0;

/// How long a [`leave`] waits for one already restoring on another thread before it
/// returns anyway.
const LEAVE_WAIT: Duration = Duration::from_secs(1);

/// How often the "hangup" thread looks at stdin ([`watch_hangup`]).
const HANGUP_LOOK: Duration = Duration::from_millis(100);

/// How many looks in a row must find stdin at end of file before it counts as closed:
/// one such look can also be crossterm reading the bytes between the two halves of it.
const EOF_LOOKS: u32 = 3;

/// The ids of the Kitty pictures this session built, which [`leave`] deletes.
static KITTY_IDS: KittyIds = KittyIds::new();

/// How many ids a session can use, and the highest base: a base is at most this
/// and a session's ids are at most this many past it, so they never wrap to 0.
const KITTY_ID_SPAN: u32 = 1 << 31;

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

/// Deletes the Kitty pictures `ids` and frees their data, one `a=d,d=I,i=<id>`
/// command each, so Kitty and Ghostty do not keep them after the program ends.
/// ratatui-image sends Kitty pictures as virtual placements (unicode placeholders),
/// which Kitty deletes only by image id: the delete-all command (`d=A`) leaves them.
/// With `tmux` each command is wrapped for tmux's passthrough, as ratatui-image
/// wraps the pictures. It prints nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteKittyImages {
    /// The image ids, in the order they were sent.
    pub ids: Vec<u32>,
    /// The pictures went through tmux.
    pub tmux: bool,
}

impl Command for DeleteKittyImages {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        let (start, escape, end) = Parser::tmux_start_escape_end(self.tmux);
        for id in &self.ids {
            write!(f, "{start}{escape}_Ga=d,d=I,i={id}{escape}\\{end}")?;
        }
        Ok(())
    }

    /// The Windows console draws no Kitty pictures.
    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Ok(())
    }
}

/// The image ids a session gives its Kitty pictures, and how the pictures reached
/// the terminal. The ids are consecutive from a base derived from the process id
/// ([`kitty_base`]), so two sessions in one terminal do not share one. Every id
/// handed out is remembered (as a count), and so is how many of them were dropped
/// (the cache drops every picture at once) and how many were deleted, so each picture
/// is deleted once: before the next draw once dropped, or at exit while still shown.
/// It takes no lock, so the panic hook and the signal thread can read it at any time.
#[derive(Debug)]
struct KittyIds {
    /// The first id, 0 until the first picture is built.
    base: AtomicU32,
    /// How many ids were handed out.
    count: AtomicU32,
    /// How many of the first ids handed out belong to pictures that were dropped.
    dropped: AtomicU32,
    /// How many of the first ids handed out were deleted (or are being deleted).
    deleted: AtomicU32,
    /// The last picture went through tmux.
    tmux: AtomicBool,
}

impl KittyIds {
    /// No id handed out yet; the base is chosen from the process id at the first.
    const fn new() -> KittyIds {
        KittyIds::starting_at(0)
    }

    /// Ids from `base` (1..=[`KITTY_ID_SPAN`]), or from the process id's base when 0.
    const fn starting_at(base: u32) -> KittyIds {
        KittyIds {
            base: AtomicU32::new(base),
            count: AtomicU32::new(0),
            dropped: AtomicU32::new(0),
            deleted: AtomicU32::new(0),
            tmux: AtomicBool::new(false),
        }
    }

    /// The first id.
    fn base(&self) -> u32 {
        let base = self.base.load(Ordering::SeqCst);
        if base != 0 {
            return base;
        }
        let chosen = kitty_base(std::process::id());
        match self
            .base
            .compare_exchange(0, chosen, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => chosen,
            Err(other) => other,
        }
    }

    /// The `index`th id: nonzero, and unique for the first [`KITTY_ID_SPAN`].
    fn id(base: u32, index: u32) -> u32 {
        // base ≤ 2^31 and index % 2^31 < 2^31, so the sum fits and is at least 1.
        base + index % KITTY_ID_SPAN
    }

    /// A new id for a picture sent through tmux when `tmux`, remembered for
    /// [`KittyIds::write_dropped`] and [`KittyIds::cleanup`].
    fn next(&self, tmux: bool) -> u32 {
        let base = self.base();
        self.tmux.store(tmux, Ordering::SeqCst);
        KittyIds::id(base, self.count.fetch_add(1, Ordering::SeqCst))
    }

    /// Every id handed out so far, oldest first.
    #[cfg(test)]
    fn recorded(&self) -> Vec<u32> {
        let count = self.count.load(Ordering::SeqCst);
        let base = self.base.load(Ordering::SeqCst);
        (0..count).map(|index| KittyIds::id(base, index)).collect()
    }

    /// Every picture handed an id so far was dropped (the picture cache was cleared).
    fn drop_all(&self) {
        self.dropped
            .fetch_max(self.count.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    /// The command that deletes the pictures from the `from`th id up to the `upto`th;
    /// `None` when there is none.
    fn delete_between(&self, from: u32, upto: u32) -> Option<DeleteKittyImages> {
        if from >= upto {
            return None;
        }
        let base = self.base.load(Ordering::SeqCst);
        Some(DeleteKittyImages {
            ids: (from..upto)
                .map(|index| KittyIds::id(base, index))
                .collect(),
            tmux: self.tmux.load(Ordering::SeqCst),
        })
    }

    /// Writes the deletion of the pictures dropped since the last time to `out`, so
    /// the terminal frees them before the next draw. They count as deleted only once
    /// the write succeeded.
    ///
    /// # Errors
    ///
    /// When `out` fails; those pictures are then left for the next call or the exit.
    fn write_dropped(&self, out: &mut impl io::Write) -> io::Result<()> {
        let dropped = self.dropped.load(Ordering::SeqCst);
        let from = self.deleted.load(Ordering::SeqCst);
        if let Some(delete) = self.delete_between(from, dropped) {
            execute!(out, delete)?;
            self.deleted.fetch_max(dropped, Ordering::SeqCst);
        }
        Ok(())
    }

    /// The command that deletes every picture not deleted yet (those still shown, and
    /// dropped ones [`KittyIds::write_dropped`] has not written), and counts them as
    /// deleted, so it is given once; `None` when there is none. It is the last chance
    /// (the exit), so they count as deleted whether or not the write then succeeds.
    fn cleanup(&self) -> Option<DeleteKittyImages> {
        let count = self.count.load(Ordering::SeqCst);
        let from = self.deleted.fetch_max(count, Ordering::SeqCst);
        self.delete_between(from, count)
    }
}

/// The first Kitty image id of the process `pid`: a mix of its bits, so sessions
/// with neighbouring process ids start far apart, in 1..=[`KITTY_ID_SPAN`].
fn kitty_base(pid: u32) -> u32 {
    // The finaliser of MurmurHash3, a bijection on u32 that spreads every bit.
    let mut x = pid;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    x % KITTY_ID_SPAN + 1
}

/// A new image id for a Kitty picture (sent through tmux when `tmux`): nonzero,
/// unique for the session, and deleted by [`leave`] before it leaves the alternate
/// screen, unless [`drop_kitty_pictures`] and [`delete_dropped_kitty_pictures`] deleted
/// it before. Build every Kitty picture with one.
pub fn next_kitty_id(tmux: bool) -> u32 {
    KITTY_IDS.next(tmux)
}

/// Every Kitty picture built so far ([`next_kitty_id`]) was dropped: the picture cache
/// was cleared, and they are not drawn again. [`delete_dropped_kitty_pictures`] then
/// deletes them.
pub fn drop_kitty_pictures() {
    KITTY_IDS.drop_all();
}

/// Writes to `out` the deletion of the Kitty pictures dropped since the last call
/// ([`drop_kitty_pictures`]), so the terminal does not keep them in its image memory
/// until the program ends. Call it before each draw.
///
/// # Errors
///
/// When writing to `out` fails; [`leave`] then deletes those pictures.
pub fn delete_dropped_kitty_pictures(out: &mut impl io::Write) -> io::Result<()> {
    KITTY_IDS.write_dropped(out)
}

/// Every Kitty image id handed out so far.
#[cfg(test)]
pub fn recorded_kitty_ids() -> Vec<u32> {
    KITTY_IDS.recorded()
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
    STATE.store(ACTIVE, Ordering::SeqCst);
    let value = query_then_enable(before_input, || {
        finish_enter(
            original,
            || execute!(stdout(), EnableClickMouse, EnableBracketedPaste),
            leave,
        )
    })?;
    Ok((terminal, value))
}

/// The order [`enter`] keeps once raw mode and the alternate screen are on: first
/// `before_input` (the graphics query), then `enable` (mouse reporting and bracketed
/// paste), so no mouse or paste report can mix into the query's answers.
fn query_then_enable<T>(
    before_input: impl FnOnce() -> T,
    enable: impl FnOnce() -> io::Result<()>,
) -> io::Result<T> {
    let value = before_input();
    enable()?;
    Ok(value)
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

/// Restores the terminal: deletes the Kitty pictures still in the
/// terminal ([`next_kitty_id`]), turns off mouse reporting, bracketed paste and raw
/// mode, leaves the alternate screen and shows the cursor. Every error is ignored and
/// nothing is printed, so this never panics, even on a hung-up tty.
///
/// Only the first call after [`enter`] restores, so every exit path may call it. A
/// call made while another thread restores (the panic hook, the signal thread) waits
/// until that one has finished, so the process does not end halfway through the
/// restore (it gives up after 1 s).
pub fn leave() {
    leave_once(&STATE, LEAVE_WAIT, || {
        restore(&mut stdout(), KITTY_IDS.cleanup(), disable_raw_mode);
    });
}

/// The restore steps of [`leave`], each on its own so that a failed one does not stop
/// the rest: `kitty` (the pictures' deletion, while still on the alternate screen),
/// mouse reporting off, bracketed paste off, `raw_off` (raw mode off, which writes
/// nothing), the alternate screen left (`?1049l`) and the cursor shown (`?25h`).
/// Errors are ignored.
fn restore(
    out: &mut impl io::Write,
    kitty: Option<DeleteKittyImages>,
    raw_off: impl FnOnce() -> io::Result<()>,
) {
    if let Some(delete) = kitty {
        let _ = execute!(out, delete);
    }
    let _ = execute!(out, DisableClickMouse);
    let _ = execute!(out, DisableBracketedPaste);
    let _ = raw_off();
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = execute!(out, Show);
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

/// Runs `restore` if `state` is [`ACTIVE`], marking it [`RESTORING`] meanwhile and
/// [`DONE`] after. When another caller is restoring, waits until it is done, at most
/// `wait` (a restore that panicked, or one stuck writing to a dead terminal, must not
/// keep the process alive). Otherwise does nothing.
fn leave_once(state: &AtomicU8, wait: Duration, restore: impl FnOnce()) {
    match state.compare_exchange(ACTIVE, RESTORING, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => {
            restore();
            state.store(DONE, Ordering::SeqCst);
        }
        Err(RESTORING) => {
            let deadline = Instant::now() + wait;
            while state.load(Ordering::SeqCst) == RESTORING && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
        Err(_) => {}
    }
}

/// Starts the "hangup" thread, which looks at stdin every 100 ms and raises SIGHUP
/// once it finds it closed: hung up, or at end of file for three looks in a row. The program then ends through the hangup path ([`register_signals`]), as
/// when the terminal sent the signal itself. Call it once the terminal is set up; the
/// thread runs until the process ends. On non-Unix platforms it does nothing.
///
/// # Errors
///
/// When the thread cannot be started.
pub fn watch_hangup() -> io::Result<()> {
    #[cfg(unix)]
    thread::Builder::new()
        .name("hangup".to_string())
        .spawn(|| {
            watch_closed(
                || look(io::stdin()),
                HANGUP_LOOK,
                || {
                    let _ = signal_hook::low_level::raise(signal_hook::consts::SIGHUP);
                },
            );
        })
        .map(drop)?;
    Ok(())
}

/// What a look at the terminal's input found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Tty {
    /// Nothing to read, or bytes waiting: a working terminal.
    Open,
    /// Readable with nothing waiting: at end of file, unless the bytes were read
    /// between the two halves of the look.
    AtEof,
    /// Hung up, or failing.
    Closed,
}

/// What a look found: `hung_up` (the poll reported a hangup, an error or a bad file),
/// `readable`, and how many bytes wait to be read (`FIONREAD`).
#[cfg_attr(not(unix), allow(dead_code))]
fn tty_state(hung_up: bool, readable: bool, waiting: io::Result<u64>) -> Tty {
    if hung_up {
        return Tty::Closed;
    }
    if !readable {
        return Tty::Open;
    }
    match waiting {
        Ok(0) => Tty::AtEof,
        Ok(_) => Tty::Open,
        Err(_) => Tty::Closed,
    }
}

/// Looks at `fd` without reading from it: a poll that does not wait, then how many
/// bytes wait. A poll that fails (a signal cut it short) counts as open.
#[cfg(unix)]
fn look(fd: impl std::os::fd::AsFd) -> Tty {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};

    let fd = fd.as_fd();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    if poll(&mut fds, Some(&Timespec::default())).is_err() {
        return Tty::Open;
    }
    let revents = fds[0].revents();
    tty_state(
        revents.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL),
        revents.contains(PollFlags::IN),
        rustix::io::ioctl_fionread(fd).map_err(io::Error::from),
    )
}

/// Takes a `look` every `interval` until it finds the terminal [`Tty::Closed`], or at
/// end of file [`EOF_LOOKS`] times in a row, then calls `on_closed` and returns.
#[cfg_attr(not(unix), allow(dead_code))]
fn watch_closed(mut look: impl FnMut() -> Tty, interval: Duration, on_closed: impl FnOnce()) {
    let mut at_eof = 0;
    loop {
        match look() {
            Tty::Closed => break,
            Tty::AtEof => {
                at_eof += 1;
                if at_eof >= EOF_LOOKS {
                    break;
                }
            }
            Tty::Open => at_eof = 0,
        }
        thread::sleep(interval);
    }
    on_closed();
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
            // is caught by the worker, which plays the local search move instead.
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
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Mutex, PoisonError};

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

    /// A terminal that records what reaches it, or refuses the writes that contain
    /// `refuse` (every write when it is empty), as a tty that hung up does.
    #[derive(Clone, Default)]
    struct Wire {
        bytes: Rc<RefCell<Vec<u8>>>,
        refuse: Option<&'static str>,
        attempts: Rc<Cell<usize>>,
    }

    impl Wire {
        fn refusing(refuse: &'static str) -> Wire {
            Wire {
                refuse: Some(refuse),
                ..Wire::default()
            }
        }

        fn text(&self) -> String {
            String::from_utf8(self.bytes.borrow().clone()).unwrap()
        }
    }

    impl io::Write for Wire {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.attempts.set(self.attempts.get() + 1);
            let refused = self.refuse.is_some_and(|refuse| {
                refuse.is_empty() || String::from_utf8_lossy(buf).contains(refuse)
            });
            if refused {
                return Err(io::Error::other("the tty is gone"));
            }
            self.bytes.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const MOUSE_AND_PASTE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?2004l";
    const SCREEN_AND_CURSOR_BACK: &str = "\x1b[?1049l\x1b[?25h";

    #[test]
    fn restore_writes_exactly_these_bytes_and_turns_raw_mode_off_in_between() {
        // Kitty pictures are deleted first, while still on the alternate screen; raw mode
        // goes off before the alternate screen is left, as `ratatui::restore` does.
        for (kitty, deletes) in [
            (
                Some(DeleteKittyImages {
                    ids: vec![9, 10],
                    tmux: false,
                }),
                "\x1b_Ga=d,d=I,i=9\x1b\\\x1b_Ga=d,d=I,i=10\x1b\\",
            ),
            (None, ""),
        ] {
            let wire = Wire::default();
            let raw_off_after = RefCell::new(None);
            restore(&mut wire.clone(), kitty, || {
                *raw_off_after.borrow_mut() = Some(wire.text());
                Ok(())
            });
            assert_eq!(
                wire.text(),
                format!("{deletes}{MOUSE_AND_PASTE_OFF}{SCREEN_AND_CURSOR_BACK}")
            );
            assert_eq!(
                raw_off_after.take(),
                Some(format!("{deletes}{MOUSE_AND_PASTE_OFF}")),
                "raw mode goes off after ?2004l and before ?1049l"
            );
        }
    }

    #[test]
    fn every_restore_step_runs_when_an_earlier_one_fails() {
        // Mouse reporting cannot be turned off: paste, raw mode, the screen and the
        // cursor are still restored.
        let wire = Wire::refusing("?1006l");
        let raw_off = Cell::new(false);
        restore(
            &mut wire.clone(),
            Some(DeleteKittyImages {
                ids: vec![3],
                tmux: false,
            }),
            || {
                raw_off.set(true);
                Ok(())
            },
        );
        assert_eq!(
            wire.text(),
            format!("\x1b_Ga=d,d=I,i=3\x1b\\\x1b[?2004l{SCREEN_AND_CURSOR_BACK}")
        );
        assert!(raw_off.get());

        // Raw mode cannot be turned off: the screen and the cursor still come back.
        let wire = Wire::default();
        restore(&mut wire.clone(), None, || Err(io::Error::other("no tty")));
        assert_eq!(
            wire.text(),
            format!("{MOUSE_AND_PASTE_OFF}{SCREEN_AND_CURSOR_BACK}")
        );

        // Nothing can be written: every step is still tried, raw mode included.
        let wire = Wire::refusing("");
        let raw_off = Cell::new(false);
        restore(
            &mut wire.clone(),
            Some(DeleteKittyImages {
                ids: vec![3],
                tmux: false,
            }),
            || {
                raw_off.set(true);
                Ok(())
            },
        );
        assert!(raw_off.get(), "raw mode is turned off");
        assert_eq!(
            wire.attempts.get(),
            5,
            "the kitty deletes, mouse, paste, alternate screen and cursor are each tried"
        );
        assert_eq!(wire.text(), "");
    }

    #[cfg(unix)]
    #[test]
    fn kitty_pictures_are_deleted_by_id_and_only_after_kitty_drew() {
        let delete = |ids: &[u32], tmux| {
            ansi(DeleteKittyImages {
                ids: ids.to_vec(),
                tmux,
            })
        };
        // Uppercase I also frees the data; one command per picture, as virtual
        // placements are deleted only by id.
        assert_eq!(
            delete(&[7, 4_000_000_000], false),
            "\x1b_Ga=d,d=I,i=7\x1b\\\x1b_Ga=d,d=I,i=4000000000\x1b\\"
        );
        assert_eq!(
            delete(&[7], true),
            "\x1bPtmux;\x1b\x1b_Ga=d,d=I,i=7\x1b\x1b\\\x1b\\"
        );

        let ids = KittyIds::starting_at(40);
        assert_eq!(ids.cleanup(), None, "no kitty picture was drawn");
        let sent = [ids.next(true), ids.next(false), ids.next(false)];
        assert_eq!(sent, [40, 41, 42]);
        assert_eq!(
            ids.cleanup(),
            Some(DeleteKittyImages {
                ids: sent.to_vec(),
                tmux: false
            }),
            "every id sent, the last way it went"
        );
        assert_eq!(ids.cleanup(), None, "cleaned up once");
    }

    #[test]
    fn kitty_pictures_dropped_by_a_cache_clear_are_deleted_before_the_next_draw() {
        let ids = KittyIds::starting_at(40);
        let mut wire = Vec::new();
        ids.write_dropped(&mut wire).unwrap();
        assert!(wire.is_empty(), "nothing dropped yet");

        let first = [ids.next(false), ids.next(false)];
        assert_eq!(first, [40, 41]);
        // The cache is cleared (a new font): both pictures are gone, and new ones follow.
        ids.drop_all();
        let second = ids.next(false);
        assert_eq!(second, 42);
        ids.write_dropped(&mut wire).unwrap();
        assert_eq!(
            String::from_utf8(wire.clone()).unwrap(),
            "\x1b_Ga=d,d=I,i=40\x1b\\\x1b_Ga=d,d=I,i=41\x1b\\"
        );
        wire.clear();
        ids.write_dropped(&mut wire).unwrap();
        assert!(wire.is_empty(), "each is deleted once");
        assert_eq!(
            ids.cleanup(),
            Some(DeleteKittyImages {
                ids: vec![42],
                tmux: false
            }),
            "exit deletes only the pictures still alive"
        );
        assert_eq!(ids.cleanup(), None);

        // Pictures dropped just before exit, not deleted yet, are deleted by the exit.
        let ids = KittyIds::starting_at(7);
        ids.next(true);
        ids.drop_all();
        ids.next(true);
        assert_eq!(
            ids.cleanup(),
            Some(DeleteKittyImages {
                ids: vec![7, 8],
                tmux: true
            })
        );
        let mut wire = Vec::new();
        ids.write_dropped(&mut wire).unwrap();
        assert!(wire.is_empty(), "the exit deleted them");
    }

    #[test]
    fn dropped_pictures_whose_delete_could_not_be_written_are_deleted_later() {
        let ids = KittyIds::starting_at(40);
        ids.next(false);
        ids.next(false);
        ids.drop_all();
        ids.next(false);
        // The tty refuses the write: the two pictures are not deleted yet.
        let refusing = Wire::refusing("");
        assert!(ids.write_dropped(&mut refusing.clone()).is_err());
        assert_eq!(refusing.text(), "");
        // The next call tries them again.
        let wire = Wire::default();
        ids.write_dropped(&mut wire.clone()).unwrap();
        assert_eq!(
            wire.text(),
            "\x1b_Ga=d,d=I,i=40\x1b\\\x1b_Ga=d,d=I,i=41\x1b\\"
        );

        // Refused again at the last draw: the exit deletes them with the live one.
        let ids = KittyIds::starting_at(40);
        ids.next(false);
        ids.drop_all();
        ids.next(false);
        assert!(ids.write_dropped(&mut Wire::refusing("")).is_err());
        assert_eq!(
            ids.cleanup(),
            Some(DeleteKittyImages {
                ids: vec![40, 41],
                tmux: false
            })
        );
    }

    #[test]
    fn kitty_ids_are_nonzero_unique_and_start_from_the_process_id() {
        // Two sessions in one terminal start far apart, so their ids do not collide.
        let bases: Vec<u32> = [1, 2, 999, 1000, 1001, 65_535, 4_194_304, u32::MAX]
            .into_iter()
            .map(kitty_base)
            .collect();
        for (i, &a) in bases.iter().enumerate() {
            assert!((1..=KITTY_ID_SPAN).contains(&a), "{a}");
            for &b in &bases[i + 1..] {
                assert!(a.abs_diff(b) > 1 << 16, "{a} and {b} are too close");
            }
        }
        // The highest base still gives nonzero ids that do not wrap around.
        let ids = KittyIds::starting_at(KITTY_ID_SPAN);
        let sent: Vec<u32> = (0..3).map(|_| ids.next(false)).collect();
        assert_eq!(sent, [KITTY_ID_SPAN, KITTY_ID_SPAN + 1, KITTY_ID_SPAN + 2]);

        // The session's ids start from its process id's base, and each is new.
        let first = next_kitty_id(false);
        let second = next_kitty_id(false);
        assert_ne!(first, 0);
        assert!(second > first);
        let base = kitty_base(std::process::id());
        assert!(
            first >= base && first - base < KITTY_ID_SPAN,
            "{first} from {base}"
        );
        let recorded = recorded_kitty_ids();
        assert!(recorded.contains(&first) && recorded.contains(&second));
    }

    #[test]
    fn leave_once_restores_only_once() {
        let state = AtomicU8::new(ACTIVE);
        let calls = Cell::new(0);
        leave_once(&state, LEAVE_WAIT, || calls.set(calls.get() + 1));
        leave_once(&state, LEAVE_WAIT, || calls.set(calls.get() + 1));
        assert_eq!(calls.get(), 1);
        assert_eq!(state.load(Ordering::SeqCst), DONE);
    }

    #[test]
    fn leave_once_does_nothing_when_never_entered() {
        let state = AtomicU8::new(DONE);
        leave_once(&state, LEAVE_WAIT, || panic!("must not restore"));
    }

    #[test]
    fn a_second_leave_waits_until_the_first_has_finished() {
        // The panic hook or the signal thread may call `leave` while the main thread is
        // restoring; the process must not end until the restore is done.
        use std::sync::mpsc;
        let state = Arc::new(AtomicU8::new(ACTIVE));
        let restored = Arc::new(AtomicBool::new(false));
        let (started_tx, started) = mpsc::channel();
        let (finish, finish_rx) = mpsc::channel::<()>();
        let first = thread::spawn({
            let (state, restored) = (Arc::clone(&state), Arc::clone(&restored));
            move || {
                leave_once(&state, LEAVE_WAIT, || {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    restored.store(true, Ordering::SeqCst);
                });
            }
        });
        started.recv().unwrap();
        assert_eq!(state.load(Ordering::SeqCst), RESTORING);
        let second = thread::spawn({
            let (state, restored) = (Arc::clone(&state), Arc::clone(&restored));
            move || {
                leave_once(&state, Duration::from_secs(10), || {
                    panic!("only the first caller restores")
                });
                restored.load(Ordering::SeqCst)
            }
        });
        // A second caller that did not wait would return (false) meanwhile.
        thread::sleep(Duration::from_millis(50));
        finish.send(()).unwrap();
        assert!(
            second.join().unwrap(),
            "the second caller returns only after the restore finished"
        );
        first.join().unwrap();
        assert_eq!(state.load(Ordering::SeqCst), DONE);
    }

    #[test]
    fn a_second_leave_waits_at_most_its_limit() {
        use std::time::Instant;
        let state = AtomicU8::new(RESTORING);
        let started = Instant::now();
        leave_once(&state, Duration::from_millis(50), || {
            panic!("only the first caller restores")
        });
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(50), "{waited:?}");
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        assert_eq!(state.load(Ordering::SeqCst), RESTORING, "left to the first");
        assert_eq!(LEAVE_WAIT, Duration::from_secs(1));
    }

    #[test]
    fn leave_and_guard_are_no_ops_without_enter() {
        // No terminal was set up in this process, so neither may write anything.
        assert_eq!(STATE.load(Ordering::SeqCst), DONE);
        leave();
        leave();
        drop(Guard);
        assert_eq!(STATE.load(Ordering::SeqCst), DONE);
    }

    /// Serialises the tests that replace the process-global panic hook.
    static PANIC_HOOK: Mutex<()> = Mutex::new(());

    /// Runs `test` with the panic hook to itself ([`put_the_hook_back_after`]).
    fn with_the_panic_hook(test: impl FnOnce()) {
        let _only_me = PANIC_HOOK.lock().unwrap_or_else(PoisonError::into_inner);
        put_the_hook_back_after(test);
    }

    /// Runs `test`, then puts back the panic hook it found, also when `test` fails (its
    /// message is then raised again, under the hook found).
    fn put_the_hook_back_after(test: impl FnOnce()) {
        let found = panic::take_hook();
        let outcome = panic::catch_unwind(panic::AssertUnwindSafe(test));
        panic::set_hook(found);
        if let Err(payload) = outcome {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_string())
                })
                .unwrap_or_default();
            panic!("{message}");
        }
    }

    #[test]
    fn the_panic_hook_found_is_put_back_even_when_a_test_fails() {
        static MARKS: AtomicI32 = AtomicI32::new(0);
        const MARK: &str = "hook marker";
        with_the_panic_hook(|| {
            panic::set_hook(Box::new(|info| {
                if info.payload().downcast_ref::<&str>() == Some(&MARK) {
                    MARKS.fetch_add(1, Ordering::SeqCst);
                }
            }));
            let failed = panic::catch_unwind(|| {
                put_the_hook_back_after(|| {
                    panic::set_hook(Box::new(|_| {}));
                    panic!("an assertion failed");
                });
            });
            let payload = failed.expect_err("the failure is raised again");
            assert_eq!(
                payload.downcast_ref::<String>().map(String::as_str),
                Some("an assertion failed")
            );
            assert!(panic::catch_unwind(|| panic::panic_any(MARK)).is_err());
            assert_eq!(
                MARKS.load(Ordering::SeqCst),
                1,
                "the hook in place before is back"
            );
        });
    }

    #[test]
    fn a_failed_enable_puts_the_original_panic_hook_back() {
        // A panic on this test thread (not "main") reaches the original hook only when
        // that hook is installed again; the thread-aware hook would just log it.
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
        with_the_panic_hook(|| {
            let undone = Cell::new(false);
            let result = finish_enter(
                probe(),
                || Err(io::Error::other("no mouse")),
                || undone.set(true),
            );
            assert_eq!(result.unwrap_err().to_string(), "no mouse");
            assert!(undone.get(), "the terminal is restored");
            assert!(reaches_original(), "original hook put back");

            let result = finish_enter(probe(), || Ok(()), || panic!("must not restore"));
            assert!(result.is_ok());
            assert!(!reaches_original(), "thread-aware hook installed");
        });
    }

    #[test]
    fn the_query_runs_before_mouse_and_paste_are_enabled() {
        // `enter` calls this once raw mode and the alternate screen are on (the pty
        // smoke test checks the query comes after ?1049h on the wire).
        let steps = RefCell::new(Vec::new());
        let value = query_then_enable(
            || {
                steps.borrow_mut().push("query");
                7
            },
            || {
                steps.borrow_mut().push("enable");
                Ok(())
            },
        );
        assert_eq!(value.unwrap(), 7);
        assert_eq!(*steps.borrow(), ["query", "enable"]);

        let failed = query_then_enable(|| 7, || Err(io::Error::other("no mouse")));
        assert_eq!(failed.unwrap_err().to_string(), "no mouse");
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

    // ----- a terminal closed without SIGHUP -----

    #[test]
    fn a_look_at_the_input_tells_a_closed_terminal() {
        // Hung up (POLLHUP, POLLERR or POLLNVAL): closed, whatever else it says.
        assert_eq!(tty_state(true, true, Ok(3)), Tty::Closed);
        assert_eq!(tty_state(true, false, Ok(0)), Tty::Closed);
        // Nothing to read: open.
        assert_eq!(tty_state(false, false, Ok(0)), Tty::Open);
        // Bytes waiting: open (a key press).
        assert_eq!(tty_state(false, true, Ok(1)), Tty::Open);
        // Readable with nothing waiting: at end of file, unless crossterm read the bytes
        // in between, so it takes a few such looks in a row.
        assert_eq!(tty_state(false, true, Ok(0)), Tty::AtEof);
        // Readable, and asking how much waits fails (EIO): closed.
        assert_eq!(
            tty_state(false, true, Err(io::Error::other("input/output error"))),
            Tty::Closed
        );
    }

    /// How many looks `watch_closed` takes at `script` before it calls `on_closed`,
    /// and how often it calls it; `None` when the script runs out first.
    fn looks_until_closed(script: &[Tty]) -> Option<(usize, usize)> {
        let looks = Cell::new(0);
        let closed = Cell::new(0);
        let outcome = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            watch_closed(
                || {
                    let look = *script.get(looks.get()).expect("script ran out");
                    looks.set(looks.get() + 1);
                    look
                },
                Duration::ZERO,
                || closed.set(closed.get() + 1),
            );
        }));
        outcome.ok().map(|()| (looks.get(), closed.get()))
    }

    #[test]
    fn a_closed_terminal_is_reported_once() {
        use Tty::{AtEof, Closed, Open};
        assert_eq!(looks_until_closed(&[Open, Open, Closed]), Some((3, 1)));
        assert_eq!(
            looks_until_closed(&[Open, AtEof, AtEof, AtEof]),
            Some((4, 1)),
            "three looks in a row at end of file"
        );
        assert_eq!(
            looks_until_closed(&[AtEof, AtEof, Open, AtEof, AtEof, AtEof]),
            Some((6, 1)),
            "a key in between starts the count again"
        );
        assert_eq!(EOF_LOOKS, 3);
        assert!(HANGUP_LOOK <= Duration::from_millis(100), "{HANGUP_LOOK:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_whose_writer_is_gone_looks_at_end_of_file_or_closed() {
        use std::io::{Read, Write};
        let (mut reader, mut writer) = io::pipe().unwrap();
        assert_eq!(look(&reader), Tty::Open, "nothing written yet");
        writer.write_all(b"k").unwrap();
        assert_eq!(look(&reader), Tty::Open, "a byte waits");
        drop(writer);
        let mut byte = [0];
        reader.read_exact(&mut byte).unwrap();
        // macOS reports a readable pipe with nothing in it, Linux a hangup.
        let gone = look(&reader);
        assert!(matches!(gone, Tty::AtEof | Tty::Closed), "{gone:?}");
    }
}
