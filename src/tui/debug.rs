//! Debug mode (spec 9.4): every Jev request and its answers, kept for the exchange
//! view and appended to a log file.
//!
//! Debug mode is on with `--debug` or `RCHESS_DEBUG` ([`enabled`]). The engine then
//! records each HTTP exchange with Jev (`EngineConfig::trace`), and the worker turns
//! it into an [`Exchange`] on the engine thread, where its body text is also rendered,
//! once. The app keeps the last [`HISTORY_LEN`] of them in a [`History`], replies it
//! discarded as stale included, and `d` shows them full screen ([`ExchangeView`]).
//!
//! Each exchange is also appended to the debug log ([`log_path`]) as one JSON object
//! per line ([`log_line`]). A thread named `debug-log` does the writing, fed by a
//! channel ([`DebugLog`]), so the UI never waits on the disk. The log file is created
//! with the first exchange, in a folder made private to the user; the first error
//! stops the log for the session and is reported once.
//!
//! The API key never reaches any of this: the engine redacts it while recording, so
//! the `Authorization` header reads `Bearer <redacted>`.

use std::collections::VecDeque;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Serialize, Serializer};
use serde_json::Value;

use super::files::{civil_date, describe, expand_tilde};
use super::glyphs::char_width;
use crate::core::Game;
use crate::engine::{ComputerMove, JevExchange};

/// Turns debug mode on unless empty or `0` (see [`enabled`]).
pub const DEBUG_ENV: &str = "RCHESS_DEBUG";
/// The debug log file, when set (see [`log_path`]).
pub const DEBUG_LOG_ENV: &str = "RCHESS_DEBUG_LOG";
/// Exchanges kept for the view; older ones are dropped.
pub const HISTORY_LEN: usize = 50;
/// What the exchange view says before the first exchange.
pub const NO_EXCHANGES: &str = "no Jev requests yet";

/// Name of the thread that writes the log.
const LOG_THREAD: &str = "debug-log";
/// The log's folder under the state folder.
const LOG_DIR: &str = "rchess";
/// The log's file name.
const LOG_FILE: &str = "jev-debug.jsonl";
/// Why there is no log when none of the variables that give its path is set.
pub const NO_LOG_PATH: &str = "no log file (set RCHESS_DEBUG_LOG, XDG_STATE_HOME or HOME)";

/// True when debug mode is on: `--debug` was given (`flag`), or `RCHESS_DEBUG` is set
/// to anything but empty or `0` (surrounding whitespace ignored).
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn enabled(flag: bool, get: impl Fn(&str) -> Option<String>) -> bool {
    flag || get(DEBUG_ENV).is_some_and(|value| !matches!(value.trim(), "" | "0"))
}

/// The debug log file: `RCHESS_DEBUG_LOG` when set, else
/// `$XDG_STATE_HOME/rchess/jev-debug.jsonl`, else `~/.local/state/rchess/jev-debug.jsonl`
/// (on macOS too). Empty variables count as unset, and so does a relative
/// `XDG_STATE_HOME` (the XDG base directory rules say to ignore one). A leading `~/` in
/// `RCHESS_DEBUG_LOG` is `HOME`, as in save paths ([`expand_tilde`]); a relative path
/// stays relative to the working directory.
///
/// # Errors
///
/// Why there is no path, for [`DebugLog::open`] to report: [`NO_LOG_PATH`] when none of
/// the three is set, or the reason `~` cannot be expanded when `RCHESS_DEBUG_LOG`
/// starts with `~/` and `HOME` is not set.
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn log_path(get: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    let set = |name: &str| get(name).filter(|value| !value.is_empty());
    if let Some(path) = set(DEBUG_LOG_ENV) {
        return expand_tilde(&path, set("HOME").as_deref().map(Path::new))
            .map_err(|error| format!("{DEBUG_LOG_ENV}: {error}"));
    }
    let state = set("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| set("HOME").map(|home| Path::new(&home).join(".local").join("state")))
        .ok_or_else(|| NO_LOG_PATH.to_string())?;
    Ok(state.join(LOG_DIR).join(LOG_FILE))
}

/// `time` in UTC as RFC 3339 with milliseconds: `2026-09-27T14:03:05.120Z`. Times before
/// 1970 count back from the epoch.
pub fn rfc3339(time: SystemTime) -> String {
    const MILLIS_PER_DAY: i128 = 86_400_000;
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(since) => i128::try_from(since.as_nanos()).unwrap_or(i128::MAX),
        Err(before) => i128::try_from(before.duration().as_nanos()).map_or(i128::MIN, |n| -n),
    };
    let millis = nanos.div_euclid(1_000_000);
    let days = i64::try_from(millis.div_euclid(MILLIS_PER_DAY)).unwrap_or(if millis < 0 {
        i64::MIN
    } else {
        i64::MAX
    });
    let of_day = millis.rem_euclid(MILLIS_PER_DAY);
    let (year, month, day) = civil_date(days);
    let (hours, minutes) = (of_day / 3_600_000, of_day / 60_000 % 60);
    let (seconds, millis) = (of_day / 1_000 % 60, of_day % 1_000);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

// ----- exchanges -----

/// One Jev exchange as debug mode shows it: the HTTP request and every attempt, as the
/// engine recorded them (the key already redacted), and the move they were for.
#[derive(Clone, Debug, PartialEq)]
pub struct Exchange {
    /// Plies played in the game when the engine was asked.
    pub ply: usize,
    /// The full-move number of the position the engine was asked about.
    pub fullmove: u16,
    /// The chosen move in SAN; played unless the reply was stale.
    pub san: String,
    /// How the move was chosen: the `MoveSource` label (`Jev`,
    /// `vetoed (Jev picked Qh4)` or `local search`).
    pub source: String,
    /// Time for the whole engine call.
    pub latency: Duration,
    /// The request and its attempts.
    pub http: JevExchange,
    /// The view's text, rendered once.
    body: Vec<BodyLine>,
}

impl Exchange {
    /// The exchange `http` that chose `computer` in `game` (the game the engine was
    /// asked about). Renders the view's body text ([`Exchange::body`]).
    pub fn new(game: &Game, computer: &ComputerMove, http: JevExchange) -> Exchange {
        Exchange {
            ply: game.moves().len(),
            fullmove: game.position().fullmove_number(),
            san: computer.san.clone(),
            source: computer.source.to_string(),
            latency: computer.latency,
            body: body_lines(&http),
            http,
        }
    }

    /// The view's text: `REQUEST` (method and URL, the headers, then the JSON body
    /// pretty-printed), then one `RESPONSE` block per attempt (its status or `no
    /// response`, the time it took, the error, and the body: pretty-printed when it is
    /// JSON, else the text as sent). Control characters are escaped (`\u{1b}`), so
    /// nothing a server sends can act on the terminal.
    pub fn body(&self) -> &[BodyLine] {
        &self.body
    }

    /// Attempts made, retries included.
    pub fn attempts(&self) -> usize {
        self.http.attempts.len()
    }

    /// How the last attempt ended: `HTTP <status>`, `no response`, or `not sent` when
    /// there was no attempt.
    pub fn status(&self) -> String {
        match self.http.attempts.last() {
            Some(attempt) => attempt_status(attempt.status),
            None => "not sent".to_string(),
        }
    }
}

/// `HTTP <status>`, or `no response`.
fn attempt_status(status: Option<u16>) -> String {
    status.map_or_else(
        || "no response".to_string(),
        |status| format!("HTTP {status}"),
    )
}

/// How a [`BodyLine`] is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// `REQUEST` and `RESPONSE` block headings.
    Heading,
    /// Why an attempt failed.
    Error,
    /// Everything else.
    Text,
}

/// One line of an exchange's body text, free of control characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyLine {
    /// The text.
    pub text: String,
    /// How it is drawn.
    pub kind: LineKind,
    /// Cells the text takes on one row.
    pub width: usize,
    /// Every character is one cell wide, so rows are simple to count.
    narrow: bool,
}

impl BodyLine {
    /// `text` (which must hold no control characters) drawn as `kind`.
    fn new(text: String, kind: LineKind) -> BodyLine {
        let (width, narrow) = text.chars().fold((0, true), |(width, narrow), c| {
            let w = char_width(c);
            (width + w, narrow && w == 1)
        });
        BodyLine {
            text,
            kind,
            width,
            narrow,
        }
    }

    /// Rows the line takes when wrapped at `width` cells (at least one).
    pub fn rows(&self, width: usize) -> usize {
        let width = width.max(1);
        if self.narrow {
            self.width.div_ceil(width).max(1)
        } else {
            self.wrapped(width).len()
        }
    }

    /// The line cut into rows of at most `width` cells, between any two characters (a
    /// wide character that would straddle the edge starts the next row).
    pub fn wrapped(&self, width: usize) -> Vec<String> {
        let width = width.max(1);
        let mut rows = vec![String::new()];
        let mut used = 0;
        for c in self.text.chars() {
            let w = char_width(c);
            if used > 0 && used + w > width {
                rows.push(String::new());
                used = 0;
            }
            if let Some(row) = rows.last_mut() {
                row.push(c);
            }
            used += w;
        }
        rows
    }
}

/// The body text of `http` (see [`Exchange::body`]).
fn body_lines(http: &JevExchange) -> Vec<BodyLine> {
    let mut lines = Vec::new();
    let mut push = |text: &str, kind: LineKind| push_text(&mut lines, text, kind);
    push("REQUEST", LineKind::Heading);
    push(&format!("{} {}", http.method, http.url), LineKind::Text);
    for (name, value) in &http.headers {
        push(&format!("{name}: {value}"), LineKind::Text);
    }
    push("", LineKind::Text);
    push(&pretty(&http.body), LineKind::Text);
    for (index, attempt) in http.attempts.iter().enumerate() {
        push("", LineKind::Text);
        push(
            &format!(
                "RESPONSE {} · {} · {} ms",
                index + 1,
                attempt_status(attempt.status),
                attempt.elapsed.as_millis()
            ),
            LineKind::Heading,
        );
        if let Some(error) = &attempt.error {
            push(&format!("error: {error}"), LineKind::Error);
        }
        match (&attempt.response, attempt.status) {
            (Some(text), _) if text.is_empty() => push("(empty body)", LineKind::Text),
            (Some(text), _) => match serde_json::from_str::<Value>(text) {
                Ok(json) => push(&pretty(&json), LineKind::Text),
                Err(_) => push(text, LineKind::Text),
            },
            (None, Some(_)) => push("(body not readable)", LineKind::Text),
            (None, None) => {}
        }
    }
    lines
}

/// Appends `text` to `lines`, one [`BodyLine`] per line of it, control characters escaped.
fn push_text(lines: &mut Vec<BodyLine>, text: &str, kind: LineKind) {
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        lines.push(BodyLine::new(escape_controls(line), kind));
    }
}

/// `text` with every control character written as its Rust escape (`\t`, `\u{1b}`).
fn escape_controls(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            escaped.extend(c.escape_default());
        } else {
            escaped.push(c);
        }
    }
    escaped
}

/// `json` pretty-printed with two-space indents.
fn pretty(json: &Value) -> String {
    serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string())
}

// ----- history -----

/// One exchange in the [`History`].
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// 1 for the session's first exchange, counting up; never reused.
    pub number: u64,
    /// When the reply reached the UI.
    pub time: SystemTime,
    /// The reply was discarded, not played (the game had moved on, or it was an illegal move).
    pub stale: bool,
    /// The exchange, shared with the log thread.
    pub exchange: Arc<Exchange>,
}

/// The last [`HISTORY_LEN`] exchanges, oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    records: VecDeque<Record>,
    recorded: u64,
}

impl History {
    /// An empty history.
    #[must_use]
    pub fn new() -> History {
        History::default()
    }

    /// Adds `exchange` as the newest record, dropping the oldest beyond [`HISTORY_LEN`].
    pub fn push(&mut self, exchange: Exchange, stale: bool, time: SystemTime) -> &Record {
        if self.records.len() == HISTORY_LEN {
            self.records.pop_front();
        }
        self.recorded += 1;
        self.records.push_back(Record {
            number: self.recorded,
            time,
            stale,
            exchange: Arc::new(exchange),
        });
        &self.records[self.records.len() - 1]
    }

    /// Records kept.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True before the first exchange.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The record at `index`, oldest first.
    pub fn get(&self, index: usize) -> Option<&Record> {
        self.records.get(index)
    }

    /// The newest record.
    pub fn last(&self) -> Option<&Record> {
        self.records.back()
    }

    /// Where the record numbered `number` is, if it is still kept.
    pub fn index_of(&self, number: u64) -> Option<usize> {
        self.records
            .iter()
            .position(|record| record.number == number)
    }

    /// The records, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Record> {
        self.records.iter()
    }
}

// ----- the view -----

/// Where the exchange view is: which exchange it shows and how far its body is
/// scrolled. The page size and the scroll limit come from the last draw, so the keys
/// stop where the screen does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExchangeView {
    /// The [`Record::number`] shown; `None` while there are no exchanges.
    pub shown: Option<u64>,
    /// Body rows scrolled off the top.
    pub scroll: usize,
    /// Body rows on screen at the last draw.
    pub page: usize,
    /// The largest `scroll` at the last draw: the body's last row at the bottom.
    pub max_scroll: usize,
}

impl ExchangeView {
    /// The view on the newest exchange in `history`, scrolled to the top.
    #[must_use]
    pub fn open(history: &History) -> ExchangeView {
        ExchangeView {
            shown: history.last().map(|record| record.number),
            ..ExchangeView::default()
        }
    }

    /// Where the exchange shown is in `history`: the oldest one kept when it has been
    /// dropped since, `None` when there is none.
    pub fn index(&self, history: &History) -> Option<usize> {
        let number = self.shown?;
        history
            .index_of(number)
            .or_else(|| (!history.is_empty()).then_some(0))
    }

    /// A new exchange has arrived in `history`: the newest is shown when nothing is.
    /// Otherwise the view stays on its exchange, scroll and all, while that is kept; once
    /// it has been dropped, the view moves to the oldest one kept, at its top.
    pub fn follow(&mut self, history: &History) {
        let kept = self.shown.and_then(|number| history.index_of(number));
        match (self.shown, kept) {
            (None, _) => self.show(history.last().map(|record| record.number)),
            (Some(_), Some(_)) => {}
            (Some(_), None) => self.show(history.get(0).map(|record| record.number)),
        }
    }

    /// Shows the next older exchange, if any.
    pub fn older(&mut self, history: &History) {
        if let Some(index) = self.index(history) {
            let number = history.get(index.saturating_sub(1)).map(|r| r.number);
            self.show(number);
        }
    }

    /// Shows the next newer exchange, if any.
    pub fn newer(&mut self, history: &History) {
        if let Some(index) = self.index(history) {
            let newest = history.len().saturating_sub(1);
            let number = history.get((index + 1).min(newest)).map(|r| r.number);
            self.show(number);
        }
    }

    fn show(&mut self, number: Option<u64>) {
        if number != self.shown {
            *self = ExchangeView {
                shown: number,
                ..ExchangeView::default()
            };
        }
    }

    /// Scrolls `rows` rows up.
    pub fn up(&mut self, rows: usize) {
        self.scroll = self.scroll.saturating_sub(rows);
    }

    /// Scrolls `rows` rows down, no further than the body's end.
    pub fn down(&mut self, rows: usize) {
        self.scroll = self.scroll.saturating_add(rows).min(self.max_scroll);
    }

    /// Scrolls a page up; a page keeps one row of the last one.
    pub fn page_up(&mut self) {
        self.up(self.page_step());
    }

    /// Scrolls a page down; a page keeps one row of the last one.
    pub fn page_down(&mut self) {
        self.down(self.page_step());
    }

    /// Scrolls to the top.
    pub fn home(&mut self) {
        self.scroll = 0;
    }

    /// Scrolls to the end.
    pub fn end(&mut self) {
        self.scroll = self.max_scroll;
    }

    fn page_step(&self) -> usize {
        self.page.saturating_sub(1).max(1)
    }
}

// ----- the log -----

/// A log record: the fields in the order the file shows them.
#[derive(Serialize)]
struct LogRecord<'a> {
    time: String,
    ply: usize,
    played: &'a str,
    source: &'a str,
    stale: bool,
    request: LogRequest<'a>,
    attempts: Vec<LogAttempt<'a>>,
}

#[derive(Serialize)]
struct LogRequest<'a> {
    method: &'a str,
    url: &'a str,
    headers: Headers<'a>,
    body: &'a Value,
}

/// Headers as a JSON object, in the order they were sent.
struct Headers<'a>(&'a [(String, String)]);

impl Serialize for Headers<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(name, value)| (name, value)))
    }
}

#[derive(Serialize)]
struct LogAttempt<'a> {
    status: Option<u16>,
    elapsed_ms: u64,
    response: Option<Value>,
    error: Option<&'a str>,
}

/// `record` as one line of JSON (without the line break): `time` (RFC 3339, UTC),
/// `ply`, `played` (SAN), `source`, `stale`, `request` (`method`, `url`, `headers` as an
/// object, `body`) and `attempts` (each `status` or null, `elapsed_ms`, `response` as
/// JSON when it parses, else as a string, or null, and `error` or null).
///
/// # Errors
///
/// Only if serde_json cannot write the record, which it always can.
pub fn log_line(record: &Record) -> serde_json::Result<String> {
    let exchange = &record.exchange;
    let http = &exchange.http;
    serde_json::to_string(&LogRecord {
        time: rfc3339(record.time),
        ply: exchange.ply,
        played: &exchange.san,
        source: &exchange.source,
        stale: record.stale,
        request: LogRequest {
            method: &http.method,
            url: &http.url,
            headers: Headers(&http.headers),
            body: &http.body,
        },
        attempts: http
            .attempts
            .iter()
            .map(|attempt| LogAttempt {
                status: attempt.status,
                elapsed_ms: u64::try_from(attempt.elapsed.as_millis()).unwrap_or(u64::MAX),
                response: attempt.response.as_deref().map(|text| {
                    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
                }),
                error: attempt.error.as_deref(),
            })
            .collect(),
    })
}

/// Why the debug log stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFailure {
    /// A short reason, such as `permission denied`.
    pub reason: String,
    /// The log file, when there is one.
    pub path: Option<PathBuf>,
}

/// The debug log: records go over a channel to the `debug-log` thread, which appends
/// each as a line ([`log_line`]) and flushes it. The file and any missing folders are
/// created with the first record: folders with mode 0700, the file with mode 0600 (an
/// existing file is appended to and made 0600 too; an existing folder keeps its mode).
///
/// The first error ends the thread; [`DebugLog::failure`] then reports it once and
/// nothing more is written. Records never wait: [`DebugLog::write`] only sends.
pub struct DebugLog {
    state: LogState,
}

enum LogState {
    /// The thread is writing.
    Running {
        records: Sender<Record>,
        failures: Receiver<LogFailure>,
        /// Disconnects when the thread ends.
        finished: Receiver<()>,
        path: PathBuf,
    },
    /// No log could be started; reported once a record is written.
    Unavailable(LogFailure),
    /// Nothing more is written; the failure is kept until it is reported.
    Stopped(Option<LogFailure>),
}

impl DebugLog {
    /// The log at `path` ([`log_path`]); without one, a log that reports why there is
    /// no path (the error) at the first record.
    #[must_use]
    pub fn open(path: Result<PathBuf, String>) -> DebugLog {
        match path {
            Ok(path) => DebugLog::start(path),
            Err(reason) => DebugLog {
                state: LogState::Unavailable(LogFailure { reason, path: None }),
            },
        }
    }

    /// Starts the `debug-log` thread appending to `path`. Nothing touches the disk
    /// until the first record.
    #[must_use]
    pub fn start(path: PathBuf) -> DebugLog {
        let (records, queued) = mpsc::channel();
        let (failed, failures) = mpsc::channel();
        let (done, finished) = mpsc::channel::<()>();
        let file = path.clone();
        let spawned = thread::Builder::new()
            .name(LOG_THREAD.to_string())
            .spawn(move || {
                // Dropped when the thread ends, however it ends.
                let _done = done;
                if let Err(failure) = write_records(&file, &queued) {
                    // Nobody is left to tell once the log has been closed.
                    let _ = failed.send(failure);
                }
            });
        let state = match spawned {
            Ok(_) => LogState::Running {
                records,
                failures,
                finished,
                path,
            },
            Err(error) => LogState::Unavailable(LogFailure {
                reason: format!("cannot start the {LOG_THREAD} thread ({error})"),
                path: Some(path),
            }),
        };
        DebugLog { state }
    }

    /// Queues `record` for the log; never blocks.
    pub fn write(&mut self, record: &Record) {
        match &mut self.state {
            LogState::Running { records, .. } => {
                // Fails only once the thread has stopped, and `failure` says why.
                let _ = records.send(record.clone());
            }
            LogState::Unavailable(failure) => {
                let failure = failure.clone();
                self.state = LogState::Stopped(Some(failure));
            }
            LogState::Stopped(_) => {}
        }
    }

    /// Why the log stopped, the first time this is asked after it did; `None` while it
    /// works and ever after.
    pub fn failure(&mut self) -> Option<LogFailure> {
        match &mut self.state {
            LogState::Running { failures, path, .. } => {
                let failure = match failures.try_recv() {
                    Ok(failure) => failure,
                    Err(TryRecvError::Empty) => return None,
                    // The thread ended without saying why: it panicked.
                    Err(TryRecvError::Disconnected) => LogFailure {
                        reason: format!("the {LOG_THREAD} thread stopped"),
                        path: Some(path.clone()),
                    },
                };
                self.state = LogState::Stopped(None);
                Some(failure)
            }
            LogState::Stopped(failure) => failure.take(),
            LogState::Unavailable(_) => None,
        }
    }

    /// Stops taking records and waits up to `grace` for the thread to write the ones
    /// queued, so the last exchanges of a session reach the file.
    pub fn close(self, grace: Duration) {
        if let LogState::Running {
            records, finished, ..
        } = self.state
        {
            drop(records);
            // Disconnected once the thread has written everything and ended.
            let _ = finished.recv_timeout(grace);
        }
    }
}

/// The `debug-log` thread's work: appends every record from `queued` to `path` until
/// the channel closes or a write fails.
fn write_records(path: &Path, queued: &Receiver<Record>) -> Result<(), LogFailure> {
    let fail = |error: io::Error| LogFailure {
        reason: describe(&error),
        path: Some(path.to_path_buf()),
    };
    let mut file = None;
    for record in queued {
        let file = match &mut file {
            Some(file) => file,
            empty => empty.insert(open_log(path).map_err(fail)?),
        };
        append(file, &record).map_err(fail)?;
    }
    Ok(())
}

/// Opens `path` for appending, creating it and its missing folders (mode 0700) as
/// needed. The file gets mode 0600, also when it was already there with a looser one;
/// folders that were already there keep theirs (one may be the user's
/// `~/.local/state`). Only a regular file is changed: a device such as `/dev/null`
/// is written as it is.
fn open_log(path: &Path) -> io::Result<File> {
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        let mut builder = DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(folder)?;
    }
    let mut options = OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = file.metadata()?;
        if metadata.is_file() && metadata.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(file)
}

/// Writes `record` as one line and flushes it.
fn append(file: &mut File, record: &Record) -> io::Result<()> {
    let mut line = log_line(record).map_err(io::Error::other)?;
    line.push('\n');
    file.write_all(line.as_bytes())?;
    file.flush()
}

// ----- the session -----

/// Debug mode's state in the app: the history and the log.
pub struct DebugSession {
    history: History,
    log: DebugLog,
}

impl DebugSession {
    /// A session with no exchanges yet, logging to `log`.
    #[must_use]
    pub fn new(log: DebugLog) -> DebugSession {
        DebugSession {
            history: History::new(),
            log,
        }
    }

    /// The exchanges kept.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Keeps `exchange` (received at `time`, `stale` when it was not played) and queues
    /// it for the log. Returns its [`Record::number`].
    pub fn record(&mut self, exchange: Exchange, stale: bool, time: SystemTime) -> u64 {
        let record = self.history.push(exchange, stale, time);
        self.log.write(record);
        record.number
    }

    /// See [`DebugLog::failure`].
    pub fn log_failure(&mut self) -> Option<LogFailure> {
        self.log.failure()
    }

    /// Closes the log ([`DebugLog::close`]); later exchanges are kept but not logged.
    pub fn close_log(&mut self, grace: Duration) {
        let stopped = DebugLog {
            state: LogState::Stopped(None),
        };
        std::mem::replace(&mut self.log, stopped).close(grace);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Instant;

    use serde_json::json;

    use super::*;
    use crate::engine::{JevAttempt, recorded_exchange};
    use crate::tui::test_support::engine::{SENTINEL_KEY, jev_exchange, jev_move};
    use crate::tui::test_support::{TempDir, env};

    /// `ms` milliseconds after the epoch.
    fn at_ms(ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(ms)
    }

    /// The exchange [`jev_exchange`] behind Jev's e4 in the start position.
    fn exchange() -> Exchange {
        let game = Game::new();
        let computer = jev_move(game.position(), "e2e4");
        Exchange::new(&game, &computer, jev_exchange())
    }

    fn record(stale: bool) -> Record {
        let mut history = History::new();
        history
            .push(exchange(), stale, at_ms(1_790_000_000_123))
            .clone()
    }

    fn texts(exchange: &Exchange) -> Vec<&str> {
        exchange
            .body()
            .iter()
            .map(|line| line.text.as_str())
            .collect()
    }

    /// Asks `log` for its failure until one comes, for up to 10 s.
    fn wait_for_failure(log: &mut DebugLog) -> LogFailure {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(failure) = log.failure() {
                return failure;
            }
            assert!(Instant::now() < deadline, "no failure reported");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    // ----- switches and paths -----

    #[test]
    fn debug_mode_is_on_with_the_flag_or_a_set_variable() {
        assert!(!enabled(false, env(&[])));
        assert!(enabled(true, env(&[])));
        assert!(enabled(true, env(&[(DEBUG_ENV, "0")])), "the flag wins");
        for on in ["1", "yes", "true", "00", " 1 ", "off"] {
            assert!(enabled(false, env(&[(DEBUG_ENV, on)])), "{on:?}");
        }
        for off in ["", "0", " 0 ", "  "] {
            assert!(!enabled(false, env(&[(DEBUG_ENV, off)])), "{off:?}");
        }
    }

    #[test]
    fn the_log_path_follows_the_variables_in_order() {
        let log = |pairs: &[(&str, &str)]| log_path(env(pairs)).ok();
        let all = [
            (DEBUG_LOG_ENV, "/tmp/mine.jsonl"),
            ("XDG_STATE_HOME", "/xdg/state"),
            ("HOME", "/home/ana"),
        ];
        assert_eq!(log(&all), Some(PathBuf::from("/tmp/mine.jsonl")));
        assert_eq!(
            log(&[(DEBUG_LOG_ENV, "relative/debug.jsonl")]),
            Some(PathBuf::from("relative/debug.jsonl")),
            "used as given"
        );
        assert_eq!(
            log(&all[1..]),
            Some(PathBuf::from("/xdg/state/rchess/jev-debug.jsonl"))
        );
        assert_eq!(
            log(&[(DEBUG_LOG_ENV, ""), ("XDG_STATE_HOME", "/xdg/state")]),
            Some(PathBuf::from("/xdg/state/rchess/jev-debug.jsonl")),
            "an empty RCHESS_DEBUG_LOG is unset"
        );
        assert_eq!(
            log(&all[2..]),
            Some(PathBuf::from(
                "/home/ana/.local/state/rchess/jev-debug.jsonl"
            ))
        );
        for xdg in ["", "relative/state"] {
            assert_eq!(
                log(&[("XDG_STATE_HOME", xdg), ("HOME", "/home/ana")]),
                Some(PathBuf::from(
                    "/home/ana/.local/state/rchess/jev-debug.jsonl"
                )),
                "XDG_STATE_HOME {xdg:?} is ignored"
            );
        }
        for pairs in [&[][..], &[("HOME", ""), ("XDG_STATE_HOME", "state")]] {
            assert_eq!(
                log_path(env(pairs)),
                Err(NO_LOG_PATH.to_string()),
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn times_are_rfc_3339_in_utc_with_milliseconds() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            rfc3339(at_ms(1_790_000_000_123)),
            "2026-09-21T14:13:20.123Z"
        );
        // 2024-02-29 is a leap day; 23:59:59.999 is its last millisecond.
        assert_eq!(
            rfc3339(at_ms(1_709_251_199_999)),
            "2024-02-29T23:59:59.999Z"
        );
        // Sub-millisecond parts are cut, also before the epoch.
        assert_eq!(
            rfc3339(UNIX_EPOCH + Duration::from_micros(1_999)),
            "1970-01-01T00:00:00.001Z"
        );
        assert_eq!(
            rfc3339(UNIX_EPOCH - Duration::from_micros(1)),
            "1969-12-31T23:59:59.999Z"
        );
    }

    // ----- exchanges -----

    #[test]
    fn an_exchange_names_the_move_it_was_for() {
        let game =
            crate::tui::test_support::game_from(crate::core::START_FEN, &["e2e4", "e7e5", "g1f3"]);
        let computer = jev_move(game.position(), "b8c6");
        let exchange = Exchange::new(&game, &computer, jev_exchange());
        assert_eq!(exchange.ply, 3);
        assert_eq!(exchange.fullmove, 2);
        assert_eq!(exchange.san, "Nc6");
        assert_eq!(exchange.source, "Jev");
        assert_eq!(exchange.latency, Duration::from_millis(1234));
        assert_eq!(exchange.http, jev_exchange());
        assert_eq!(exchange.attempts(), 2);
        assert_eq!(exchange.status(), "HTTP 200");
    }

    #[test]
    fn the_body_shows_the_request_and_every_response() {
        let exchange = exchange();
        let lines = texts(&exchange);
        let pretty_body = serde_json::to_string_pretty(&jev_exchange().body).unwrap();
        let body_lines: Vec<&str> = pretty_body.lines().collect();
        let mut expected = vec![
            "REQUEST",
            "POST https://api.typesafe.ai/v1/systemone",
            "Authorization: Bearer <redacted>",
            "Content-Type: application/json",
            "",
        ];
        expected.extend(&body_lines);
        expected.extend([
            "",
            "RESPONSE 1 · HTTP 503 · 120 ms",
            "error: HTTP 503: busy \\u{1b}[2J now",
            "busy \\u{1b}[2J now",
            "\\ttry again",
            "",
            "RESPONSE 2 · HTTP 200 · 850 ms",
            "{",
            "  \"answers\": {",
        ]);
        assert_eq!(&lines[..expected.len()], &expected[..]);
        assert_eq!(lines.last(), Some(&"}"));
        let kinds: Vec<LineKind> = exchange.body().iter().map(|line| line.kind).collect();
        assert_eq!(kinds[0], LineKind::Heading);
        let error = expected.len() - 7;
        assert_eq!(
            kinds[error - 1..=error],
            [LineKind::Heading, LineKind::Error]
        );
        assert!(
            exchange
                .body()
                .iter()
                .all(|line| !line.text.chars().any(char::is_control)),
            "control characters are escaped"
        );
    }

    #[test]
    fn failed_and_empty_responses_say_so() {
        let mut http = jev_exchange();
        http.attempts = vec![
            JevAttempt {
                status: None,
                response: None,
                error: Some("network error: connection refused".to_string()),
                elapsed: Duration::from_millis(5),
            },
            JevAttempt {
                status: Some(502),
                response: None,
                error: Some("HTTP 502: <unreadable body>".to_string()),
                elapsed: Duration::from_millis(7),
            },
            JevAttempt {
                status: Some(500),
                response: Some(String::new()),
                error: Some("HTTP 500: ".to_string()),
                elapsed: Duration::from_millis(9),
            },
        ];
        let game = Game::new();
        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http);
        let lines = texts(&exchange);
        let start = lines
            .iter()
            .position(|line| line.starts_with("RESPONSE 1"))
            .expect("a response block");
        assert_eq!(
            lines[start..],
            [
                "RESPONSE 1 · no response · 5 ms",
                "error: network error: connection refused",
                "",
                "RESPONSE 2 · HTTP 502 · 7 ms",
                "error: HTTP 502: <unreadable body>",
                "(body not readable)",
                "",
                "RESPONSE 3 · HTTP 500 · 9 ms",
                "error: HTTP 500: ",
                "(empty body)",
            ]
        );
        assert_eq!(exchange.status(), "HTTP 500");
        let mut http = jev_exchange();
        http.attempts.truncate(1);
        http.attempts[0].status = None;
        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http.clone());
        assert_eq!(exchange.status(), "no response");
        http.attempts.clear();
        let exchange = Exchange::new(&game, &jev_move(game.position(), "e2e4"), http);
        assert_eq!(exchange.status(), "not sent");
        assert_eq!(exchange.attempts(), 0);
    }

    #[test]
    fn body_lines_wrap_between_characters() {
        let line = BodyLine::new("abcdefghij".to_string(), LineKind::Text);
        assert_eq!(line.width, 10);
        assert_eq!(line.rows(4), 3);
        assert_eq!(line.wrapped(4), ["abcd", "efgh", "ij"]);
        assert_eq!(line.rows(10), 1);
        assert_eq!(line.rows(0), 10, "at least one cell per row");
        let empty = BodyLine::new(String::new(), LineKind::Text);
        assert_eq!(empty.rows(8), 1);
        assert_eq!(empty.wrapped(8), [""]);
        // A wide character never straddles the edge.
        let wide = BodyLine::new("ab日本cd".to_string(), LineKind::Text);
        assert_eq!(wide.width, 8);
        assert_eq!(wide.wrapped(3), ["ab", "日", "本c", "d"]);
        assert_eq!(wide.rows(3), 4);
    }

    // ----- history and view -----

    #[test]
    fn the_history_keeps_the_last_fifty_and_numbers_them() {
        let mut history = History::new();
        assert!(history.is_empty());
        for n in 1..=55_u64 {
            let record = history.push(exchange(), n % 10 == 0, at_ms(n));
            assert_eq!(record.number, n);
        }
        assert_eq!(history.len(), HISTORY_LEN);
        assert_eq!(history.get(0).map(|r| r.number), Some(6));
        assert_eq!(history.last().map(|r| r.number), Some(55));
        assert_eq!(history.index_of(6), Some(0));
        assert_eq!(history.index_of(5), None, "dropped");
        assert_eq!(history.index_of(55), Some(49));
        let stale: Vec<u64> = history
            .iter()
            .filter(|r| r.stale)
            .map(|r| r.number)
            .collect();
        assert_eq!(stale, [10, 20, 30, 40, 50]);
    }

    #[test]
    fn the_view_steps_between_exchanges_and_stays_on_its_own() {
        let mut history = History::new();
        let mut view = ExchangeView::open(&history);
        assert_eq!(view.shown, None);
        assert_eq!(view.index(&history), None);
        view.older(&history);
        view.newer(&history);
        assert_eq!(view.shown, None);

        history.push(exchange(), false, at_ms(1));
        view.follow(&history);
        assert_eq!(view.shown, Some(1), "the first exchange is shown at once");
        for n in 2..=3 {
            history.push(exchange(), false, at_ms(n));
            view.follow(&history);
        }
        assert_eq!(view.shown, Some(1), "later ones do not move the view");
        view.newer(&history);
        view.newer(&history);
        view.newer(&history);
        assert_eq!(view.index(&history), Some(2), "stops at the newest");
        view.older(&history);
        assert_eq!(view.shown, Some(2));

        let view = ExchangeView::open(&history);
        assert_eq!(view.shown, Some(3), "opens on the newest");
    }

    #[test]
    fn a_dropped_exchange_leaves_the_view_on_the_oldest() {
        let mut history = History::new();
        history.push(exchange(), false, at_ms(1));
        history.push(exchange(), false, at_ms(2));
        let mut view = ExchangeView::open(&history);
        view.older(&history);
        (view.page, view.max_scroll) = (10, 40);
        view.down(7);
        assert_eq!((view.shown, view.scroll), (Some(1), 7));
        // While the exchange shown is kept, new ones leave the view and its scroll alone.
        for n in 3..=50 {
            history.push(exchange(), false, at_ms(n));
            view.follow(&history);
        }
        assert_eq!((view.shown, view.scroll), (Some(1), 7));
        // Once it is dropped, the view moves to the oldest kept, at its top.
        history.push(exchange(), false, at_ms(51));
        view.follow(&history);
        assert_eq!(history.index_of(1), None);
        assert_eq!((view.shown, view.scroll), (Some(2), 0));
        assert_eq!(view.index(&history), Some(0));
        view.down(3);
        history.push(exchange(), false, at_ms(52));
        view.follow(&history);
        assert_eq!((view.shown, view.scroll), (Some(3), 0), "and again");
        view.newer(&history);
        assert_eq!(view.shown, Some(4));

        // A view that missed the drop (no follow) still shows the oldest kept.
        let mut view = ExchangeView {
            shown: Some(1),
            ..ExchangeView::default()
        };
        assert_eq!(view.index(&history), Some(0));
        view.newer(&history);
        assert_eq!(view.shown, Some(4));
    }

    #[test]
    fn scrolling_stops_at_the_ends_and_resets_on_another_exchange() {
        let mut history = History::new();
        for n in 1..=2 {
            history.push(exchange(), false, at_ms(n));
        }
        let mut view = ExchangeView::open(&history);
        view.page = 10;
        view.max_scroll = 25;
        view.down(1);
        assert_eq!(view.scroll, 1);
        view.page_down();
        assert_eq!(view.scroll, 10, "a page keeps one row");
        view.end();
        assert_eq!(view.scroll, 25);
        view.down(3);
        view.page_down();
        assert_eq!(view.scroll, 25);
        view.up(1);
        assert_eq!(view.scroll, 24, "one up from the end");
        view.page_up();
        assert_eq!(view.scroll, 15);
        view.home();
        assert_eq!(view.scroll, 0);
        view.up(5);
        view.page_up();
        assert_eq!(view.scroll, 0);

        view.page = 1;
        view.page_down();
        assert_eq!(view.scroll, 1, "a page is at least one row");
        view.end();
        view.older(&history);
        assert_eq!(
            view,
            ExchangeView {
                shown: Some(1),
                ..ExchangeView::default()
            }
        );
        view.older(&history);
        view.max_scroll = 25;
        view.end();
        view.older(&history);
        assert_eq!(view.scroll, 25, "already the oldest: nothing moves");
    }

    // ----- log records -----

    #[test]
    fn a_log_line_is_one_json_object_with_every_field_in_order() {
        let line = log_line(&record(false)).expect("serializes");
        assert!(!line.contains('\n'));
        let keys = [
            "\"time\"",
            "\"ply\"",
            "\"played\"",
            "\"source\"",
            "\"stale\"",
            "\"request\"",
            "\"attempts\"",
        ];
        let at: Vec<usize> = keys
            .iter()
            .map(|key| line.find(key).unwrap_or_else(|| panic!("{key} in {line}")))
            .collect();
        assert!(at.is_sorted(), "{line}");
        let value: Value = serde_json::from_str(&line).expect("valid JSON");
        assert_eq!(
            value,
            json!({
                "time": "2026-09-21T14:13:20.123Z",
                "ply": 0,
                "played": "e4",
                "source": "Jev",
                "stale": false,
                "request": {
                    "method": "POST",
                    "url": "https://api.typesafe.ai/v1/systemone",
                    "headers": {
                        "Authorization": "Bearer <redacted>",
                        "Content-Type": "application/json",
                    },
                    "body": jev_exchange().body,
                },
                "attempts": [
                    {
                        "status": 503,
                        "elapsed_ms": 120,
                        "response": "busy \u{1b}[2J now\n\ttry again",
                        "error": "HTTP 503: busy \u{1b}[2J now",
                    },
                    {
                        "status": 200,
                        "elapsed_ms": 850,
                        "response": serde_json::from_str::<Value>(
                            jev_exchange().attempts[1].response.as_deref().unwrap()
                        ).unwrap(),
                        "error": null,
                    },
                ],
            })
        );
        let stale: Value = serde_json::from_str(&log_line(&record(true)).unwrap()).unwrap();
        assert_eq!(stale["stale"], json!(true));
    }

    #[test]
    fn a_failed_attempt_logs_nulls() {
        let mut http = jev_exchange();
        http.attempts = vec![JevAttempt {
            status: None,
            response: None,
            error: Some("request timed out".to_string()),
            elapsed: Duration::from_millis(5000),
        }];
        let game = Game::new();
        let mut computer = jev_move(game.position(), "e2e4");
        computer.source = crate::engine::MoveSource::Fallback;
        let mut history = History::new();
        let record = history.push(Exchange::new(&game, &computer, http), false, at_ms(0));
        let value: Value = serde_json::from_str(&log_line(record).unwrap()).unwrap();
        assert_eq!(value["source"], json!("local search"));
        assert_eq!(
            value["attempts"],
            json!([{
                "status": null,
                "elapsed_ms": 5000,
                "response": null,
                "error": "request timed out",
            }])
        );
    }

    #[test]
    fn the_key_is_nowhere_in_an_exchange_or_its_log_line() {
        // An exchange the engine recorded against a server that echoes the key.
        let game = Game::new();
        let computer = jev_move(game.position(), "e2e4");
        let http = recorded_exchange(SENTINEL_KEY);
        let mut history = History::new();
        let record = history
            .push(Exchange::new(&game, &computer, http), false, at_ms(0))
            .clone();
        let line = log_line(&record).unwrap();
        let text = texts(&record.exchange).join("\n");
        for shown in [line, text, format!("{record:?}")] {
            assert!(!shown.contains(SENTINEL_KEY), "{shown}");
            assert!(shown.contains("Bearer <redacted>"), "{shown}");
        }
    }

    // ----- the log thread -----

    #[test]
    fn a_leading_tilde_in_the_log_path_is_the_home_folder() {
        let dir = TempDir::new("debug-log");
        let home = dir.path().to_str().expect("a UTF-8 temp dir");
        let path = log_path(env(&[(DEBUG_LOG_ENV, "~/x.jsonl"), ("HOME", home)]));
        assert_eq!(path, Ok(dir.join("x.jsonl")));
        let mut log = DebugLog::open(path);
        log.write(&record(false));
        log.close(Duration::from_secs(10));
        assert_eq!(
            fs::read_to_string(dir.join("x.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert!(!dir.join("~").exists());

        // Only a leading `~/` is the home folder.
        let log = |raw: &str| log_path(env(&[(DEBUG_LOG_ENV, raw), ("HOME", "/home/ana")]));
        assert_eq!(log("~/a/b.jsonl"), Ok(PathBuf::from("/home/ana/a/b.jsonl")));
        assert_eq!(log("logs/~/b.jsonl"), Ok(PathBuf::from("logs/~/b.jsonl")));
        assert_eq!(log("~ana/b.jsonl"), Ok(PathBuf::from("~ana/b.jsonl")));
    }

    #[test]
    fn a_tilde_without_a_home_is_reported_as_such() {
        // Without a home there is nothing to expand `~` to, so there is no log, and the
        // reason says so rather than asking for RCHESS_DEBUG_LOG, which is set.
        for pairs in [
            &[(DEBUG_LOG_ENV, "~/x.jsonl")][..],
            &[(DEBUG_LOG_ENV, "~/x.jsonl"), ("HOME", "")],
            &[
                (DEBUG_LOG_ENV, "~/x.jsonl"),
                ("XDG_STATE_HOME", "/xdg/state"),
            ],
        ] {
            let mut log = DebugLog::open(log_path(env(pairs)));
            log.write(&record(false));
            let failure = log.failure().expect("reported");
            assert_eq!(
                failure.reason, "RCHESS_DEBUG_LOG: cannot expand ~ in ~/x.jsonl: HOME is not set",
                "{pairs:?}"
            );
            assert_eq!(failure.path, None);
        }
    }

    #[test]
    fn the_log_is_created_private_and_gets_one_line_per_record() {
        let dir = TempDir::new("debug-log");
        let path = dir.path().join("state").join("rchess").join(LOG_FILE);
        let mut log = DebugLog::start(path.clone());
        assert!(
            !dir.path().join("state").exists(),
            "nothing before a record"
        );
        let (first, second) = (record(false), record(true));
        log.write(&first);
        log.write(&second);
        log.close(Duration::from_secs(10));
        let text = fs::read_to_string(&path).expect("log written");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [log_line(&first).unwrap(), log_line(&second).unwrap()]
        );
        assert!(text.ends_with('\n'));
        #[cfg(unix)]
        {
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
            assert_eq!(mode(&dir.path().join("state")), 0o700);
        }
    }

    #[test]
    fn an_existing_log_is_appended_to() {
        let dir = TempDir::new("debug-log");
        let path = dir.join("jev.jsonl");
        fs::write(&path, "{\"earlier\":true}\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let mut log = DebugLog::start(path.clone());
        log.write(&record(false));
        log.close(Duration::from_secs(10));
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.starts_with("{\"earlier\":true}\n{\"time\""));
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600, "a looser existing mode is tightened");
    }

    #[test]
    #[cfg(unix)]
    fn an_existing_log_is_made_private_and_its_folder_is_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("debug-log");
        // A folder the user already had, such as ~/.local/state, and a log another
        // program left readable by everyone.
        let folder = dir.join("state");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let path = folder.join(LOG_FILE);
        fs::write(&path, "{\"earlier\":true}\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut log = DebugLog::start(path.clone());
        log.write(&record(false));
        log.close(Duration::from_secs(10));
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&folder), 0o755);
    }

    #[test]
    fn a_write_error_is_reported_once_and_ends_the_log() {
        let dir = TempDir::new("debug-log");
        // A folder where the file should be: opening it fails.
        let path = dir.path().to_path_buf();
        let mut log = DebugLog::start(path.clone());
        assert_eq!(log.failure(), None, "no record, no error");
        log.write(&record(false));
        assert_eq!(
            wait_for_failure(&mut log),
            LogFailure {
                reason: "it is a folder".to_string(),
                path: Some(path),
            }
        );
        log.write(&record(false));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(log.failure(), None, "reported once");
        log.close(Duration::from_secs(10));
    }

    #[test]
    fn a_missing_path_is_reported_at_the_first_record() {
        let mut log = DebugLog::open(Err(NO_LOG_PATH.to_string()));
        assert_eq!(log.failure(), None);
        log.write(&record(false));
        let failure = log.failure().expect("reported");
        assert_eq!(failure.path, None);
        assert!(failure.reason.contains(DEBUG_LOG_ENV), "{}", failure.reason);
        log.write(&record(false));
        assert_eq!(log.failure(), None);
    }

    #[test]
    fn a_session_keeps_every_exchange_even_after_the_log_fails() {
        let dir = TempDir::new("debug-log");
        let mut session = DebugSession::new(DebugLog::start(dir.path().to_path_buf()));
        assert_eq!(session.record(exchange(), false, at_ms(1)), 1);
        let deadline = Instant::now() + Duration::from_secs(10);
        while session.log_failure().is_none() {
            assert!(Instant::now() < deadline, "no failure reported");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(session.record(exchange(), true, at_ms(2)), 2);
        assert_eq!(session.log_failure(), None);
        assert_eq!(session.history().len(), 2);
        assert!(session.history().last().is_some_and(|r| r.stale));
        session.close_log(Duration::from_secs(10));
        assert_eq!(session.record(exchange(), false, at_ms(3)), 3);
    }

    #[test]
    fn a_closed_session_log_writes_nothing_more() {
        let dir = TempDir::new("debug-log");
        let path = dir.join("jev.jsonl");
        let mut session = DebugSession::new(DebugLog::start(path.clone()));
        session.record(exchange(), false, at_ms(1));
        session.close_log(Duration::from_secs(10));
        session.record(exchange(), false, at_ms(2));
        assert_eq!(session.log_failure(), None);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
    }
}
