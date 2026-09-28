//! Graphics detection (spec 9.3): which image protocol the terminal speaks and how
//! large its font is, found out once at start-up. After that only the font size can
//! change (a font zoom), and it matters to Sixel, iTerm2 and Kitty pictures, which are
//! encoded at a pixel size: after a resize the run loop asks the terminal for it again
//! ([`FontMeter::measure`]), the same way.
//!
//! [`detect`] writes ratatui-image's capability query ([`Parser::query`]) and reads the
//! answers itself, on the UI thread: it polls stdin with a deadline of
//! [`QUERY_TIMEOUT`] and feeds each byte to [`Parser::push`] until the status report
//! that ends the answers. It never starts a thread, so nothing is left reading stdin
//! when a terminal does not answer (`Picker::from_query_stdio` would leave one behind,
//! swallowing keystrokes), and it stops reading right after the status report, so keys
//! typed after it reach the event loop. When it stops before the status report (the
//! deadline passed, maybe in the middle of an answer, or a quit signal came), it goes
//! on reading and dropping the answers for up to [`DRAIN_TIME`], so the rest of them
//! neither becomes key presses nor reaches the shell after the program ends.
//!
//! The answers map to a protocol the way ratatui-image maps them: Kitty when the
//! terminal accepted the kitty graphics probe, else Sixel when its device attributes
//! list sixel, else iTerm2 when the environment says so (WezTerm, iTerm2 and a few
//! others), else half-blocks; except that iTerm2 wins over a Sixel answer when the
//! environment names iTerm2 or WezTerm, which draw iTerm2 pictures better. WezTerm and
//! Konsole are never sent the Kitty and Sixel probes (neither draws those correctly).
//! Every query ends with the device attributes request after the status report: the
//! query reads its answer too, but never as a capability, and when the answers come
//! too late for the query, it is the last one and wakes crossterm's reader. A
//! protocol needs a real font size; it comes from the cell-size answer, else from the
//! window's pixel size, and without either the picker draws half-blocks at 10×20. A
//! query that fails or times out never stops the program: it falls back to half-blocks
//! with a warning for the menu, and an answer that arrives after the deadline is kept
//! out of the input ([`LateAnswers`]). Where stdin cannot be polled (not unix) the
//! query is not asked at all, without a warning.

use std::fmt;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{WindowSize, window_size};
use ratatui_image::FontSize;
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};
use ratatui_image::picker::{Picker, ProtocolType};

use super::board::CellSize;
use super::glyphs::ImageSupport;

/// How long start-up waits for the terminal's answers to the graphics query.
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(1);

/// The longest single wait for input while reading the answers, so that a quit
/// signal ends the query within this time rather than at the deadline.
const POLL_SLICE: Duration = Duration::from_millis(50);

/// How long the start-up query ([`detect`]) goes on reading, and dropping, the
/// answers still owed once it stopped waiting for them.
pub const DRAIN_TIME: Duration = Duration::from_millis(200);

/// How long after a query that gave up waiting its answer may still arrive and is
/// kept out of the input ([`LateAnswers`]).
const LATE_ANSWER_WINDOW: Duration = Duration::from_secs(10);

/// The largest font size believed, in pixels per side of a cell. A bigger one is a
/// bogus answer, and would make every picture huge.
const MAX_CELL_PX: u16 = 256;

/// The most key presses a late kitty answer is taken to have after its Alt+`_`;
/// Kitty's answer has 8 (`Gi=31;OK`), an error answer a few dozen.
const MAX_ANSWER_KEYS: usize = 128;

/// What start-up learned about drawing pictures: the input for
/// [`glyphs::initial_glyphs`](super::glyphs::initial_glyphs) and the App.
#[derive(Clone, Debug)]
pub struct Graphics {
    /// Builds the ratatui-image protocol objects for the detected protocol and font
    /// size; `None` when images are off (the query was skipped).
    pub picker: Option<Picker>,
    /// The font size in pixels, which shapes the board's squares. Known also when
    /// images are off, if the window reports its pixel size.
    pub cell_size: CellSize,
    /// Why the query failed, for the menu.
    pub warning: Option<String>,
    /// True when the query timed out or was interrupted and the answers did not come
    /// in [`DRAIN_TIME`] either, so the terminal may still answer; crossterm would read
    /// that answer as key presses ([`LateAnswers`]).
    pub answers_pending: bool,
}

impl Graphics {
    /// Images are off: no picker, only the font size.
    pub const fn off(cell_size: CellSize) -> Graphics {
        Graphics {
            picker: None,
            cell_size,
            warning: None,
            answers_pending: false,
        }
    }

    /// What the glyph choice needs to know: whether there is a picker, and whether
    /// it speaks a graphics protocol or draws half-blocks.
    pub fn support(&self) -> ImageSupport {
        match self.picker.as_ref().map(Picker::protocol_type) {
            None => ImageSupport::Off,
            Some(ProtocolType::Halfblocks) => ImageSupport::Halfblocks,
            Some(_) => ImageSupport::Protocol,
        }
    }
}

impl From<Detection> for Graphics {
    fn from(detection: Detection) -> Graphics {
        Graphics {
            picker: Some(picker_for(detection.protocol, detection.cell_size)),
            cell_size: detection.cell_size,
            warning: detection.warning,
            answers_pending: detection.answers_pending,
        }
    }
}

/// What the answers to the query mean (see [`interpret`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Detection {
    protocol: ProtocolType,
    cell_size: CellSize,
    /// Set when the query failed or timed out; the protocol is then half-blocks.
    warning: Option<String>,
    /// See [`Graphics::answers_pending`].
    answers_pending: bool,
}

/// Why the answers to the query could not be read.
#[derive(Debug)]
enum QueryError {
    /// No status report within the deadline: a terminal that does not answer.
    Timeout,
    /// A quit signal arrived while waiting.
    Interrupted,
    /// Writing the query or reading stdin failed, or stdin ended.
    Io(io::Error),
    /// This platform cannot read the answers, so the query was not asked.
    Unsupported,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::Timeout => write!(f, "no answer within {} s", QUERY_TIMEOUT.as_secs()),
            QueryError::Interrupted => f.write_str("interrupted"),
            QueryError::Io(error) => error.fmt(f),
            QueryError::Unsupported => f.write_str("unsupported"),
        }
    }
}

impl From<io::Error> for QueryError {
    fn from(error: io::Error) -> QueryError {
        QueryError::Io(error)
    }
}

/// Asks the terminal about graphics and builds the picker (spec 9.3). Call it
/// once, on the UI thread, after raw mode and the alternate screen are on (so the
/// answers are neither echoed nor shown) and before mouse capture and bracketed
/// paste (so no mouse or paste report mixes into them). It takes at most
/// [`QUERY_TIMEOUT`], and less once `stop` returns true (checked every 50 ms: a
/// quit signal arrived).
///
/// Never fails: when the query cannot be written or answered, the picker draws
/// half-blocks and [`Graphics::warning`] says why.
pub fn detect(stop: impl Fn() -> bool) -> Graphics {
    let get = |name: &str| std::env::var(name).ok();
    // ratatui-image's own tmux check. Inside tmux it also turns passthrough on, which
    // the kitty probe needs to reach the outer terminal (`from_query_stdio` does the
    // same before its query).
    let is_tmux = Picker::halfblocks().tmux_detected();
    let asked = ask(&query_text(is_tmux, get), stop);
    Graphics::from(detect_from(asked, is_tmux, get, window_cell_size()))
}

/// What [`ask`] found out: the answers ([`interpret`]), and whether some may still come
/// ([`Graphics::answers_pending`]).
fn detect_from(
    asked: Asked,
    is_tmux: bool,
    get: impl Fn(&str) -> Option<String>,
    window: Option<CellSize>,
) -> Detection {
    let mut detection = interpret(asked.answers, is_tmux, get, window);
    detection.answers_pending &= asked.owed;
    detection
}

/// [`Graphics::off`] with the font size from the window's pixel size, for when
/// images are off and the query is skipped.
pub fn without_images() -> Graphics {
    Graphics::off(window_cell_size().unwrap_or_default())
}

/// A picker that draws with `protocol` for a font of `cell_size` pixels.
pub fn picker_for(protocol: ProtocolType, cell_size: CellSize) -> Picker {
    // The only public constructor that takes a font size. Its replacements query the
    // terminal (`from_query_stdio`, see the module documentation) or fix the size at
    // 10×20 (`halfblocks`).
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(FontSize::new(cell_size.width(), cell_size.height()));
    picker.set_protocol_type(protocol);
    picker
}

/// The capability query: the kitty graphics probe, device attributes (sixel),
/// the cell size in pixels and a status report, which every terminal answers and
/// which therefore ends the answers the query interprets. WezTerm and Konsole are not
/// sent the kitty and sixel probes, as ratatui-image does: neither draws those
/// correctly, and WezTerm gets iTerm2 from the environment instead.
///
/// The device attributes are asked for again after the status report, so that the
/// answers end with one that crossterm reports, as [`FontMeter::measure`]'s do:
/// crossterm drops the cell size and status reports, and when they arrive too late
/// for the query, alone in a read, they would leave its reader waiting for the next
/// key. In time, [`ask_with`] reads that answer itself, after the status report, so
/// it is never taken for a capability (WezTerm's lists sixel) nor left for crossterm.
fn query_text(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> String {
    let set = |name: &str| get(name).is_some_and(|value| !value.is_empty());
    let mut options = QueryStdioOptions::default();
    if set("WEZTERM_EXECUTABLE") || set("KONSOLE_VERSION") {
        options.blacklist_protocols = vec![ProtocolType::Kitty, ProtocolType::Sixel];
    }
    let query = Parser::query(is_tmux, options);
    let (_, escape, end) = Parser::tmux_start_escape_end(is_tmux);
    let body = query.strip_suffix(end).unwrap_or(&query);
    format!("{body}{escape}[c{end}")
}

/// Writes `query` to stdout and reads the answers from stdin ([`ask_with`], with
/// [`QUERY_TIMEOUT`] and [`DRAIN_TIME`]). Where the answers cannot be read at all
/// ([`ANSWERS_READABLE`] is false: every platform but unix, see [`read_stdin_byte`]'s
/// stub below), the query is never written either: otherwise the terminal's reply would
/// still land on the wire with nothing here left to read it, and crossterm's event
/// reader would pick it up as ordinary key presses once the event loop starts
/// (`ESC [ ? 1 ; 6 ; ...` reads as `Esc`, `[`, `?`, `1`, ...).
fn ask(query: &str, stop: impl Fn() -> bool) -> Asked {
    ask_with(
        ANSWERS_READABLE,
        write_stdout,
        read_stdin_byte,
        query,
        stop,
        QUERY_TIMEOUT,
        DRAIN_TIME,
    )
}

/// What [`ask_with`] got.
#[derive(Debug)]
struct Asked {
    /// The answers, or why they did not all come.
    answers: Result<Vec<Response>, QueryError>,
    /// Some answers may still come: the query stopped before the status report, and the
    /// drain did not read it either.
    owed: bool,
}

/// Whether this platform's [`read_stdin_byte`] can actually read stdin. Only the unix
/// implementation polls and reads; the `#[cfg(not(unix))]` stub always errors without
/// reading anything, so [`ask`] and [`FontMeter::measure`] must not write their query
/// there either.
#[cfg(unix)]
const ANSWERS_READABLE: bool = true;
#[cfg(not(unix))]
const ANSWERS_READABLE: bool = false;

/// Writes `bytes` to stdout and flushes it.
fn write_stdout(bytes: &[u8]) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes)?;
    stdout.flush()
}

/// [`ask`], with `readable`, `write` and `read_byte` injectable so both of its
/// branches are covered by a test regardless of the platform it runs on: nothing is
/// written when `readable` is false ([`QueryError::Unsupported`]), otherwise `write`
/// runs before `read_byte` is asked for the answers, for up to `timeout` (less once
/// `stop` returns true).
///
/// When it stops before the status report, it goes on reading for up to `drain`,
/// whatever `stop` says, and drops what it reads up to the status report: the rest of
/// an answer the deadline cut in two would reach crossterm as key presses, and answers
/// still on their way at a quit signal would reach the shell. Keys typed meanwhile are
/// lost with them. [`Asked::owed`] says whether the status report is still to come.
///
/// Once the status report is read, it also reads the answer to the device attributes
/// request after it ([`query_text`]), waiting up to `drain` for it, without
/// interpreting it; a byte that cannot be part of it ends that read (and is lost).
fn ask_with(
    readable: bool,
    write: impl FnOnce(&[u8]) -> io::Result<()>,
    mut read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
    query: &str,
    stop: impl Fn() -> bool,
    timeout: Duration,
    drain: Duration,
) -> Asked {
    if !readable {
        return Asked {
            answers: Err(QueryError::Unsupported),
            owed: false,
        };
    }
    if let Err(error) = write(query.as_bytes()) {
        return Asked {
            answers: Err(error.into()),
            owed: false,
        };
    }
    let mut reader = AnswerReader::new(0);
    let answers = reader.read(&mut read_byte, Instant::now() + timeout, &stop);
    if let Err(QueryError::Timeout | QueryError::Interrupted) = answers {
        match reader.read(&mut read_byte, Instant::now() + drain, &|| false) {
            Ok(_) => {}
            Err(QueryError::Timeout | QueryError::Interrupted) => {
                return Asked {
                    answers,
                    owed: true,
                };
            }
            Err(_) => {
                return Asked {
                    answers,
                    owed: false,
                };
            }
        }
    } else if answers.is_err() {
        return Asked {
            answers,
            owed: false,
        };
    }
    read_attributes_answer(&mut read_byte, Instant::now() + drain);
    Asked {
        answers,
        owed: false,
    }
}

/// Reads one device attributes answer (`ESC [ ? <digits and ;> c`) from `read_byte`,
/// waiting until `deadline` at most, and drops it. Stops early at a byte that cannot be
/// part of it, or when reading fails. True when the whole answer was read.
fn read_attributes_answer(
    read_byte: &mut impl FnMut(Duration) -> io::Result<Option<u8>>,
    deadline: Instant,
) -> bool {
    let mut read = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let byte = match read_byte(left.min(POLL_SLICE)) {
            Ok(Some(byte)) => byte,
            Ok(None) => continue,
            Err(_) => return false,
        };
        let fits = match read {
            0 => byte == 0x1b,
            1 => byte == b'[',
            2 => byte == b'?',
            _ if byte == b'c' => return true,
            _ => byte.is_ascii_digit() || byte == b';',
        };
        if !fits {
            return false;
        }
        read += 1;
    }
}

/// Asks the terminal for its font size again after resizes ([`FontMeter::measure`]),
/// and remembers what the measurements that gave up waiting still owe.
///
/// A terminal answers in order. On a slow link the answers of a measurement that gave
/// up may still be on their way when the next one is asked, so that one reads them
/// first: it skips one status report (and the cell size before it) for each
/// measurement that gave up in the last 10 s, and takes the answer after them, its own.
/// When they do not come (crossterm read them in between), it takes the last answer it
/// read once the deadline has passed.
#[derive(Clone, Debug, Default)]
pub struct FontMeter {
    /// How many measurements gave up since the last one that got an answer: the
    /// status reports the terminal may still send ahead of the next one's.
    owed: usize,
    /// Until when those may still come (10 s after the last one gave up); later they
    /// are taken to be read by crossterm or lost.
    until: Option<Instant>,
}

impl FontMeter {
    /// Asks the terminal for its font size again, after a resize: a font zoom changes
    /// the cells, not the window, and Sixel, iTerm2 and Kitty pictures are encoded at the
    /// pixel size of the cells they fill. Call it on the UI thread between batches of
    /// events.
    ///
    /// It writes the cell-size query, the status request and the device attributes
    /// request (`ESC [ 16 t`, `ESC [ 5 n`, `ESC [ c`, wrapped for tmux like the
    /// start-up query) and reads the answers as [`detect`] does, up to its own status
    /// report (see [`FontMeter`]), taking at most [`QUERY_TIMEOUT`], less once `stop`
    /// returns true. The size is the cell-size answer, else the window's pixels per
    /// cell, else `None`: keep the current one.
    ///
    /// The answers need no [`LateAnswers`]: all three are CSI sequences that crossterm
    /// cannot take for a key. It drops the cell size and status reports (ending in `t`
    /// and `n`) and keeps the device attributes to itself. But crossterm's reader
    /// reads stdin until its bytes make an event, so dropped answers alone would leave
    /// it blocked until the next key, click or paste, and the screen would not be
    /// redrawn until then. The device attributes answer comes after them and makes
    /// that event, so whatever answers reach crossterm (this measurement's own device
    /// attributes, or late answers), they end with one. Bytes that arrive while the
    /// measurement reads (keys typed in those milliseconds) are fed to the answer
    /// parser and lost.
    pub fn measure(&mut self, is_tmux: bool, stop: impl Fn() -> bool) -> Option<CellSize> {
        let answers = self.ask_with(
            ANSWERS_READABLE,
            write_stdout,
            read_stdin_byte,
            is_tmux,
            QUERY_TIMEOUT,
            stop,
        );
        measured_font(answers, window_cell_size())
    }

    /// Writes the font query with `write` and reads the answers with `read_byte`
    /// ([`FontMeter::read`]), waiting up to `timeout`. Where the answers cannot be read
    /// (`readable` is false, see [`ANSWERS_READABLE`]) nothing is written or read, as in
    /// [`ask_with`], and nothing is owed.
    fn ask_with(
        &mut self,
        readable: bool,
        write: impl FnOnce(&[u8]) -> io::Result<()>,
        read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
        is_tmux: bool,
        timeout: Duration,
        stop: impl Fn() -> bool,
    ) -> Result<Vec<Response>, QueryError> {
        if !readable {
            return Err(QueryError::Unsupported);
        }
        write(font_query_text(is_tmux).as_bytes())?;
        self.read(read_byte, timeout, stop)
    }

    /// Reads the answers to a measurement with [`read_answers_after`], skipping those
    /// still owed, and keeps count.
    fn read(
        &mut self,
        read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
        timeout: Duration,
        stop: impl Fn() -> bool,
    ) -> Result<Vec<Response>, QueryError> {
        let owed = self.owed_at(Instant::now());
        let answers = read_answers_after(read_byte, timeout, stop, owed);
        self.settle(owed, &answers, Instant::now());
        answers
    }

    /// The status reports still owed at `now`.
    fn owed_at(&self, now: Instant) -> usize {
        match self.until {
            Some(until) if now <= until => self.owed,
            _ => 0,
        }
    }

    /// Counts a measurement that was owed `owed` status reports and read `answers`,
    /// at `now`: one that got an answer settles every debt, one that gave up adds its
    /// own.
    fn settle(&mut self, owed: usize, answers: &Result<Vec<Response>, QueryError>, now: Instant) {
        if answers.is_ok() {
            self.owed = 0;
            self.until = None;
        } else {
            self.owed = owed + 1;
            self.until = Some(now + LATE_ANSWER_WINDOW);
        }
    }
}

/// The query [`FontMeter::measure`] writes: the cell-size query, the status request
/// and the device attributes request, wrapped for tmux like the start-up query.
fn font_query_text(is_tmux: bool) -> String {
    let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
    format!("{start}{escape}[16t{escape}[5n{escape}[c{end}")
}

/// The font size the answers to [`FontMeter::measure`]'s query give: the cell-size answer
/// when it is plausible ([`cell_size_answer`]), else `window` (the window's pixel
/// size per cell); `None` when neither says, or the query failed and the window
/// does not say either, so the current size stays.
fn measured_font(
    answers: Result<Vec<Response>, QueryError>,
    window: Option<CellSize>,
) -> Option<CellSize> {
    answers
        .ok()
        .and_then(|responses| cell_size_answer(&responses))
        .or(window)
}

/// The font size in the last cell-size answer among `responses`, when it is
/// plausible ([`font_size`]).
fn cell_size_answer(responses: &[Response]) -> Option<CellSize> {
    responses
        .iter()
        .rev()
        .find_map(|response| match response {
            Response::CellSize(Some((width, height))) => Some(font_size(*width, *height)),
            _ => None,
        })
        .flatten()
}

/// Feeds the bytes from `read_byte` to the answer parser, skipping `owed` status
/// reports and the answers before each, and returns the answers before the status
/// report numbered `owed + 1`. Nothing after that status report is read, so keys typed
/// later stay for the event loop. When the deadline passes after at least one status
/// report, the answers before the last one read are the result (the owed ones did not
/// come; that one was the query's own).
///
/// `read_byte(wait)` returns the next byte, or `None` when none arrived within
/// `wait`; it is never asked to wait past the deadline `timeout` from now, nor
/// longer than 50 ms at a time. `stop` is checked before every read.
fn read_answers_after(
    mut read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
    timeout: Duration,
    stop: impl Fn() -> bool,
    owed: usize,
) -> Result<Vec<Response>, QueryError> {
    AnswerReader::new(owed).read(&mut read_byte, Instant::now() + timeout, &stop)
}

/// Reads answers up to a status report, in as many calls to [`AnswerReader::read`] as
/// it takes: a call that stops early keeps what it read, a partly read answer included,
/// for the next.
struct AnswerReader {
    parser: Parser,
    /// The status reports to skip, with the answers before each.
    owed: usize,
    /// How many were skipped.
    skipped: usize,
    /// The answers since the last status report.
    responses: Vec<Response>,
    /// The answers before the last status report skipped.
    last: Option<Vec<Response>>,
}

impl AnswerReader {
    /// A reader that skips `owed` status reports first.
    fn new(owed: usize) -> AnswerReader {
        AnswerReader {
            parser: Parser::new(),
            owed,
            skipped: 0,
            responses: Vec::new(),
            last: None,
        }
    }

    /// Feeds the bytes from `read_byte` to the parser until the status report after
    /// those owed arrives, and returns the answers before it; see
    /// [`read_answers_after`]. Gives up at `deadline`, or when `stop` returns true
    /// (checked before every read).
    fn read(
        &mut self,
        read_byte: &mut impl FnMut(Duration) -> io::Result<Option<u8>>,
        deadline: Instant,
        stop: &impl Fn() -> bool,
    ) -> Result<Vec<Response>, QueryError> {
        loop {
            if stop() {
                return Err(QueryError::Interrupted);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return self.last.take().ok_or(QueryError::Timeout);
            }
            let Some(byte) = read_byte(left.min(POLL_SLICE))? else {
                continue;
            };
            // The answers are ASCII; ratatui-image feeds its parser the same way.
            for response in self.parser.push(char::from(byte)) {
                match response {
                    Response::Status if self.skipped == self.owed => {
                        return Ok(std::mem::take(&mut self.responses));
                    }
                    Response::Status => {
                        self.skipped += 1;
                        self.last = Some(std::mem::take(&mut self.responses));
                    }
                    other => self.responses.push(other),
                }
            }
        }
    }
}

/// Maps the answers to a protocol and font size as ratatui-image does
/// (`Picker::from_query_stdio`): the protocol the answers name (Kitty over Sixel),
/// else one the environment names ([`protocol_from_env`]), else half-blocks; the font
/// size from the cell-size answer when it is plausible ([`font_size`]), else `window`
/// (the window's pixel size per cell). Unlike ratatui-image, iTerm2 wins over a Sixel
/// answer when the environment names iTerm2 or WezTerm ([`names_iterm2`]).
///
/// Without any font size the protocol is half-blocks at 10×20, even when the answers
/// name Kitty or Sixel. That is intended (spec 10.3): those protocols draw at the pixel
/// size they are given, and pictures encoded for a guessed font spill into the
/// neighbouring squares or leave gaps. A failed query is half-blocks with a warning,
/// except where the query cannot be asked at all ([`QueryError::Unsupported`]), which
/// the person cannot change.
fn interpret(
    answers: Result<Vec<Response>, QueryError>,
    is_tmux: bool,
    get: impl Fn(&str) -> Option<String>,
    window: Option<CellSize>,
) -> Detection {
    let responses = match answers {
        Ok(responses) => responses,
        Err(QueryError::Unsupported) => {
            return Detection {
                protocol: ProtocolType::Halfblocks,
                cell_size: window.unwrap_or_default(),
                warning: None,
                answers_pending: false,
            };
        }
        Err(error) => {
            return Detection {
                protocol: ProtocolType::Halfblocks,
                cell_size: window.unwrap_or_default(),
                warning: Some(format!("graphics query: {error}; images use half-blocks")),
                answers_pending: matches!(error, QueryError::Timeout | QueryError::Interrupted),
            };
        }
    };
    let mut answered = None;
    for response in &responses {
        match response {
            Response::Kitty => answered = Some(ProtocolType::Kitty),
            Response::Sixel => {
                answered.get_or_insert(ProtocolType::Sixel);
            }
            _ => {}
        }
    }
    let cell_size = cell_size_answer(&responses);
    let (protocol, cell_size) = match cell_size.or(window) {
        Some(cell_size) => {
            let protocol = match answered {
                Some(ProtocolType::Sixel) if names_iterm2(is_tmux, &get) => ProtocolType::Iterm2,
                Some(protocol) => protocol,
                None => protocol_from_env(is_tmux, &get).unwrap_or(ProtocolType::Halfblocks),
            };
            (protocol, cell_size)
        }
        // No font size: half-blocks, whatever answered (see above).
        None => (ProtocolType::Halfblocks, CellSize::DEFAULT),
    };
    Detection {
        protocol,
        cell_size,
        warning: None,
        answers_pending: false,
    }
}

/// Keeps a late answer to the query out of the input. A terminal that answers after
/// [`QUERY_TIMEOUT`] (a slow SSH link) sends its answers to crossterm's reader, which
/// turns the kitty answer `ESC _ G i=31;OK ESC \` into the key presses Alt+`_`, `G`,
/// `i`, `=`, `3`, `1`, `;`, `O`, `K`, Alt+`\`; on the menu the `3` would start a
/// game. The other answers never become key presses: crossterm keeps the device
/// attributes to itself and drops the cell size and status reports (CSI sequences
/// ending in `t` and `n`, which it cannot parse), so a font measurement that gave up
/// waiting needs none of this (see [`FontMeter::measure`] for what it does instead).
///
/// After a query that gave up waiting ([`Graphics::answers_pending`]) and for the
/// next 10 s, it drops one such run of key presses: an Alt+`_`, then printable
/// characters starting with `G`, up to an Alt+`\`. A key that does not fit ends the
/// run and is kept, so an Alt+`_` typed by hand costs only itself.
#[derive(Clone, Debug)]
pub struct LateAnswers {
    /// Until when an answer may start; `None` once one was dropped or none is expected.
    until: Option<Instant>,
    /// The key presses dropped since the answer's Alt+`_`; `None` outside an answer.
    inside: Option<usize>,
}

impl LateAnswers {
    /// Expects a late answer when `graphics` says the query gave up waiting at `now`.
    pub fn after(graphics: &Graphics, now: Instant) -> LateAnswers {
        LateAnswers {
            until: graphics.answers_pending.then(|| now + LATE_ANSWER_WINDOW),
            inside: None,
        }
    }

    /// Whether `event`, read at `now`, is input to keep: false for the key presses
    /// of a late kitty answer. Other events never end or break an answer.
    pub fn keep(&mut self, event: &Event, now: Instant) -> bool {
        let Some(until) = self.until else {
            return true;
        };
        let Event::Key(key) = event else {
            return true;
        };
        match self.inside {
            None if now > until => {
                self.until = None;
                true
            }
            None => {
                let starts = is_alt(key, '_');
                if starts {
                    self.inside = Some(0);
                }
                !starts
            }
            Some(_) if is_alt(key, '\\') => {
                // One answer only: later keys are the person's.
                self.until = None;
                self.inside = None;
                false
            }
            Some(count) if count < MAX_ANSWER_KEYS && is_answer_char(key, count == 0) => {
                self.inside = Some(count + 1);
                false
            }
            Some(_) => {
                self.inside = None;
                true
            }
        }
    }
}

/// Alt+`c`, as crossterm reads `ESC c`.
fn is_alt(key: &KeyEvent, c: char) -> bool {
    key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char(c)
        && key.modifiers.difference(KeyModifiers::SHIFT) == KeyModifiers::ALT
}

/// A printable character as crossterm reads it from an answer; the `first` one is
/// the `G` of a kitty graphics answer.
fn is_answer_char(key: &KeyEvent, first: bool) -> bool {
    let KeyCode::Char(c) = key.code else {
        return false;
    };
    key.kind == KeyEventKind::Press
        && key.modifiers.difference(KeyModifiers::SHIFT).is_empty()
        && (c.is_ascii_graphic() || c == ' ')
        && (!first || c == 'G')
}

/// iTerm2 when the environment says the terminal speaks it, with ratatui-image's
/// rules: inside tmux, when the outer terminal left `ITERM_SESSION_ID` or
/// `WEZTERM_EXECUTABLE`; otherwise when `TERM_PROGRAM` names iTerm2, WezTerm,
/// mintty, VS Code, Tabby, Hyper, Rio, Bobcat or Warp, or `LC_TERMINAL` names
/// iTerm2.
fn protocol_from_env(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> Option<ProtocolType> {
    const TERM_PROGRAMS: [&str; 9] = [
        "iTerm",
        "WezTerm",
        "mintty",
        "vscode",
        "Tabby",
        "Hyper",
        "rio",
        "Bobcat",
        "WarpTerminal",
    ];
    let set = |name: &str| get(name).is_some_and(|value| !value.is_empty());
    let contains = |name: &str, part: &str| get(name).is_some_and(|value| value.contains(part));
    let iterm2 = (is_tmux && (set("ITERM_SESSION_ID") || set("WEZTERM_EXECUTABLE")))
        || TERM_PROGRAMS
            .iter()
            .any(|program| contains("TERM_PROGRAM", program))
        || contains("LC_TERMINAL", "iTerm");
    iterm2.then_some(ProtocolType::Iterm2)
}

/// Whether the environment names iTerm2 or WezTerm, the terminals whose iTerm2
/// pictures win over their Sixel answer: `TERM_PROGRAM` names either, `LC_TERMINAL`
/// names iTerm2, or inside tmux the outer terminal left `ITERM_SESSION_ID` or
/// `WEZTERM_EXECUTABLE`.
fn names_iterm2(is_tmux: bool, get: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| get(name).is_some_and(|value| !value.is_empty());
    let contains = |name: &str, part: &str| get(name).is_some_and(|value| value.contains(part));
    (is_tmux && (set("ITERM_SESSION_ID") || set("WEZTERM_EXECUTABLE")))
        || contains("TERM_PROGRAM", "iTerm")
        || contains("TERM_PROGRAM", "WezTerm")
        || contains("LC_TERMINAL", "iTerm")
}

/// The font size from the terminal's pixel and cell counts, as ratatui-image
/// computes it (rounded down); `None` when the terminal reports no pixel size, or
/// one that gives no plausible font ([`font_size`]).
fn cell_size_from_window(window: WindowSize) -> Option<CellSize> {
    let width = window.width.checked_div(window.columns)?;
    let height = window.height.checked_div(window.rows)?;
    font_size(width, height)
}

/// A font `width` × `height` pixels, or `None` when that cannot be one: a side of
/// zero or over [`MAX_CELL_PX`].
fn font_size(width: u16, height: u16) -> Option<CellSize> {
    let plausible = |side: u16| (1..=MAX_CELL_PX).contains(&side);
    (plausible(width) && plausible(height)).then(|| CellSize::new(width, height))
}

/// The font size from this terminal's pixel and cell counts
/// ([`cell_size_from_window`]); `None` when it reports no pixel size.
fn window_cell_size() -> Option<CellSize> {
    window_size().ok().and_then(cell_size_from_window)
}

/// Waits up to `wait` for stdin to be readable and reads one byte, so nothing
/// after the answers is taken from the event loop. `None` when nothing arrived
/// (or a signal cut the wait short).
#[cfg(unix)]
fn read_stdin_byte(wait: Duration) -> io::Result<Option<u8>> {
    use std::os::fd::AsFd;

    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::io::Errno;

    let stdin = io::stdin();
    let fd = stdin.as_fd();
    let timeout = Timespec::try_from(wait).map_err(io::Error::other)?;
    match poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout)) {
        Ok(0) | Err(Errno::INTR) => return Ok(None),
        Ok(_) => {}
        Err(error) => return Err(error.into()),
    }
    let mut byte = [0u8];
    match rustix::io::read(fd, &mut byte) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(_) => Ok(Some(byte[0])),
        Err(Errno::INTR | Errno::AGAIN) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Reading the answers needs `poll`, which this platform does not have. Passed to
/// [`ask_with`] as `read_byte` for [`ask`] to compile unchanged on every platform, but
/// never actually called there: [`ANSWERS_READABLE`] is false here, so `ask_with`
/// returns before reading (or writing) anything.
#[cfg(not(unix))]
fn read_stdin_byte(_wait: Duration) -> io::Result<Option<u8>> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
    use ratatui::crossterm::terminal::WindowSize;
    use ratatui_image::picker::ProtocolType;
    use ratatui_image::picker::cap_parser::Response;

    use super::*;
    use crate::tui::board::CellSize;
    use crate::tui::glyphs::ImageSupport;
    use crate::tui::test_support::{chord_event, env, key_event, late_kitty_answer};

    /// Kitty 0.39: the graphics probe is accepted, no sixel, 9×18 cells.
    const KITTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n";
    /// Ghostty-like: kitty graphics, more device attributes, a 17×38 font on a HiDPI screen.
    const GHOSTTY: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22;52c\x1b[6;38;17t\x1b[0n";
    /// WezTerm-like: asked only for the cell size (Kitty and Sixel are not probed).
    const WEZTERM: &[u8] = b"\x1b[6;16;8t\x1b[0n";
    /// A sixel terminal (xterm -ti vt340, foot): no kitty answer, `4` in the attributes.
    const SIXEL: &[u8] = b"\x1b[?63;1;2;4;6;9;15;22c\x1b[6;20;10t\x1b[0n";
    /// A terminal that knows none of it (Alacritty-like): attributes and status only.
    const PLAIN: &[u8] = b"\x1b[?6c\x1b[0n";
    /// Kitty's answer to the device attributes request after the status report.
    const KITTY_ATTRIBUTES: &[u8] = b"\x1b[?62;c";
    /// Both kitty graphics and sixel: Kitty wins, as in ratatui-image.
    const KITTY_AND_SIXEL: &[u8] = b"\x1b[?62;4c\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b[0n";

    /// What the query reader returns.
    type Answers = Result<Vec<Response>, QueryError>;

    /// The answers up to the first status report ([`read_answers_after`], nothing owed).
    fn read_answers(
        read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
        timeout: Duration,
        stop: impl Fn() -> bool,
    ) -> Answers {
        read_answers_after(read_byte, timeout, stop, 0)
    }

    /// The answers in `bytes` as the query reader collects them; bytes that end before
    /// the status report are a terminal that never finished answering.
    fn answers(bytes: &[u8]) -> Answers {
        let mut source = VecDeque::from(bytes.to_vec());
        read_answers(
            |_| Ok(source.pop_front()),
            Duration::from_millis(50),
            || false,
        )
    }

    fn detection(protocol: ProtocolType, (width, height): (u16, u16)) -> Detection {
        Detection {
            protocol,
            cell_size: CellSize::new(width, height),
            warning: None,
            answers_pending: false,
        }
    }

    // ----- the query -----

    #[test]
    fn the_query_asks_for_kitty_sixel_the_cell_size_and_status() {
        let query = query_text(false, env(&[]));
        assert!(
            query.starts_with("\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"),
            "{query:?}"
        );
        assert!(
            query.contains("\x1b[c"),
            "device attributes (sixel): {query:?}"
        );
        assert!(query.contains("\x1b[16t"), "cell size: {query:?}");
        // The status report ends the answers the query reads; the device attributes
        // after it make the answers end in one crossterm reports, should they come late.
        assert!(
            query.ends_with("\x1b[16t\x1b[5n\x1b[c"),
            "status report, then the device attributes again: {query:?}"
        );
        assert_eq!(query.matches("\x1b[c").count(), 2, "{query:?}");
        assert!(!query.contains("\x1b]11;?"), "no background colour query");
        assert!(!query.contains("\x1b[6n"), "no text sizing probe");
    }

    #[test]
    fn wezterm_and_konsole_are_not_probed_for_kitty_or_sixel() {
        for pairs in [
            &[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui")][..],
            &[("KONSOLE_VERSION", "240802")],
        ] {
            let query = query_text(false, env(pairs));
            assert!(!query.contains("_Gi="), "{pairs:?}: {query:?}");
            // The device attributes come after the status report, so their answer (which
            // may list sixel) is not read as a capability; it is the last answer, which
            // crossterm reports, so a late one does not leave its reader waiting.
            assert_eq!(query, "\x1b[16t\x1b[5n\x1b[c", "{pairs:?}");
        }
        let query = query_text(true, env(&[("KONSOLE_VERSION", "240802")]));
        assert_eq!(query, "\x1bPtmux;\x1b\x1b[16t\x1b\x1b[5n\x1b\x1b[c\x1b\\");
        // Empty values do not count, as in ratatui-image.
        assert!(query_text(false, env(&[("WEZTERM_EXECUTABLE", "")])).contains("_Gi=31"));
    }

    #[test]
    fn inside_tmux_the_query_is_wrapped_for_passthrough() {
        let query = query_text(true, env(&[]));
        assert!(query.starts_with("\x1bPtmux;\x1b\x1b_Gi=31"), "{query:?}");
        assert!(query.ends_with("\x1b\x1b[5n\x1b\x1b[c\x1b\\"), "{query:?}");
    }

    // ----- asking (writing the query and reading the answers) -----

    #[test]
    fn ask_writes_nothing_where_the_answers_cannot_be_read() {
        let mut written = Vec::new();
        let asked = ask_with(
            false,
            |bytes| {
                written.extend_from_slice(bytes);
                Ok(())
            },
            |_| Ok(None),
            "query bytes",
            || false,
            QUERY_TIMEOUT,
            DRAIN_TIME,
        );
        assert!(
            written.is_empty(),
            "nothing must reach the terminal: {written:?}"
        );
        assert!(
            matches!(asked.answers, Err(QueryError::Unsupported)),
            "{:?}",
            asked.answers
        );
        assert!(!asked.owed);
    }

    #[test]
    fn ask_writes_then_reads_where_the_answers_can_be_read() {
        let mut written = Vec::new();
        let mut source = VecDeque::from(KITTY.to_vec());
        let asked = ask_with(
            true,
            |bytes| {
                written.extend_from_slice(bytes);
                Ok(())
            },
            |_| Ok(source.pop_front()),
            "query bytes",
            || false,
            QUERY_TIMEOUT,
            DRAIN_TIME,
        );
        assert_eq!(written, b"query bytes");
        assert_eq!(
            asked.answers.expect("complete"),
            [Response::Kitty, Response::CellSize(Some((9, 18)))]
        );
        assert!(!asked.owed);
    }

    /// The answers a [`slow_terminal`] has not sent yet.
    type Unread = Rc<RefCell<VecDeque<u8>>>;

    /// A terminal whose answers come in two parts: `early` at once, `late` from `at` on
    /// (a slow link), then nothing.
    fn slow_terminal(
        early: &[u8],
        at: Instant,
        late: &[u8],
    ) -> (
        Unread,
        impl FnMut(Duration) -> io::Result<Option<u8>> + use<>,
    ) {
        let mut early = VecDeque::from(early.to_vec());
        let late = Rc::new(RefCell::new(VecDeque::from(late.to_vec())));
        let unread = Rc::clone(&late);
        let read_byte = move |wait: Duration| {
            if let Some(byte) = early.pop_front() {
                return Ok(Some(byte));
            }
            let now = Instant::now();
            if now >= at
                && let Some(byte) = late.borrow_mut().pop_front()
            {
                return Ok(Some(byte));
            }
            let until_late = at.saturating_duration_since(now);
            std::thread::sleep(if until_late.is_zero() {
                wait
            } else {
                wait.min(until_late)
            });
            Ok(None)
        };
        (unread, read_byte)
    }

    /// Asks with a `timeout` and `drain` time, for tests that run in milliseconds.
    /// A terminal whose answers come in two parts: `early` at once, and `late` only
    /// after the reader has passed its deadline, `timeout` after it started. That time
    /// is counted from the first read, which comes after the reader set its deadline, and
    /// `late` is released only to the reads after one that ended past it: the reader
    /// checks its deadline before every read, so those reads come after the deadline
    /// however slow the machine is.
    fn split_by_the_deadline(
        early: &[u8],
        timeout: Duration,
        late: &[u8],
    ) -> (
        Unread,
        impl FnMut(Duration) -> io::Result<Option<u8>> + use<>,
    ) {
        let mut early = VecDeque::from(early.to_vec());
        let late = Rc::new(RefCell::new(VecDeque::from(late.to_vec())));
        let unread = Rc::clone(&late);
        let mut deadline = None;
        let mut passed = false;
        let read_byte = move |wait: Duration| {
            let deadline = *deadline.get_or_insert_with(|| Instant::now() + timeout);
            if let Some(byte) = early.pop_front() {
                return Ok(Some(byte));
            }
            if passed && let Some(byte) = late.borrow_mut().pop_front() {
                return Ok(Some(byte));
            }
            std::thread::sleep(wait);
            passed = passed || Instant::now() >= deadline;
            Ok(None)
        };
        (unread, read_byte)
    }

    fn ask_quickly(
        read_byte: impl FnMut(Duration) -> io::Result<Option<u8>>,
        stop: impl Fn() -> bool,
        timeout: Duration,
        drain: Duration,
    ) -> Asked {
        ask_with(true, |_| Ok(()), read_byte, "query", stop, timeout, drain)
    }

    #[test]
    fn an_answer_split_by_the_deadline_is_drained() {
        // The deadline falls inside the kitty answer: its tail must not reach crossterm,
        // which would read `3` and start a game. A key typed after the answers is kept.
        let timeout = Duration::from_millis(40);
        let (unread, read_byte) = split_by_the_deadline(
            b"\x1b_Gi=",
            timeout,
            b"31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n\x1b[?62;cq",
        );
        let asked = ask_quickly(read_byte, || false, timeout, Duration::from_secs(5));
        assert!(
            matches!(asked.answers, Err(QueryError::Timeout)),
            "{:?}",
            asked.answers
        );
        assert!(!asked.owed, "the drain read the rest of the answers");
        assert_eq!(
            Vec::from(unread.borrow().clone()),
            b"q",
            "read up to the status report only"
        );
    }

    #[test]
    fn answers_owed_at_a_quit_signal_are_drained() {
        // A quit signal after a few bytes: the rest is read and dropped, so the shell does
        // not get it once the program has ended.
        let mut source = VecDeque::from([KITTY, KITTY_ATTRIBUTES, b"q"].concat());
        let read = Cell::new(0);
        let asked = ask_quickly(
            |_| {
                read.set(read.get() + 1);
                Ok(source.pop_front())
            },
            || read.get() >= 3,
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        assert!(
            matches!(asked.answers, Err(QueryError::Interrupted)),
            "{:?}",
            asked.answers
        );
        assert!(!asked.owed);
        assert_eq!(Vec::from(source), b"q");
    }

    #[test]
    fn the_device_attributes_after_the_status_report_are_read_but_not_as_sixel() {
        // A Sixel terminal's second device attributes answer, after the status report:
        // it is read (so crossterm never sees it), and it names no capability.
        let mut source = VecDeque::from([PLAIN, b"\x1b[?63;4c".as_slice(), b"q"].concat());
        let asked = ask_quickly(
            |_| Ok(source.pop_front()),
            || false,
            Duration::from_secs(5),
            Duration::from_secs(5),
        );
        assert!(!asked.owed);
        assert_eq!(asked.answers.expect("complete"), []);
        assert_eq!(Vec::from(source), b"q", "nothing after it is read");
    }

    #[test]
    fn a_missing_attributes_answer_is_waited_for_at_most_the_drain_time() {
        let mut source = VecDeque::from(KITTY.to_vec());
        let started = Instant::now();
        let asked = ask_quickly(
            |wait| {
                let byte = source.pop_front();
                if byte.is_none() {
                    std::thread::sleep(wait);
                }
                Ok(byte)
            },
            || false,
            Duration::from_secs(5),
            Duration::from_millis(60),
        );
        let took = started.elapsed();
        assert!(asked.answers.is_ok());
        assert!(!asked.owed, "the answers the query reads all came");
        assert!(took >= Duration::from_millis(60), "{took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
    }

    #[test]
    fn the_drain_waits_at_most_its_time() {
        // Nothing comes: after the deadline the drain waits its 200 ms, no longer, and the
        // answers are still owed.
        let started = Instant::now();
        let asked = ask_quickly(
            |wait| {
                std::thread::sleep(wait);
                Ok(None)
            },
            || false,
            Duration::from_millis(20),
            Duration::from_millis(60),
        );
        let took = started.elapsed();
        assert!(matches!(asked.answers, Err(QueryError::Timeout)));
        assert!(asked.owed, "the answers may still come");
        assert!(took >= Duration::from_millis(80), "{took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        // Bytes that never end in a status report do not keep it going either.
        let started = Instant::now();
        let asked = ask_quickly(
            |_| Ok(Some(b'x')),
            || false,
            Duration::from_millis(20),
            Duration::from_millis(60),
        );
        assert!(asked.owed);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(DRAIN_TIME, Duration::from_millis(200));
    }

    #[test]
    fn a_complete_answer_needs_no_drain() {
        let (unread, read_byte) =
            slow_terminal(&[KITTY, KITTY_ATTRIBUTES].concat(), Instant::now(), b"q");
        let asked = ask_quickly(read_byte, || false, Duration::from_secs(5), Duration::ZERO);
        assert!(asked.answers.is_ok());
        assert!(!asked.owed);
        assert_eq!(Vec::from(unread.borrow().clone()), b"q");
    }

    #[test]
    fn late_answers_are_expected_only_when_the_drain_did_not_read_them() {
        let owed = |answers: Answers, owed| {
            detect_from(Asked { answers, owed }, false, env(&[]), None).answers_pending
        };
        assert!(owed(Err(QueryError::Timeout), true));
        assert!(owed(Err(QueryError::Interrupted), true));
        assert!(!owed(Err(QueryError::Timeout), false), "drained");
        assert!(!owed(Err(QueryError::Interrupted), false), "drained");
        assert!(!owed(answers(KITTY), false));
        // The warning stays: the query did not finish in time.
        let drained = detect_from(
            Asked {
                answers: Err(QueryError::Timeout),
                owed: false,
            },
            false,
            env(&[]),
            None,
        );
        assert_eq!(
            drained.warning.as_deref(),
            Some("graphics query: no answer within 1 s; images use half-blocks")
        );
    }

    // ----- reading the answers -----

    #[test]
    fn the_reader_collects_the_answers_up_to_the_status_report() {
        assert_eq!(
            answers(KITTY).expect("complete"),
            [Response::Kitty, Response::CellSize(Some((9, 18)))]
        );
        assert_eq!(
            answers(SIXEL).expect("complete"),
            [Response::Sixel, Response::CellSize(Some((10, 20)))]
        );
        assert_eq!(answers(PLAIN).expect("complete"), []);
    }

    #[test]
    fn keys_typed_after_the_status_report_are_left_for_the_event_loop() {
        let mut source = VecDeque::from(b"\x1b[6;20;10t\x1b[0nq\x1b[A".to_vec());
        let responses = read_answers(|_| Ok(source.pop_front()), Duration::from_secs(5), || false)
            .expect("complete");
        assert_eq!(responses, [Response::CellSize(Some((10, 20)))]);
        assert_eq!(
            source, b"q\x1b[A",
            "nothing after the status report is read"
        );
    }

    #[test]
    fn a_terminal_that_never_answers_times_out() {
        let started = Instant::now();
        let mut waits = Vec::new();
        let result = read_answers(
            |wait| {
                waits.push(wait);
                std::thread::sleep(wait.min(Duration::from_millis(5)));
                Ok(None)
            },
            Duration::from_millis(40),
            || false,
        );
        assert!(matches!(result, Err(QueryError::Timeout)), "{result:?}");
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(40), "{took:?}");
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(
            waits.iter().all(|&wait| wait <= Duration::from_millis(40)),
            "never waits past the deadline: {waits:?}"
        );
    }

    #[test]
    fn answers_cut_off_before_the_status_report_time_out() {
        assert!(matches!(
            answers(b"\x1b_Gi=31;OK\x1b\\\x1b[6;20;"),
            Err(QueryError::Timeout)
        ));
        assert!(matches!(answers(b""), Err(QueryError::Timeout)));
    }

    #[test]
    fn a_long_wait_is_cut_into_short_polls() {
        // A quit signal must end the query within one poll, long before the deadline.
        let mut waits = Vec::new();
        let calls = Cell::new(0);
        let result = read_answers(
            |wait| {
                waits.push(wait);
                calls.set(calls.get() + 1);
                Ok(None)
            },
            Duration::from_secs(10),
            || calls.get() >= 3,
        );
        assert!(matches!(result, Err(QueryError::Interrupted)), "{result:?}");
        assert_eq!(waits.len(), 3);
        assert!(
            waits.iter().all(|&wait| wait <= Duration::from_millis(50)),
            "{waits:?}"
        );
    }

    #[test]
    fn read_errors_end_the_query() {
        let result = read_answers(
            |_| Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Duration::from_secs(5),
            || false,
        );
        let Err(QueryError::Io(error)) = result else {
            panic!("expected an I/O error, got {result:?}");
        };
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }

    // ----- what the answers mean -----

    #[test]
    fn kitty_and_ghostty_get_the_kitty_protocol_and_their_font_size() {
        assert_eq!(
            interpret(answers(KITTY), false, env(&[]), None),
            detection(ProtocolType::Kitty, (9, 18))
        );
        assert_eq!(
            interpret(
                answers(GHOSTTY),
                false,
                env(&[("TERM_PROGRAM", "ghostty")]),
                Some(CellSize::new(8, 16))
            ),
            detection(ProtocolType::Kitty, (17, 38)),
            "the answer wins over the window's pixel size"
        );
        assert_eq!(
            interpret(answers(KITTY_AND_SIXEL), false, env(&[]), None),
            detection(ProtocolType::Kitty, (10, 20))
        );
    }

    #[test]
    fn wezterm_gets_iterm2_from_the_environment() {
        let get = env(&[
            ("TERM_PROGRAM", "WezTerm"),
            ("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui"),
        ]);
        assert_eq!(
            interpret(answers(WEZTERM), false, get, None),
            detection(ProtocolType::Iterm2, (8, 16))
        );
    }

    #[test]
    fn a_sixel_terminal_gets_sixel() {
        assert_eq!(
            interpret(answers(SIXEL), false, env(&[]), None),
            detection(ProtocolType::Sixel, (10, 20))
        );
        // Other terminals the environment names only decide when nothing answers.
        for pairs in [
            &[("TERM_PROGRAM", "vscode")][..],
            &[("TERM_PROGRAM", "mintty")],
        ] {
            assert_eq!(
                interpret(answers(SIXEL), false, env(pairs), None).protocol,
                ProtocolType::Sixel,
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn iterm2_wins_over_a_sixel_answer_when_the_environment_names_iterm2_or_wezterm() {
        // iTerm2 and WezTerm answer the sixel probe too, but draw iTerm2 pictures better.
        for (is_tmux, pairs) in [
            (false, &[("TERM_PROGRAM", "iTerm.app")][..]),
            (false, &[("LC_TERMINAL", "iTerm2")]),
            (false, &[("TERM_PROGRAM", "WezTerm")]),
            (true, &[("ITERM_SESSION_ID", "w0t0p0")]),
            (true, &[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui")]),
        ] {
            assert_eq!(
                interpret(answers(SIXEL), is_tmux, env(pairs), None),
                detection(ProtocolType::Iterm2, (10, 20)),
                "{pairs:?}"
            );
        }
        // A kitty answer still wins.
        let get = env(&[("TERM_PROGRAM", "iTerm.app")]);
        assert_eq!(
            interpret(answers(KITTY_AND_SIXEL), false, get, None).protocol,
            ProtocolType::Kitty
        );
    }

    #[test]
    fn the_environment_names_iterm2_like_ratatui_image_does() {
        let window = Some(CellSize::new(8, 16));
        for pairs in [
            &[("TERM_PROGRAM", "iTerm.app")][..],
            &[("TERM_PROGRAM", "WezTerm")],
            &[("TERM_PROGRAM", "vscode")],
            &[("TERM_PROGRAM", "mintty")],
            &[("TERM_PROGRAM", "WarpTerminal")],
            &[("LC_TERMINAL", "iTerm2")],
        ] {
            assert_eq!(
                interpret(answers(PLAIN), false, env(pairs), window),
                detection(ProtocolType::Iterm2, (8, 16)),
                "{pairs:?}"
            );
        }
        for pairs in [
            &[][..],
            &[("TERM_PROGRAM", "Apple_Terminal")],
            &[("TERM_PROGRAM", "tmux")],
            &[("ITERM_SESSION_ID", "w0t0p0")],
        ] {
            assert_eq!(
                interpret(answers(PLAIN), false, env(pairs), window),
                detection(ProtocolType::Halfblocks, (8, 16)),
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn inside_tmux_the_outer_terminal_is_guessed_from_its_variables() {
        let window = Some(CellSize::new(8, 16));
        for pairs in [
            &[("ITERM_SESSION_ID", "w0t0p0")][..],
            &[("WEZTERM_EXECUTABLE", "/usr/bin/wezterm-gui")],
        ] {
            assert_eq!(
                interpret(answers(PLAIN), true, env(pairs), window).protocol,
                ProtocolType::Iterm2,
                "{pairs:?}"
            );
        }
        assert_eq!(
            interpret(answers(PLAIN), true, env(&[]), window).protocol,
            ProtocolType::Halfblocks
        );
    }

    #[test]
    fn the_font_size_falls_back_to_the_window_then_to_ten_by_twenty() {
        // No cell-size answer: the window's pixels per cell.
        let kitty_without_size: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[0n";
        assert_eq!(
            interpret(
                answers(kitty_without_size),
                false,
                env(&[]),
                Some(CellSize::new(11, 23))
            ),
            detection(ProtocolType::Kitty, (11, 23))
        );
        // Neither: a protocol cannot be drawn at a guessed size, so half-blocks at 10×20.
        assert_eq!(
            interpret(answers(kitty_without_size), false, env(&[]), None),
            detection(ProtocolType::Halfblocks, (10, 20))
        );
        let get = env(&[("TERM_PROGRAM", "iTerm.app")]);
        assert_eq!(
            interpret(answers(PLAIN), false, get, None),
            detection(ProtocolType::Halfblocks, (10, 20))
        );
    }

    #[test]
    fn implausible_font_sizes_are_not_believed() {
        // Over 256 pixels per cell is no font: every picture would be huge. Such a
        // cell-size answer counts as missing, so the window's size is used, else 10×20.
        let huge: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[6;4000;300t\x1b[0n";
        assert_eq!(
            interpret(answers(huge), false, env(&[]), Some(CellSize::new(9, 18))),
            detection(ProtocolType::Kitty, (9, 18))
        );
        assert_eq!(
            interpret(answers(huge), false, env(&[]), None),
            detection(ProtocolType::Halfblocks, (10, 20))
        );
        let window = |columns, rows, width, height| WindowSize {
            rows,
            columns,
            width,
            height,
        };
        assert_eq!(
            cell_size_from_window(window(80, 24, 80 * 256, 24 * 256)),
            Some(CellSize::new(256, 256))
        );
        assert_eq!(cell_size_from_window(window(80, 24, 80 * 257, 480)), None);
        assert_eq!(cell_size_from_window(window(1, 1, 800, 65535)), None);
    }

    #[test]
    fn a_platform_that_cannot_ask_uses_half_blocks_without_a_warning() {
        // Nothing the person can fix, so nothing on the menu.
        assert_eq!(
            interpret(
                Err(QueryError::Unsupported),
                false,
                env(&[("TERM_PROGRAM", "iTerm.app")]),
                Some(CellSize::new(8, 16))
            ),
            detection(ProtocolType::Halfblocks, (8, 16))
        );
        assert_eq!(
            interpret(Err(QueryError::Unsupported), false, env(&[]), None),
            detection(ProtocolType::Halfblocks, (10, 20))
        );
    }

    #[test]
    fn no_answer_falls_back_to_half_blocks_with_a_warning() {
        let silent = interpret(
            Err(QueryError::Timeout),
            false,
            env(&[("TERM_PROGRAM", "iTerm.app")]),
            Some(CellSize::new(8, 16)),
        );
        assert_eq!(silent.protocol, ProtocolType::Halfblocks);
        assert_eq!(silent.cell_size, CellSize::new(8, 16));
        assert_eq!(
            silent.warning.as_deref(),
            Some("graphics query: no answer within 1 s; images use half-blocks")
        );
        let failed = interpret(
            Err(QueryError::Io(io::Error::other("input/output error"))),
            false,
            env(&[]),
            None,
        );
        assert_eq!(failed.protocol, ProtocolType::Halfblocks);
        assert_eq!(failed.cell_size, CellSize::DEFAULT);
        assert_eq!(
            failed.warning.as_deref(),
            Some("graphics query: input/output error; images use half-blocks")
        );
        assert_eq!(
            interpret(answers(b""), false, env(&[]), None)
                .warning
                .as_deref(),
            Some("graphics query: no answer within 1 s; images use half-blocks")
        );
    }

    #[test]
    fn the_window_gives_the_cell_size_when_it_knows_its_pixels() {
        let window = |columns, rows, width, height| WindowSize {
            rows,
            columns,
            width,
            height,
        };
        assert_eq!(
            cell_size_from_window(window(80, 24, 800, 480)),
            Some(CellSize::new(10, 20))
        );
        assert_eq!(
            cell_size_from_window(window(100, 30, 1712, 1140)),
            Some(CellSize::new(17, 38)),
            "rounded down, as ratatui-image does"
        );
        assert_eq!(cell_size_from_window(window(80, 24, 0, 0)), None);
        assert_eq!(cell_size_from_window(window(0, 0, 800, 480)), None);
        assert_eq!(
            cell_size_from_window(window(80, 24, 40, 480)),
            None,
            "under a pixel"
        );
    }

    // ----- measuring the font again -----

    /// A font measurement's answers: a `width` × `height` cell, the status report and
    /// a Sixel terminal's device attributes.
    fn font_answer(width: u16, height: u16) -> Vec<u8> {
        format!("\x1b[6;{height};{width}t\x1b[0n\x1b[?62;4c").into_bytes()
    }

    /// What a measurement reads from `bytes` when `owed` status reports are owed,
    /// and the bytes it leaves unread.
    fn owed_answers(bytes: &[u8], owed: usize) -> (Answers, Vec<u8>) {
        let mut source = VecDeque::from(bytes.to_vec());
        let answers = read_answers_after(
            |_| Ok(source.pop_front()),
            Duration::from_millis(50),
            || false,
            owed,
        );
        (answers, source.into())
    }

    #[test]
    fn the_font_query_asks_for_the_cell_size_the_status_and_the_device_attributes() {
        assert_eq!(font_query_text(false), "\x1b[16t\x1b[5n\x1b[c");
        assert_eq!(
            font_query_text(true),
            "\x1bPtmux;\x1b\x1b[16t\x1b\x1b[5n\x1b\x1b[c\x1b\\"
        );
    }

    #[test]
    fn a_measurement_leaves_only_the_device_attributes_to_crossterm() {
        // crossterm drops the cell size and status reports without making an event, and
        // its reader then waits in `read` for more input: the device attributes after
        // them make the event that ends that wait.
        let (answers, left) = owed_answers(&font_answer(8, 16), 0);
        assert_eq!(
            answers.expect("complete"),
            [Response::CellSize(Some((8, 16)))]
        );
        assert_eq!(left, b"\x1b[?62;4c");
    }

    #[test]
    fn a_measurement_writes_nothing_where_the_answers_cannot_be_read() {
        let mut meter = FontMeter::default();
        let mut written = Vec::new();
        let answers = meter.ask_with(
            false,
            |bytes| {
                written.extend_from_slice(bytes);
                Ok(())
            },
            |_: Duration| -> io::Result<Option<u8>> { panic!("nothing is read") },
            false,
            QUERY_TIMEOUT,
            || false,
        );
        assert!(
            written.is_empty(),
            "nothing must reach the terminal: {written:?}"
        );
        assert!(
            matches!(answers, Err(QueryError::Unsupported)),
            "{answers:?}"
        );
        assert_eq!(
            meter.owed_at(Instant::now()),
            0,
            "no answers are owed by it"
        );
    }

    #[test]
    fn a_measurement_writes_its_query_then_reads_its_answer() {
        let mut meter = FontMeter::default();
        let mut written = Vec::new();
        let mut source = VecDeque::from(font_answer(11, 22));
        let answers = meter.ask_with(
            true,
            |bytes| {
                written.extend_from_slice(bytes);
                Ok(())
            },
            |_| Ok(source.pop_front()),
            true,
            QUERY_TIMEOUT,
            || false,
        );
        assert_eq!(written, font_query_text(true).as_bytes());
        assert_eq!(measured_font(answers, None), Some(CellSize::new(11, 22)));
    }

    #[test]
    fn a_measurement_skips_the_answers_owed_by_one_that_gave_up() {
        // The late answers of a measurement that gave up (8×16) come first, then this
        // one's own (11×22).
        let bytes = [font_answer(8, 16), font_answer(11, 22)].concat();
        let (answers, left) = owed_answers(&bytes, 1);
        assert_eq!(
            measured_font(answers, None),
            Some(CellSize::new(11, 22)),
            "its own answer, not the late one"
        );
        assert_eq!(
            left, b"\x1b[?62;4c",
            "only its own device attributes are left"
        );
        // Two owed, and keys typed in between do not count as answers.
        let bytes = [
            font_answer(8, 16),
            b"q".to_vec(),
            font_answer(9, 18),
            font_answer(11, 22),
        ]
        .concat();
        let (answers, _) = owed_answers(&bytes, 2);
        assert_eq!(measured_font(answers, None), Some(CellSize::new(11, 22)));
    }

    #[test]
    fn owed_answers_that_never_come_leave_the_last_answer_after_the_deadline() {
        // crossterm read the late answers before this measurement: its own answer is the
        // only one, taken once the deadline has passed.
        let (answers, left) = owed_answers(&font_answer(11, 22), 1);
        assert_eq!(measured_font(answers, None), Some(CellSize::new(11, 22)));
        assert_eq!(left, b"", "the device attributes were read while waiting");
        // Nothing at all is a timeout, whatever is owed.
        for owed in [0, 1, 3] {
            let (answers, _) = owed_answers(b"", owed);
            assert!(
                matches!(answers, Err(QueryError::Timeout)),
                "{owed}: {answers:?}"
            );
        }
        // An answer cut off before its status report is none.
        let (answers, _) = owed_answers(b"\x1b[6;22;11t", 1);
        assert!(matches!(answers, Err(QueryError::Timeout)), "{answers:?}");
    }

    #[test]
    fn a_measurement_that_gave_up_is_owed_by_the_next_for_a_while() {
        let now = Instant::now();
        let mut meter = FontMeter::default();
        assert_eq!(meter.owed_at(now), 0);
        meter.settle(0, &Err(QueryError::Timeout), now);
        assert_eq!(meter.owed_at(now), 1);
        meter.settle(1, &Err(QueryError::Timeout), now);
        assert_eq!(
            meter.owed_at(now),
            2,
            "each one that gave up owes its answers"
        );
        assert_eq!(
            meter.owed_at(now + LATE_ANSWER_WINDOW + Duration::from_millis(1)),
            0,
            "answers that late are taken to be read by crossterm or lost"
        );
        meter.settle(2, &Ok(vec![Response::CellSize(Some((8, 16)))]), now);
        assert_eq!(
            meter.owed_at(now),
            0,
            "a measurement that got an answer owes nothing"
        );
        meter.settle(0, &Err(QueryError::Interrupted), now);
        assert_eq!(meter.owed_at(now), 1);
    }

    #[test]
    fn a_resize_after_a_measurement_that_gave_up_gets_its_own_answer() {
        // The pty experiment: a measurement gives up; the next one is asked before the
        // late answers arrive, then gets them and its own.
        let mut meter = FontMeter::default();
        let mut measure = |bytes: &[u8]| {
            let mut source = VecDeque::from(bytes.to_vec());
            let answers = meter.read(
                |_| Ok(source.pop_front()),
                Duration::from_millis(50),
                || false,
            );
            (measured_font(answers, None), Vec::from(source))
        };
        assert_eq!(measure(b""), (None, vec![]), "the first gives up");
        let bytes = [font_answer(8, 16), font_answer(11, 22)].concat();
        assert_eq!(
            measure(&bytes),
            (Some(CellSize::new(11, 22)), b"\x1b[?62;4c".to_vec())
        );
        // Nothing is owed any more: the next takes the first answer.
        assert_eq!(
            measure(&font_answer(9, 18)),
            (Some(CellSize::new(9, 18)), b"\x1b[?62;4c".to_vec())
        );
    }

    #[test]
    fn a_measurement_takes_the_cell_size_answer() {
        const ZOOMED: &[u8] = b"\x1b[6;16;8t\x1b[0n";
        assert_eq!(
            measured_font(answers(ZOOMED), Some(CellSize::DEFAULT)),
            Some(CellSize::new(8, 16)),
            "the answer wins over the window's pixels per cell"
        );
        assert_eq!(
            measured_font(answers(ZOOMED), None),
            Some(CellSize::new(8, 16))
        );
        // Recorded answers from real terminals carry it the same way.
        assert_eq!(
            measured_font(answers(WEZTERM), None),
            Some(CellSize::new(8, 16))
        );
        assert_eq!(
            measured_font(answers(GHOSTTY), None),
            Some(CellSize::new(17, 38))
        );
        assert_eq!(measured_font(answers(SIXEL), None), Some(CellSize::DEFAULT));
    }

    #[test]
    fn a_measurement_without_an_answer_uses_the_window_else_keeps_the_font() {
        let window = Some(CellSize::new(8, 16));
        // A terminal that never answers, one that answers only the status request, and
        // a query that failed or was interrupted.
        let cases: [fn() -> Answers; 5] = [
            || answers(b""),
            || answers(PLAIN),
            || answers(b"\x1b[0n"),
            || Err(QueryError::Interrupted),
            || Err(QueryError::Io(io::Error::other("input/output error"))),
        ];
        for failed in cases {
            assert_eq!(measured_font(failed(), window), window);
            assert_eq!(measured_font(failed(), None), None, "the font stays");
        }
    }

    #[test]
    fn garbage_around_a_measurement_is_not_a_font_size() {
        let window = Some(CellSize::new(8, 16));
        // Keys typed while the query waits do not hide the answer after them.
        assert_eq!(
            measured_font(answers(b"qe4\x1b[A\x1b[6;20;10t\x1b[0n"), window),
            Some(CellSize::DEFAULT)
        );
        // A mangled or implausible answer counts as none.
        for garbage in [
            &b"\x1b[6;x;yt\x1b[0n"[..],
            b"\x1b[6;4000;300t\x1b[0n",
            b"\x1b[6;0;0t\x1b[0n",
            b"\x1b[6t\x1b[0n",
            b"\x01\x7f\xff\x1b\x1b[0n",
        ] {
            assert_eq!(
                measured_font(answers(garbage), window),
                window,
                "{garbage:?}"
            );
            assert_eq!(measured_font(answers(garbage), None), None, "{garbage:?}");
        }
        // Without the status report the answers never end: a timeout.
        assert_eq!(measured_font(answers(b"zz\x1b[6;16;8t"), None), None);
    }

    // ----- the picker -----

    #[test]
    fn the_picker_uses_the_detected_protocol_and_font_size() {
        for protocol in [
            ProtocolType::Kitty,
            ProtocolType::Iterm2,
            ProtocolType::Sixel,
            ProtocolType::Halfblocks,
        ] {
            let picker = picker_for(protocol, CellSize::new(9, 18));
            assert_eq!(picker.protocol_type(), protocol);
            let font = picker.font_size();
            assert_eq!((font.width, font.height), (9, 18));
        }
    }

    #[test]
    fn graphics_report_what_the_glyph_choice_needs() {
        let graphics = Graphics::from(detection(ProtocolType::Kitty, (9, 18)));
        assert_eq!(graphics.support(), ImageSupport::Protocol);
        assert_eq!(graphics.cell_size, CellSize::new(9, 18));
        assert!(graphics.warning.is_none());
        for protocol in [ProtocolType::Iterm2, ProtocolType::Sixel] {
            assert_eq!(
                Graphics::from(detection(protocol, (9, 18))).support(),
                ImageSupport::Protocol
            );
        }

        let fallback = Graphics::from(Detection {
            warning: Some("graphics query: no answer within 1 s; images use half-blocks".into()),
            ..detection(ProtocolType::Halfblocks, (10, 20))
        });
        assert_eq!(fallback.support(), ImageSupport::Halfblocks);
        assert_eq!(
            fallback
                .picker
                .as_ref()
                .map(|picker| picker.protocol_type()),
            Some(ProtocolType::Halfblocks)
        );
        assert!(fallback.warning.is_some());

        let off = Graphics::off(CellSize::new(8, 16));
        assert_eq!(off.support(), ImageSupport::Off);
        assert!(off.picker.is_none());
        assert_eq!(off.cell_size, CellSize::new(8, 16));
        assert!(off.warning.is_none());
    }

    // ----- late answers -----

    fn timed_out() -> Graphics {
        Graphics::from(interpret(Err(QueryError::Timeout), false, env(&[]), None))
    }

    fn char_key(c: char) -> Event {
        key_event(KeyCode::Char(c))
    }

    /// The events of `events` that `late` keeps, all seen at `now`.
    fn kept(late: &mut LateAnswers, events: &[Event], now: Instant) -> Vec<Event> {
        events
            .iter()
            .filter(|event| late.keep(event, now))
            .cloned()
            .collect()
    }

    #[test]
    fn only_a_query_that_gave_up_waiting_expects_late_answers() {
        assert!(timed_out().answers_pending);
        let interrupted = interpret(Err(QueryError::Interrupted), false, env(&[]), None);
        assert!(interrupted.answers_pending);
        assert!(Graphics::from(interrupted).answers_pending);
        let failed = interpret(
            Err(QueryError::Io(io::Error::other("input/output error"))),
            false,
            env(&[]),
            None,
        );
        assert!(!failed.answers_pending, "no answer comes after an error");
        let answered = interpret(answers(KITTY), false, env(&[]), None);
        assert!(!answered.answers_pending);
        assert!(!Graphics::from(answered).answers_pending);
        assert!(!Graphics::off(CellSize::DEFAULT).answers_pending);
    }

    #[test]
    fn a_late_kitty_answer_is_not_typed_and_later_keys_are() {
        let start = Instant::now();
        let mut late = LateAnswers::after(&timed_out(), start);
        let mut events = late_kitty_answer();
        events.insert(4, Event::Resize(100, 30));
        events.push(char_key('1'));
        let at = start + Duration::from_millis(330);
        assert_eq!(
            kept(&mut late, &events, at),
            [Event::Resize(100, 30), char_key('1')],
            "the answer goes, a resize in the middle and the key after it stay"
        );
        assert_eq!(
            kept(&mut late, &late_kitty_answer(), at),
            late_kitty_answer(),
            "one answer only: the same keys again are typed"
        );
    }

    #[test]
    fn late_answers_are_expected_only_after_a_failed_query_and_for_a_while() {
        let start = Instant::now();
        for graphics in [
            Graphics::from(detection(ProtocolType::Kitty, (9, 18))),
            Graphics::off(CellSize::DEFAULT),
        ] {
            let mut late = LateAnswers::after(&graphics, start);
            assert_eq!(
                kept(&mut late, &late_kitty_answer(), start),
                late_kitty_answer()
            );
        }
        let mut late = LateAnswers::after(&timed_out(), start);
        let after_window = start + LATE_ANSWER_WINDOW + Duration::from_millis(1);
        assert_eq!(
            kept(&mut late, &late_kitty_answer(), after_window),
            late_kitty_answer()
        );
    }

    #[test]
    fn keys_that_are_not_a_kitty_answer_are_kept() {
        let start = Instant::now();
        let alt_underscore = chord_event(KeyCode::Char('_'), KeyModifiers::ALT);
        let mut late = LateAnswers::after(&timed_out(), start);
        assert_eq!(
            kept(
                &mut late,
                &[alt_underscore.clone(), char_key('e'), char_key('4')],
                start
            ),
            [char_key('e'), char_key('4')],
            "an answer starts with G"
        );
        let mut late = LateAnswers::after(&timed_out(), start);
        let shift_g = chord_event(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(
            kept(
                &mut late,
                &[
                    alt_underscore,
                    shift_g,
                    key_event(KeyCode::Enter),
                    char_key('x')
                ],
                start
            ),
            [key_event(KeyCode::Enter), char_key('x')],
            "an answer holds only printable characters"
        );
    }

    #[test]
    fn an_answer_that_never_ends_is_cut_off() {
        let start = Instant::now();
        let mut late = LateAnswers::after(&timed_out(), start);
        let mut events = vec![
            chord_event(KeyCode::Char('_'), KeyModifiers::ALT),
            chord_event(KeyCode::Char('G'), KeyModifiers::SHIFT),
        ];
        events.extend(std::iter::repeat_n(char_key('a'), 200));
        let kept = kept(&mut late, &events, start);
        assert_eq!(kept.len(), events.len() - 1 - MAX_ANSWER_KEYS);
        assert!(kept.iter().all(|event| *event == char_key('a')));
    }
}
