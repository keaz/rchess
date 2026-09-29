//! Debug mode (spec 9.4): every Jev request and its answers, kept for the exchange
//! view and appended to a log file.
//!
//! Debug mode is on with `--debug` or `RCHESS_DEBUG` ([`enabled`]). The engine then
//! records each HTTP exchange with Jev (`EngineConfig::trace`), and the worker turns
//! it into an [`Exchange`] on the engine thread. The app keeps the last
//! [`HISTORY_LEN`] of them in a [`History`], replies it discarded as stale and replies
//! held while Jev vs Jev is paused included, and `d` shows them full screen
//! ([`ExchangeView`]). Only the exchange on screen has its body text rendered
//! ([`BodyCache`]).
//!
//! Each exchange is also appended to the debug log ([`log_path`]) as one JSON object
//! per line ([`log_line`]). A thread named `debug-log` does the writing, fed by a
//! channel of at most [`LOG_QUEUE`] records ([`DebugLog`]), so the UI never waits on
//! the disk. The log file is created with the first exchange, in a folder made private
//! to the user; a log path that is a link is refused; the first error stops the log for
//! the session and is reported once.
//!
//! The API key never reaches any of this: the engine redacts it while recording, so
//! the `Authorization` header reads `Bearer <redacted>`.

use std::collections::VecDeque;
use std::fs::{self, DirBuilder, File};
use std::io::{self, Read as _, Seek as _, SeekFrom, Write as _};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
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
/// The [`LogFailure::reason`] for a log path that is a symbolic link or has more than one
/// hard link: the message reads `debug log disabled: <path> is a link`.
pub const LINK: &str = "is a link";
/// Records waiting for the `debug-log` thread at most; more are dropped ([`LOG_BEHIND`]).
pub const LOG_QUEUE: usize = 64;
/// The warning shown the first time a record is dropped because the queue is full.
pub const LOG_BEHIND: &str = "debug log cannot keep up; some exchanges were not logged";

/// True when debug mode is on: `--debug` was given (`flag`), or `RCHESS_DEBUG` is set
/// to anything but empty or `0` (surrounding whitespace ignored).
///
/// `get` reads an environment variable; pass `|k| std::env::var(k).ok()`.
pub fn enabled(flag: bool, get: impl Fn(&str) -> Option<String>) -> bool {
    flag || get(DEBUG_ENV).is_some_and(|value| !matches!(value.trim(), "" | "0"))
}

/// The debug log file: `RCHESS_DEBUG_LOG` when set, else
/// `$XDG_STATE_HOME/rchess/jev-debug.jsonl`, else `~/.local/state/rchess/jev-debug.jsonl`
/// (on macOS too). Empty variables count as unset, and so do a relative
/// `XDG_STATE_HOME` (the XDG base directory rules say to ignore one) and a relative
/// `HOME`. A leading `~/` in `RCHESS_DEBUG_LOG` is `HOME`, as in save paths
/// ([`expand_tilde`]); a relative `RCHESS_DEBUG_LOG` stays relative to the working
/// directory.
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
    let absolute = |name: &str| set(name).map(PathBuf::from).filter(|dir| dir.is_absolute());
    let home = absolute("HOME");
    if let Some(path) = set(DEBUG_LOG_ENV) {
        return expand_tilde(&path, home.as_deref())
            .map_err(|error| format!("{DEBUG_LOG_ENV}: {error}"));
    }
    let state = absolute("XDG_STATE_HOME")
        .or_else(|| home.map(|home| home.join(".local").join("state")))
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
    /// The model that was asked: `Jev` or `Laya`.
    pub engine: String,
    /// How the move was chosen: the `MoveSource` label (`Jev`, `Laya`,
    /// `vetoed (Jev picked Qh4)` or `local search`).
    pub source: String,
    /// Time for the whole engine call.
    pub latency: Duration,
    /// The request and its attempts.
    pub http: JevExchange,
}

impl Exchange {
    /// The exchange `http` that chose `computer` in `game` (the game the engine was
    /// asked about).
    pub fn new(game: &Game, computer: &ComputerMove, http: JevExchange) -> Exchange {
        Exchange {
            ply: game.moves().len(),
            fullmove: game.position().fullmove_number(),
            san: computer.san.clone(),
            engine: computer.provider.name().to_string(),
            source: computer.source.label(computer.provider),
            latency: computer.latency,
            http,
        }
    }

    /// The view's text, rendered anew on each call (the view keeps it for the exchange
    /// on screen only, [`BodyCache`]): `REQUEST` (method and URL, the headers, then the
    /// JSON body pretty-printed), then one `RESPONSE` block per attempt (its status or
    /// `no response`, the time it took, the error, and the body: pretty-printed when it
    /// is JSON, else the text as sent). Control characters are escaped (`\u{1b}`), so
    /// nothing a server sends can act on the terminal.
    pub fn body(&self) -> Vec<BodyLine> {
        body_lines(&self.http)
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
}

impl BodyLine {
    /// `text` (which must hold no control characters) drawn as `kind`.
    fn new(text: String, kind: LineKind) -> BodyLine {
        BodyLine { text, kind }
    }

    /// The line cut into rows of at most `width` cells (at least one), between any two
    /// characters: a wide character that would straddle the edge starts the next row, and
    /// a zero-width one stays with the character before it.
    pub fn wrapped(&self, width: usize) -> Vec<&str> {
        self.row_ranges(width)
            .map(|range| &self.text[range])
            .collect()
    }

    /// Where each row of [`BodyLine::wrapped`] is in `text`.
    fn row_ranges(&self, width: usize) -> impl Iterator<Item = Range<usize>> + '_ {
        let width = width.max(1);
        let mut chars = self.text.char_indices().peekable();
        let mut first = true;
        std::iter::from_fn(move || {
            let start = match chars.peek() {
                Some(&(start, _)) => start,
                // An empty line is one empty row.
                None if first => 0,
                None => return None,
            };
            first = false;
            let mut used = 0;
            let mut end = start;
            while let Some(&(at, c)) = chars.peek() {
                let w = char_width(c);
                if used > 0 && used + w > width {
                    break;
                }
                used += w;
                end = at + c.len_utf8();
                chars.next();
            }
            Some(start..end)
        })
    }
}

/// The exchange view's body for the exchange on screen: its text rendered
/// ([`Exchange::body`]) when that exchange is first drawn, and cut into rows for one
/// width, again only when the width changes. No other exchange keeps a rendered body.
#[derive(Clone, Debug, Default)]
pub struct BodyCache {
    /// The [`Record::number`] of the exchange whose text `lines` holds.
    number: Option<u64>,
    lines: Vec<BodyLine>,
    /// The width `rows` were cut for, and each row: its line and where it is in it.
    width: usize,
    rows: Vec<(usize, Range<usize>)>,
}

impl BodyCache {
    /// The body of `record`'s exchange cut into rows of at most `width` cells,
    /// rendering and cutting only what is not held already.
    pub fn rows(&mut self, record: &Record, width: usize) -> BodyRows<'_> {
        let width = width.max(1);
        if self.number != Some(record.number) {
            self.number = Some(record.number);
            self.lines = record.exchange.body();
            self.rows.clear();
        }
        if self.width != width || self.rows.is_empty() {
            self.width = width;
            self.rows = self
                .lines
                .iter()
                .enumerate()
                .flat_map(|(index, line)| line.row_ranges(width).map(move |range| (index, range)))
                .collect();
        }
        BodyRows {
            lines: &self.lines,
            rows: &self.rows,
        }
    }

    /// The record number and the width of the rows held, if any.
    pub fn holds(&self) -> Option<(u64, usize)> {
        self.number.map(|number| (number, self.width))
    }
}

/// An exchange's body cut into rows ([`BodyCache::rows`]).
#[derive(Clone, Copy, Debug)]
pub struct BodyRows<'a> {
    lines: &'a [BodyLine],
    rows: &'a [(usize, Range<usize>)],
}

impl<'a> BodyRows<'a> {
    /// Rows in the body.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True for a body without rows (never so for an exchange's body).
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row `index`, from 0: how it is drawn and its text.
    pub fn get(&self, index: usize) -> Option<(LineKind, &'a str)> {
        let (line, range) = self.rows.get(index)?;
        let line = &self.lines[*line];
        Some((line.kind, &line.text[range.clone()]))
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
    /// The reply is held while Jev vs Jev is paused: neither played nor discarded yet.
    /// Settled ([`History::settle`]) once it is; the log keeps the mark it had on arrival.
    pub held: bool,
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
        self.push_marked(exchange, stale, false, time)
    }

    /// [`History::push`], marked [`Record::held`] when `held` is true.
    fn push_marked(
        &mut self,
        exchange: Exchange,
        stale: bool,
        held: bool,
        time: SystemTime,
    ) -> &Record {
        if self.records.len() == HISTORY_LEN {
            self.records.pop_front();
        }
        self.recorded += 1;
        self.records.push_back(Record {
            number: self.recorded,
            time,
            stale,
            held,
            exchange: Arc::new(exchange),
        });
        &self.records[self.records.len() - 1]
    }

    /// The held reply numbered `number` has been played (`stale` false) or discarded:
    /// its record is no longer held. Nothing happens once it has been dropped.
    pub fn settle(&mut self, number: u64, stale: bool) {
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.number == number)
        {
            record.held = false;
            record.stale = stale;
        }
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
/// stop where the screen does. Opening the view or switching exchanges keeps the page
/// size and leaves the limit unknown (`usize::MAX`) until the next draw, so keys in the
/// same input batch still scroll; the draw then clamps the scroll to the body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExchangeView {
    /// The [`Record::number`] shown; `None` while there are no exchanges.
    pub shown: Option<u64>,
    /// Body rows scrolled off the top.
    pub scroll: usize,
    /// Body rows on screen at the last draw.
    pub page: usize,
    /// The largest `scroll` at the last draw: the body's last row at the bottom;
    /// `usize::MAX` before the exchange shown has been drawn.
    pub max_scroll: usize,
}

impl ExchangeView {
    /// The view on the newest exchange in `history`, scrolled to the top, with the
    /// `page` size the view last had (0 before it was ever drawn).
    #[must_use]
    pub fn open(history: &History, page: usize) -> ExchangeView {
        ExchangeView {
            shown: history.last().map(|record| record.number),
            scroll: 0,
            page,
            max_scroll: usize::MAX,
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

    /// Shows exchange `number` from its top, keeping the page size.
    fn show(&mut self, number: Option<u64>) {
        if number != self.shown {
            *self = ExchangeView {
                shown: number,
                scroll: 0,
                page: self.page,
                max_scroll: usize::MAX,
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
    engine: &'a str,
    source: &'a str,
    stale: bool,
    held: bool,
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
/// `ply`, `played` (SAN), `source`, `stale`, `held` (it arrived while Jev vs Jev was
/// paused; a held record is logged on arrival, so its `stale` is its state then, and
/// whether it was played or discarded later is only in the [`History`]), `request`
/// (`method`, `url`, `headers` as an object, `body`) and `attempts`
/// (each `status` or null, `elapsed_ms`, `response` as JSON when it parses, else as a
/// string, or null, and `error` or null).
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
        engine: &exchange.engine,
        source: &exchange.source,
        stale: record.stale,
        held: record.held,
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
    /// A short reason, such as `permission denied`, or [`LINK`].
    pub reason: String,
    /// The log file, when there is one.
    pub path: Option<PathBuf>,
}

/// The debug log: records go over a channel to the `debug-log` thread, which appends
/// each as a line ([`log_line`]) and flushes it. The file and any missing folders are
/// created with the first record: folders with mode 0700, the file with mode 0600 (an
/// existing file is appended to and made 0600 too; an existing folder keeps its mode).
/// A log path that is a symbolic link or has more than one hard link is refused
/// ([`LINK`]), and an existing log whose last record was cut short gets a line break
/// first.
///
/// The first error ends the thread; [`DebugLog::failure`] then reports it once and
/// nothing more is written. Records never wait: [`DebugLog::write`] only sends, and
/// drops the record when [`LOG_QUEUE`] are already waiting ([`DebugLog::dropped`]).
pub struct DebugLog {
    state: LogState,
    /// A record was dropped: `Some(true)` until [`DebugLog::dropped`] reports it.
    dropped: Option<bool>,
}

enum LogState {
    /// The thread is writing.
    Running {
        records: SyncSender<Record>,
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
                dropped: None,
            },
        }
    }

    /// Starts the `debug-log` thread appending to `path`. Nothing touches the disk
    /// until the first record.
    #[must_use]
    pub fn start(path: PathBuf) -> DebugLog {
        let (records, queued) = mpsc::sync_channel(LOG_QUEUE);
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
        DebugLog {
            state,
            dropped: None,
        }
    }

    /// Queues `record` for the log; never blocks. With [`LOG_QUEUE`] records already
    /// waiting, `record` is dropped.
    pub fn write(&mut self, record: &Record) {
        match &mut self.state {
            LogState::Running { records, .. } => match records.try_send(record.clone()) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.dropped.get_or_insert(true);
                }
                // The thread has stopped, and `failure` says why.
                Err(TrySendError::Disconnected(_)) => {}
            },
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

    /// True the first time this is asked after a record was dropped because the queue
    /// was full; false before and ever after, so the warning is shown once.
    pub fn dropped(&mut self) -> bool {
        match &mut self.dropped {
            Some(unreported) => std::mem::replace(unreported, false),
            None => false,
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
///
/// A symbolic link at `path`, or a file with another hard link, is refused with the
/// reason [`LINK`]: checked before the file is opened, not followed by the open
/// ([`open_appending`]), so a symbolic link planted in between is refused without
/// touching its target, and checked again after (a hard link or another file planted in
/// between is still refused, before anything is written or its mode changed). A regular
/// file whose last byte is not a line break (a session that ended mid-record) gets one.
fn open_log(path: &Path) -> io::Result<File> {
    let refused = || io::Error::other(LINK);
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink()) {
        return Err(refused());
    }
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
    let file = open_appending(path)?;
    let metadata = file.metadata()?;
    if is_link(path, &metadata)? {
        return Err(refused());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.is_file() && metadata.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
    }
    if metadata.is_file() && metadata.len() > 0 && !ends_with_newline(path)? {
        (&file).write_all(b"\n")?;
    }
    Ok(file)
}

/// Opens `path` for appending, creating it with mode 0600 when it is not there, and
/// without following a symbolic link at `path` (`O_NOFOLLOW`): one is refused with the
/// reason [`LINK`], and its target is neither opened nor created.
#[cfg(unix)]
fn open_appending(path: &Path) -> io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    let flags =
        OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match rustix::fs::open(path, flags, Mode::RUSR | Mode::WUSR) {
        Ok(fd) => Ok(File::from(fd)),
        // What `O_NOFOLLOW` gives for a symbolic link.
        Err(rustix::io::Errno::LOOP) => Err(io::Error::other(LINK)),
        Err(errno) => Err(errno.into()),
    }
}

/// Opens `path` for appending, creating it when it is not there.
#[cfg(not(unix))]
fn open_appending(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new().append(true).create(true).open(path)
}

/// True when the file opened at `path` (its `opened` metadata) is not simply the file
/// named `path`: `path` is now a symbolic link, or names another file, or the file has
/// more than one hard link.
#[cfg(unix)]
fn is_link(path: &Path, opened: &fs::Metadata) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let named = fs::symlink_metadata(path)?;
    Ok(named.is_symlink()
        || (named.dev(), named.ino()) != (opened.dev(), opened.ino())
        || (opened.is_file() && opened.nlink() > 1))
}

/// True when `path` is now a symbolic link (other systems: hard links are not checked).
#[cfg(not(unix))]
fn is_link(path: &Path, _opened: &fs::Metadata) -> io::Result<bool> {
    Ok(fs::symlink_metadata(path)?.is_symlink())
}

/// True when the last byte of the (non-empty) file at `path` is a line break.
fn ends_with_newline(path: &Path) -> io::Result<bool> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    Ok(last[0] == b'\n')
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

    /// Keeps `exchange`, received at `time` while Jev vs Jev is paused, marked held, and
    /// queues it for the log at once, so quitting while paused never loses it. Returns its
    /// [`Record::number`], for [`DebugSession::settle`].
    pub fn hold(&mut self, exchange: Exchange, time: SystemTime) -> u64 {
        let record = self.history.push_marked(exchange, false, true, time);
        self.log.write(record);
        record.number
    }

    /// The held reply numbered `number` was played (`stale` false) or discarded; the
    /// log is not written again ([`History::settle`]).
    pub fn settle(&mut self, number: u64, stale: bool) {
        self.history.settle(number, stale);
    }

    /// See [`DebugLog::failure`].
    pub fn log_failure(&mut self) -> Option<LogFailure> {
        self.log.failure()
    }

    /// See [`DebugLog::dropped`].
    pub fn log_dropped(&mut self) -> bool {
        self.log.dropped()
    }

    /// Closes the log ([`DebugLog::close`]); later exchanges are kept but not logged.
    pub fn close_log(&mut self, grace: Duration) {
        let stopped = DebugLog {
            state: LogState::Stopped(None),
            dropped: None,
        };
        std::mem::replace(&mut self.log, stopped).close(grace);
    }
}

// ----- test support -----

#[cfg(test)]
impl DebugLog {
    /// A log whose `debug-log` thread never takes a record, as when it is stuck on a slow
    /// disk: the test holds the queue's other end (`queued`), so the queue fills after
    /// [`LOG_QUEUE`] records, with no thread to leave behind. While the test also keeps
    /// `failures`, the log does not take its thread for stopped.
    pub(crate) fn stalled(path: PathBuf) -> (DebugLog, Receiver<Record>, mpsc::Sender<LogFailure>) {
        let (records, queued) = mpsc::sync_channel(LOG_QUEUE);
        let (failed, failures) = mpsc::channel();
        let (_done, finished) = mpsc::channel::<()>();
        let state = LogState::Running {
            records,
            failures,
            finished,
            path,
        };
        let log = DebugLog {
            state,
            dropped: None,
        };
        (log, queued, failed)
    }

    /// Waits up to 10 s for the `debug-log` thread to end, as it does at its first error
    /// (after handing the error over), so that [`DebugLog::failure`] can report it at once.
    /// True when there is no thread running any more.
    pub(crate) fn wait_for_thread(&self) -> bool {
        match &self.state {
            LogState::Running { finished, .. } => matches!(
                finished.recv_timeout(Duration::from_secs(10)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ),
            LogState::Unavailable(_) | LogState::Stopped(_) => true,
        }
    }
}

#[cfg(test)]
impl DebugSession {
    /// See [`DebugLog::wait_for_thread`].
    pub(crate) fn wait_for_log_thread(&self) -> bool {
        self.log.wait_for_thread()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

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

    fn texts(exchange: &Exchange) -> Vec<String> {
        exchange
            .body()
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    /// The failure `log` reports once its thread has ended (waiting up to 10 s for that).
    fn wait_for_failure(log: &mut DebugLog) -> LogFailure {
        assert!(log.wait_for_thread(), "the debug-log thread still runs");
        log.failure().expect("the thread ended with a failure")
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
        for pairs in [
            &[][..],
            &[("HOME", ""), ("XDG_STATE_HOME", "state")],
            &[("HOME", "home/ana")],
            &[("HOME", "home/ana"), ("XDG_STATE_HOME", "state")],
        ] {
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
        assert_eq!(lines.last().map(String::as_str), Some("}"));
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
        assert_eq!(line.wrapped(4), ["abcd", "efgh", "ij"]);
        assert_eq!(line.wrapped(10), ["abcdefghij"]);
        assert_eq!(line.wrapped(0).len(), 10, "at least one cell per row");
        let empty = BodyLine::new(String::new(), LineKind::Text);
        assert_eq!(empty.wrapped(8), [""]);
        // A wide character never straddles the edge; a combining mark stays with its base.
        let wide = BodyLine::new("ab日本cd".to_string(), LineKind::Text);
        assert_eq!(wide.wrapped(3), ["ab", "日", "本c", "d"]);
        let marked = BodyLine::new("abe\u{301}f".to_string(), LineKind::Text);
        assert_eq!(marked.wrapped(3), ["abe\u{301}", "f"]);
    }

    /// The rows `cache` holds for `record` at `width`, as text.
    fn cached_rows(cache: &mut BodyCache, record: &Record, width: usize) -> Vec<String> {
        let rows = cache.rows(record, width);
        (0..rows.len())
            .map(|index| rows.get(index).expect("a row").1.to_string())
            .collect()
    }

    #[test]
    fn the_body_is_rendered_for_the_exchange_on_screen_only() {
        let mut history = History::new();
        history.push(exchange(), false, at_ms(1));
        let mut other = jev_exchange();
        other.attempts.truncate(1);
        let game = Game::new();
        let second = Exchange::new(&game, &jev_move(game.position(), "d2d4"), other);
        history.push(second, false, at_ms(2));
        let (first, second) = (history.get(0).unwrap(), history.get(1).unwrap());

        let mut cache = BodyCache::default();
        assert_eq!(cache.holds(), None);
        let wide = cached_rows(&mut cache, first, 40);
        assert_eq!(cache.holds(), Some((1, 40)));
        let expected: Vec<String> = first
            .exchange
            .body()
            .iter()
            .flat_map(|line| {
                line.wrapped(40)
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(wide, expected);
        let rows = cache.rows(first, 40);
        assert_eq!(rows.get(0), Some((LineKind::Heading, "REQUEST")));
        assert_eq!(rows.get(rows.len()), None);

        // Another width wraps the same text again.
        let narrow = cached_rows(&mut cache, first, 12);
        assert_eq!(cache.holds(), Some((1, 12)));
        assert!(narrow.len() > wide.len());
        assert!(narrow.iter().all(|row| row.chars().count() <= 12));
        assert_eq!(narrow.concat(), wide.concat());

        // Another exchange replaces the one held.
        let rows = cached_rows(&mut cache, second, 12);
        assert_eq!(cache.holds(), Some((2, 12)));
        assert!(rows.len() < narrow.len(), "one response block, not two");
        assert!(!rows.concat().contains("RESPONSE 2"));
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
        let mut view = ExchangeView::open(&history, 0);
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

        let view = ExchangeView::open(&history, 0);
        assert_eq!(view.shown, Some(3), "opens on the newest");
    }

    #[test]
    fn a_dropped_exchange_leaves_the_view_on_the_oldest() {
        let mut history = History::new();
        history.push(exchange(), false, at_ms(1));
        history.push(exchange(), false, at_ms(2));
        let mut view = ExchangeView::open(&history, 0);
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
        let mut view = ExchangeView::open(&history, 0);
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
                scroll: 0,
                page: 1,
                max_scroll: usize::MAX,
            },
            "at the top of the other exchange, with the page size kept"
        );
        view.older(&history);
        view.max_scroll = 25;
        view.end();
        view.older(&history);
        assert_eq!(view.scroll, 25, "already the oldest: nothing moves");
    }

    #[test]
    fn the_page_size_is_kept_when_opening_or_switching() {
        let mut history = History::new();
        for n in 1..=2 {
            history.push(exchange(), false, at_ms(n));
        }
        let mut view = ExchangeView::open(&history, 12);
        assert_eq!(
            view,
            ExchangeView {
                shown: Some(2),
                scroll: 0,
                page: 12,
                max_scroll: usize::MAX,
            }
        );
        // Until the next draw finds the body's end, the keys scroll freely and the draw
        // clamps where they went.
        view.page_down();
        assert_eq!(view.scroll, 11);
        view.down(1);
        assert_eq!(view.scroll, 12);
        view.older(&history);
        assert_eq!((view.shown, view.scroll, view.page), (Some(1), 0, 12));
        view.end();
        assert_eq!(
            view.scroll,
            usize::MAX,
            "the end, wherever the draw finds it"
        );
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
            "\"engine\"",
            "\"source\"",
            "\"stale\"",
            "\"held\"",
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
                "engine": "Jev",
                "source": "Jev",
                "stale": false,
                "held": false,
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
    fn a_relative_home_is_not_used_for_the_log() {
        // An absolute HOME gives the log a place; a relative one counts as unset.
        assert_eq!(
            log_path(env(&[("HOME", "/home/ana")])),
            Ok(PathBuf::from(
                "/home/ana/.local/state/rchess/jev-debug.jsonl"
            ))
        );
        assert_eq!(
            log_path(env(&[("HOME", "home/ana")])),
            Err(NO_LOG_PATH.to_string())
        );
        assert_eq!(
            log_path(env(&[("HOME", "home/ana"), ("XDG_STATE_HOME", "/xdg")])),
            Ok(PathBuf::from("/xdg/rchess/jev-debug.jsonl"))
        );
        assert_eq!(
            log_path(env(&[(DEBUG_LOG_ENV, "~/x.jsonl"), ("HOME", "home/ana")])),
            Err("RCHESS_DEBUG_LOG: cannot expand ~ in ~/x.jsonl: HOME is not set".to_string())
        );
        assert_eq!(
            log_path(env(&[(DEBUG_LOG_ENV, "x.jsonl"), ("HOME", "home/ana")])),
            Ok(PathBuf::from("x.jsonl")),
            "no ~, no home needed"
        );
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
    #[cfg(unix)]
    fn a_log_path_that_is_a_link_is_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = TempDir::new("debug-log");
        let target = dir.join("target.jsonl");
        fs::write(&target, "mine\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        let soft = dir.join("soft.jsonl");
        symlink(&target, &soft).unwrap();
        let hard = dir.join("hard.jsonl");
        fs::hard_link(&target, &hard).unwrap();
        let dangling = dir.join("dangling.jsonl");
        symlink(dir.join("nowhere.jsonl"), &dangling).unwrap();
        for path in [soft, hard, dangling] {
            let mut log = DebugLog::start(path.clone());
            log.write(&record(false));
            assert_eq!(
                wait_for_failure(&mut log),
                LogFailure {
                    reason: LINK.to_string(),
                    path: Some(path),
                }
            );
            log.close(Duration::from_secs(10));
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "mine\n");
        assert_eq!(mode(&target), 0o644, "the target's mode is left alone");
        assert!(!dir.join("nowhere.jsonl").exists());
    }

    #[test]
    #[cfg(unix)]
    fn the_open_does_not_follow_a_link_planted_after_the_check() {
        // `open_log` looks at the path before it opens it; a link planted after that look
        // reaches the open itself, which must neither follow it nor create its target.
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("debug-log");
        let target = dir.join("target.jsonl");
        fs::write(&target, "mine\n").unwrap();
        let soft = dir.join("soft.jsonl");
        symlink(&target, &soft).unwrap();
        let dangling = dir.join("dangling.jsonl");
        symlink(dir.join("nowhere.jsonl"), &dangling).unwrap();
        for path in [soft, dangling] {
            let error = open_appending(&path).expect_err("a link is not opened");
            assert_eq!(describe(&error), LINK, "{path:?}");
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "mine\n");
        assert!(!dir.join("nowhere.jsonl").exists(), "no target is created");

        // A plain file is opened, and created with mode 0600.
        let plain = dir.join("plain.jsonl");
        open_appending(&plain).expect("a plain path opens");
        assert_eq!(mode(&plain), 0o600);
    }

    #[test]
    fn a_record_cut_short_is_ended_before_the_next_one() {
        let dir = TempDir::new("debug-log");
        // A session that ended in the middle of a record.
        let path = dir.join("jev.jsonl");
        let half = "{\"time\":\"2026-09-21T14:13:20.123Z\",\"pl";
        fs::write(&path, half).unwrap();
        let (first, second) = (record(false), record(true));
        let mut log = DebugLog::start(path.clone());
        log.write(&first);
        log.write(&second);
        log.close(Duration::from_secs(10));
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                half.to_string(),
                log_line(&first).unwrap(),
                log_line(&second).unwrap()
            ]
        );
        assert!(text.ends_with('\n'));

        // An empty log and a whole one get no extra line.
        for earlier in ["", "{\"earlier\":true}\n"] {
            let path = dir.join("whole.jsonl");
            fs::write(&path, earlier).unwrap();
            let mut log = DebugLog::start(path.clone());
            log.write(&first);
            log.close(Duration::from_secs(10));
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                format!("{earlier}{}\n", log_line(&first).unwrap())
            );
        }
    }

    #[test]
    fn a_full_queue_drops_records_and_warns_once() {
        // Nothing takes the records, as when the thread is stuck writing.
        let (mut log, queued, _failures) = DebugLog::stalled(PathBuf::from("jev.jsonl"));
        assert!(!log.dropped());
        for _ in 0..LOG_QUEUE + 10 {
            log.write(&record(false));
        }
        assert!(log.dropped(), "the queue holds {LOG_QUEUE} records");
        assert!(!log.dropped(), "one warning");
        log.write(&record(false));
        assert!(!log.dropped(), "still one warning");
        assert_eq!(log.failure(), None, "dropping is not a failure");
        assert_eq!(queued.try_iter().count(), LOG_QUEUE, "the first ones wait");
        // Once the queue has room again, records are taken again.
        log.write(&record(false));
        assert_eq!(queued.try_iter().count(), 1);
        assert!(!log.dropped());
    }

    #[test]
    fn a_held_reply_is_logged_when_it_arrives() {
        let dir = TempDir::new("debug-log");
        let path = dir.join("jev.jsonl");
        let mut session = DebugSession::new(DebugLog::start(path.clone()));
        let first = session.hold(exchange(), at_ms(1));
        assert_eq!(first, 1);
        let record = session.history().last().unwrap();
        assert!(record.held && !record.stale, "{record:?}");
        session.settle(first, false);
        let record = session.history().last().unwrap();
        assert!(!record.held && !record.stale, "played: {record:?}");

        let second = session.hold(exchange(), at_ms(2));
        session.settle(second, true);
        let record = session.history().last().unwrap();
        assert!(!record.held && record.stale, "dropped: {record:?}");
        // A record no longer kept is left alone.
        session.settle(99, true);
        assert_eq!(session.history().len(), 2);

        session.close_log(Duration::from_secs(10));
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{text}");
        for (line, ms) in lines.iter().zip([1, 2]) {
            assert_eq!(line["held"], json!(true), "{line}");
            assert_eq!(line["stale"], json!(false), "{line}");
            assert_eq!(line["time"], json!(rfc3339(at_ms(ms))), "when it arrived");
        }
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
        // The thread has ended: nothing more is written, and nothing more reported.
        log.write(&record(false));
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
        assert!(
            session.wait_for_log_thread(),
            "the debug-log thread still runs"
        );
        assert!(session.log_failure().is_some());
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

    #[test]
    fn the_log_names_the_engine() {
        let game = Game::new();
        let laya = ComputerMove {
            provider: crate::engine::Provider::Laya,
            ..jev_move(game.position(), "e2e4")
        };
        let exchange = Exchange::new(&game, &laya, jev_exchange());
        assert_eq!(exchange.engine, "Laya");
        assert_eq!(exchange.source, "Laya");
        let mut history = History::new();
        let record = history
            .push(exchange, false, at_ms(1_790_000_000_123))
            .clone();
        let value: Value = serde_json::from_str(&log_line(&record).unwrap()).unwrap();
        assert_eq!(value["engine"], json!("Laya"));
    }
}
