//! Saving games to disk.
//!
//! Everything the save dialogs need that is not UI: turning typed text into a path (`~`
//! expansion, default extension), writing atomically with an overwrite check, today's date in
//! PGN form, and a PGN exporter that fills in real tags and wraps the movetext for the PGN
//! export format.

use std::ffi::{OsStr, OsString};
use std::fmt::{self, Write as _};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, ErrorKind, Write as _};
use std::path::{Path, PathBuf, is_separator};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::Game;

/// Longest movetext line: export format needs fewer than 80 characters (PGN standard 8.2.1).
const PGN_MAX_LINE: usize = 79;

/// The PGN `Date` value for an unknown date.
const UNKNOWN_DATE: &str = "????.??.??";

const SECS_PER_DAY: u64 = 86_400;

/// Turns the text typed into a save dialog into the path to write.
///
/// Surrounding whitespace is trimmed. A leading `~` followed by a path separator is replaced
/// with `home`; `~name` is left alone (it is an ordinary relative file name, not another user's
/// home). `.{ext}` is appended unless the file name already ends in it, in any case (`game`
/// becomes `game.pgn`, `game.` becomes `game.pgn`, `2026.09.27` becomes `2026.09.27.pgn`,
/// `game.PGN` stays): a dot in a name is common (dates, versions) and is not taken as a
/// different extension. `ext` is given with or without its dot. Relative paths stay
/// relative to the working directory.
///
/// # Errors
///
/// A message for the status line when the text is empty, names a folder instead of a file
/// (`~`, `~/`, `games/`, `.`, `..`), or starts with `~/` while `home` is `None` or empty.
pub fn resolve_path(raw: &str, ext: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("type a file name".to_string());
    }
    let name = raw.rsplit(is_separator).next().unwrap_or(raw);
    if raw == "~" || matches!(name, "" | "." | "..") {
        return Err(format!("{raw} is a folder; add a file name"));
    }

    let mut path = expand_tilde(raw, home)?;

    let ext = ext.trim_start_matches('.');
    let has_ext = Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|typed| typed.eq_ignore_ascii_case(ext));
    if !ext.is_empty() && !has_ext {
        let path = path.as_mut_os_string();
        if !name.ends_with('.') {
            path.push(".");
        }
        path.push(ext);
    }
    Ok(path)
}

/// `raw` with a leading `~` followed by a path separator replaced by `home`, as
/// [`resolve_path`] does; `~name` and a `~` later in the path are left alone. Relative
/// paths stay relative to the working directory.
///
/// # Errors
///
/// A message for the status line when `raw` starts with `~/` while `home` is `None` or
/// empty.
pub fn expand_tilde(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    match raw.strip_prefix('~') {
        Some(rest) if rest.starts_with(is_separator) => {
            let home = home
                .filter(|home| !home.as_os_str().is_empty())
                .ok_or_else(|| format!("cannot expand ~ in {raw}: HOME is not set"))?;
            Ok(home.join(rest.trim_start_matches(is_separator)))
        }
        _ => Ok(PathBuf::from(raw)),
    }
}

/// Why [`write_file`] did not write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    /// The target exists and `overwrite` was false: ask the user, then retry with `overwrite`.
    Exists,
    /// The write failed.
    Io {
        /// The file that was not written.
        path: PathBuf,
        /// A short reason for the status line, such as `folder does not exist`.
        reason: String,
    },
}

impl fmt::Display for SaveError {
    /// `cannot save (<reason>): <path>`: the reason first, so a path cut short on screen
    /// never hides it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Exists => f.write_str("file already exists"),
            SaveError::Io { path, reason } => {
                write!(f, "cannot save ({reason}): {}", path.display())
            }
        }
    }
}

impl std::error::Error for SaveError {}

/// Writes `contents` to `path` atomically.
///
/// The data goes to a sibling temp file `.<name>.tmp-<pid>`, which is flushed to disk and then
/// put in place, so readers see either the old file or the complete new one and a failed save
/// never leaves a truncated file or a stray temp file behind. The temp file is created
/// exclusively, so a symlink planted at its name is never followed. Parent folders are not
/// created.
///
/// Without `overwrite`, the temp file is hard-linked into place, which fails if the name
/// exists by then, so a file another program creates while the save runs is never replaced.
/// Only where the file system has no hard links does the save fall back to checking the name
/// once more and renaming. With `overwrite`, the temp file is renamed over the old file, which
/// keeps its permission bits. A symlink at `path` stays: the save writes to the file it
/// points to (following a chain of links, and creating the file a dangling link names).
///
/// # Errors
///
/// [`SaveError::Exists`] when `path` exists and `overwrite` is false (nothing is written), and
/// [`SaveError::Io`] when `path` is a folder (or a link to one) or any file operation fails.
pub fn write_file(path: &Path, contents: &str, overwrite: bool) -> Result<(), SaveError> {
    write_file_with(path, contents, overwrite, |tmp, path| {
        fs::hard_link(tmp, path)
    })
}

/// [`write_file`], with `link` doing what [`fs::hard_link`] does, so a test can stand in for
/// another program or for a file system without hard links.
fn write_file_with(
    path: &Path,
    contents: &str,
    overwrite: bool,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), SaveError> {
    let fail = |reason: String| SaveError::Io {
        path: path.to_path_buf(),
        reason,
    };
    let folder = || fail(describe(&ErrorKind::IsADirectory.into()));
    if path.file_name().is_none() {
        return Err(fail("not a file name".to_string()));
    }
    // Where the data goes (the file a symlink points to), and what is there now.
    let (target, replacing) = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => return Err(folder()),
        Ok(_) if !overwrite => return Err(SaveError::Exists),
        Ok(meta) if meta.is_symlink() => {
            let target = link_target(path).map_err(|err| fail(describe(&err)))?;
            match fs::symlink_metadata(&target) {
                Ok(meta) if meta.is_dir() => return Err(folder()),
                Ok(meta) => (target, Some(meta)),
                Err(err) if err.kind() == ErrorKind::NotFound => (target, None),
                Err(err) => return Err(fail(describe(&err))),
            }
        }
        Ok(meta) => (path.to_path_buf(), Some(meta)),
        Err(err) if err.kind() == ErrorKind::NotFound => (path.to_path_buf(), None),
        Err(err) => return Err(fail(describe(&err))),
    };
    let Some(name) = target.file_name() else {
        return Err(fail("not a file name".to_string()));
    };

    let mut tmp_name = OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = target.with_file_name(tmp_name);

    let placed = write_temp(&tmp, contents, replacing.as_ref()).and_then(|()| {
        if overwrite {
            fs::rename(&tmp, &target)
        } else {
            place_new(&tmp, &target, link)
        }
    });
    if let Err(err) = placed {
        // Best effort: the temp file may never have been created.
        let _ = fs::remove_file(&tmp);
        return Err(if err.kind() == ErrorKind::AlreadyExists {
            SaveError::Exists
        } else {
            fail(describe(&err))
        });
    }
    sync_parent(&target);
    Ok(())
}

/// Puts the finished `tmp` at `path`, which must not exist: `link` makes it a second name
/// for `tmp` (failing with [`ErrorKind::AlreadyExists`] when the name is taken), then `tmp`
/// goes. Where the file system has no hard links, `path` is checked once more and `tmp`
/// renamed to it; a file created between the two is then replaced.
fn place_new(
    tmp: &Path,
    path: &Path,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match link(tmp, path) {
        Ok(()) => {
            // Best effort: the file is saved; a temp file left behind is overwritten by the
            // next save with this process id.
            let _ = fs::remove_file(tmp);
            Ok(())
        }
        // Linux reports a file system without hard links (FAT, some network file systems)
        // as EPERM, which is PermissionDenied.
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::Unsupported | ErrorKind::PermissionDenied
            ) =>
        {
            match fs::symlink_metadata(path) {
                Ok(_) => Err(ErrorKind::AlreadyExists.into()),
                Err(err) if err.kind() == ErrorKind::NotFound => fs::rename(tmp, path),
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    }
}

/// Symlinks followed before [`link_target`] gives up (Linux's own limit).
const MAX_LINKS: usize = 40;

/// The file the symlink at `path` points to, following a chain of links: the first name
/// that is not a symlink, which may not exist yet. A relative link is taken from the
/// folder the link is in.
fn link_target(path: &Path) -> io::Result<PathBuf> {
    let mut path = path.to_path_buf();
    for _ in 0..MAX_LINKS {
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_symlink() => {
                let next = fs::read_link(&path)?;
                path = match path.parent() {
                    Some(folder) => folder.join(next),
                    None => next,
                };
            }
            Ok(_) => return Ok(path),
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(path),
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other("too many levels of symbolic links"))
}

/// Creates `tmp` exclusively and writes `contents` to it, flushed to disk.
fn write_temp(tmp: &Path, contents: &str, replacing: Option<&Metadata>) -> io::Result<()> {
    let create = || OpenOptions::new().write(true).create_new(true).open(tmp);
    let mut file = match create() {
        // Left behind by an earlier run that died mid-save with the same process id.
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(tmp)?;
            create()?
        }
        result => result?,
    };
    file.write_all(contents.as_bytes())?;
    if let Some(meta) = replacing.filter(|meta| meta.is_file()) {
        // Best effort: an overwrite should not change who can read the file.
        let _ = file.set_permissions(meta.permissions());
    }
    file.sync_all()
}

/// Best effort: flushes the folder entry so the rename survives a crash.
fn sync_parent(path: &Path) {
    if cfg!(unix) {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

/// A short, user-facing reason for a failed file operation.
pub fn describe(err: &io::Error) -> String {
    match err.kind() {
        ErrorKind::NotFound => "folder does not exist".to_string(),
        ErrorKind::PermissionDenied => "permission denied".to_string(),
        ErrorKind::IsADirectory => "it is a folder".to_string(),
        ErrorKind::NotADirectory => "part of the path is not a folder".to_string(),
        ErrorKind::ReadOnlyFilesystem => "read-only file system".to_string(),
        ErrorKind::StorageFull => "disk is full".to_string(),
        _ => err.to_string(),
    }
}

/// `path` for a message, with the `home` folder written as `~` (`~/games/one.pgn`). Only
/// whole folder names match: `/home/anabel` is not under `/home/ana`.
#[must_use]
pub fn tilde_path(path: &Path, home: Option<&Path>) -> String {
    let rest = home
        .filter(|home| !home.as_os_str().is_empty())
        .and_then(|home| path.strip_prefix(home).ok());
    match rest {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Converts days since 1970-01-01 into a proleptic Gregorian `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`
/// (<https://howardhinnant.github.io/date_algorithms.html#civil_from_days>). Exact for every
/// date whose year fits in an `i32`; beyond that the year saturates. Never panics.
#[must_use]
pub fn civil_date(unix_days: i64) -> (i32, u32, u32) {
    // Shift the epoch to 0000-03-01 so the leap day is the last day of the (March-based) year.
    let z = unix_days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year of era, [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of March-based year, [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = era * 400 + yoe + i64::from(month <= 2);
    let year = i32::try_from(year).unwrap_or(if year < 0 { i32::MIN } else { i32::MAX });
    // Both are in [1, 31], so the casts are lossless.
    (year, month as u32, day as u32)
}

/// Today's date (UTC) in PGN form, `YYYY.MM.DD` ([`pgn_date_at`] of the system clock).
#[must_use]
pub fn today() -> String {
    pgn_date_at(SystemTime::now())
}

/// The UTC date of `time` in PGN form, `YYYY.MM.DD`, or the unknown date `????.??.??` when
/// the year has no four-digit form. Times before 1970 count back from the epoch.
#[must_use]
pub fn pgn_date_at(time: SystemTime) -> String {
    let days = match time.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs() / SECS_PER_DAY).unwrap_or(i64::MAX),
        Err(before) => {
            // Any part of a day before the epoch belongs to the day before it.
            let before = before.duration();
            let secs = before.as_secs() + u64::from(before.subsec_nanos() > 0);
            i64::try_from(secs.div_ceil(SECS_PER_DAY)).map_or(i64::MIN, |days| -days)
        }
    };
    pgn_date(days)
}

/// The PGN `Date` value for a day, or the unknown date when the year has no 4-digit form.
fn pgn_date(unix_days: i64) -> String {
    match civil_date(unix_days) {
        (year @ 0..=9999, month, day) => format!("{year:04}.{month:02}.{day:02}"),
        _ => UNKNOWN_DATE.to_string(),
    }
}

/// The game as an export-format PGN file.
///
/// Starts from [`Game::to_pgn`], which writes the Seven Tag Roster in its standard order and
/// then any other tags (`SetUp`, `FEN`), and replaces its placeholder `Date`, `White` and
/// `Black` values (blank values become the PGN unknowns `?` and `????.??.??`; `"` and `\`
/// are escaped; control characters become spaces); every other tag is kept as written. The
/// movetext is re-wrapped so no line reaches 80 characters, breaking only between tokens.
#[must_use]
pub fn pgn_export(game: &Game, white: &str, black: &str, date: &str) -> String {
    let pgn = game.to_pgn();
    let (headers, movetext) = pgn.split_once("\n\n").unwrap_or(("", pgn.as_str()));
    let mut out = String::with_capacity(pgn.len() + 64);
    for line in headers.lines().filter(|line| !line.trim().is_empty()) {
        let filled = match tag_name(line) {
            Some(name @ "Date") => Some((name, tag_value(date, UNKNOWN_DATE))),
            Some(name @ "White") => Some((name, tag_value(white, "?"))),
            Some(name @ "Black") => Some((name, tag_value(black, "?"))),
            _ => None,
        };
        match filled {
            Some((name, value)) => {
                let _ = writeln!(out, "[{name} \"{value}\"]");
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push('\n');
    out.push_str(&wrap_tokens(movetext, PGN_MAX_LINE));
    out
}

/// The tag name of a `[Name "value"]` header line.
fn tag_name(line: &str) -> Option<&str> {
    let (name, _) = line.trim().strip_prefix('[')?.split_once(' ')?;
    Some(name)
}

/// A PGN tag value: trimmed, `unknown` when blank, `"` and `\` escaped, controls as spaces.
fn tag_value(value: &str, unknown: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return unknown.to_string();
    }
    let mut escaped = String::with_capacity(value.len() + 2);
    for c in value.chars() {
        match c {
            '"' | '\\' => {
                escaped.push('\\');
                escaped.push(c);
            }
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Re-flows whitespace-separated tokens into lines of at most `max` characters, each ending in
/// a newline. A token longer than `max` gets a line of its own; tokens are never split.
fn wrap_tokens(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / max.max(1) + 1);
    let mut width = 0;
    for token in text.split_whitespace() {
        let len = token.chars().count();
        if width > 0 {
            if width + 1 + len > max {
                out.push('\n');
                width = 0;
            } else {
                out.push(' ');
                width += 1;
            }
        }
        out.push_str(token);
        width += len;
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Position;
    use crate::core::START_FEN;
    use crate::tui::test_support::{TempDir, game_from};
    use std::time::Duration;

    fn resolved(raw: &str, ext: &str) -> PathBuf {
        resolve_path(raw, ext, Some(Path::new("/home/ana"))).unwrap()
    }

    #[test]
    fn resolve_path_expands_leading_tilde() {
        assert_eq!(
            resolved("~/games/one", "pgn"),
            PathBuf::from("/home/ana/games/one.pgn")
        );
        assert_eq!(
            resolved("  ~/one.fen ", "fen"),
            PathBuf::from("/home/ana/one.fen")
        );
        assert_eq!(
            resolved("~//one", "pgn"),
            PathBuf::from("/home/ana/one.pgn")
        );
    }

    #[test]
    fn resolve_path_needs_home_for_tilde() {
        for home in [None, Some(Path::new(""))] {
            let err = resolve_path("~/one", "pgn", home).unwrap_err();
            assert_eq!(err, "cannot expand ~ in ~/one: HOME is not set");
        }
        // Without a tilde, no home is needed.
        assert_eq!(
            resolve_path("one", "pgn", None),
            Ok(PathBuf::from("one.pgn"))
        );
    }

    #[test]
    fn resolve_path_keeps_tilde_names_and_inner_tildes_literal() {
        assert_eq!(resolved("~draft", "pgn"), PathBuf::from("~draft.pgn"));
        assert_eq!(resolved("games/~/x", "pgn"), PathBuf::from("games/~/x.pgn"));
    }

    #[test]
    fn resolve_path_rejects_empty_text_and_folders() {
        for raw in ["", "   ", "\t"] {
            assert_eq!(
                resolve_path(raw, "pgn", None),
                Err("type a file name".to_string())
            );
        }
        for raw in ["~", "~/", "games/", "/", ".", "..", "games/.", "games/.."] {
            let err = resolve_path(raw, "pgn", Some(Path::new("/home/ana"))).unwrap_err();
            assert_eq!(err, format!("{raw} is a folder; add a file name"));
        }
    }

    #[test]
    fn resolve_path_appends_missing_extension_only() {
        assert_eq!(resolved("game", "pgn"), PathBuf::from("game.pgn"));
        assert_eq!(resolved(" game ", "pgn"), PathBuf::from("game.pgn"));
        assert_eq!(resolved("game.", "pgn"), PathBuf::from("game.pgn"));
        assert_eq!(resolved("game.pgn", "pgn"), PathBuf::from("game.pgn"));
        assert_eq!(resolved("game.PGN", "pgn"), PathBuf::from("game.PGN"));
        assert_eq!(resolved("pos.Fen", ".fen"), PathBuf::from("pos.Fen"));
        // A dot in the name is not the extension the save needs.
        assert_eq!(resolved("game.txt", "pgn"), PathBuf::from("game.txt.pgn"));
        assert_eq!(
            resolved("carlsen-2026.09.27", "pgn"),
            PathBuf::from("carlsen-2026.09.27.pgn")
        );
        assert_eq!(resolved("game.pgn", "fen"), PathBuf::from("game.pgn.fen"));
        assert_eq!(resolved(".hidden", "pgn"), PathBuf::from(".hidden.pgn"));
        assert_eq!(resolved(".pgn", "pgn"), PathBuf::from(".pgn.pgn"));
        assert_eq!(resolved("v1.2/game", "pgn"), PathBuf::from("v1.2/game.pgn"));
        assert_eq!(resolved("/tmp/pos", "fen"), PathBuf::from("/tmp/pos.fen"));
        assert_eq!(resolved("/tmp/pos", ".fen"), PathBuf::from("/tmp/pos.fen"));
        assert_eq!(resolved("/tmp/pos", ""), PathBuf::from("/tmp/pos"));
    }

    #[test]
    fn write_file_creates_then_refuses_to_clobber_then_overwrites() {
        let dir = TempDir::new("clobber");
        let target = dir.join("game.pgn");

        assert_eq!(write_file(&target, "first\n", false), Ok(()));
        assert_eq!(fs::read_to_string(&target).unwrap(), "first\n");

        assert_eq!(
            write_file(&target, "second\n", false),
            Err(SaveError::Exists)
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "first\n");

        assert_eq!(write_file(&target, "second\n", true), Ok(()));
        assert_eq!(fs::read_to_string(&target).unwrap(), "second\n");

        assert_eq!(dir.entries(), ["game.pgn"], "no temp file is left behind");
    }

    #[test]
    fn write_file_into_missing_folder_fails_cleanly() {
        let dir = TempDir::new("missing");
        let target = dir.join("nope").join("game.pgn");
        let err = write_file(&target, "x", false).unwrap_err();
        assert_eq!(
            err,
            SaveError::Io {
                path: target.clone(),
                reason: "folder does not exist".to_string(),
            }
        );
        assert_eq!(
            err.to_string(),
            format!("cannot save (folder does not exist): {}", target.display())
        );
        assert!(dir.entries().is_empty());
    }

    #[test]
    fn write_file_onto_a_folder_is_an_io_error_even_with_overwrite() {
        let dir = TempDir::new("folder");
        let target = dir.join("sub");
        fs::create_dir(&target).unwrap();
        for overwrite in [false, true] {
            let err = write_file(&target, "x", overwrite).unwrap_err();
            assert_eq!(
                err,
                SaveError::Io {
                    path: target.clone(),
                    reason: "it is a folder".to_string(),
                }
            );
        }
        assert_eq!(dir.entries(), ["sub"]);
    }

    #[test]
    fn write_file_replaces_a_stale_temp_file() {
        let dir = TempDir::new("stale");
        let stale = dir.join(&format!(".game.pgn.tmp-{}", std::process::id()));
        fs::write(&stale, "junk from a crashed run").unwrap();
        assert_eq!(write_file(&dir.join("game.pgn"), "fresh", false), Ok(()));
        assert_eq!(fs::read_to_string(dir.join("game.pgn")).unwrap(), "fresh");
        assert_eq!(dir.entries(), ["game.pgn"]);
    }

    #[cfg(unix)]
    #[test]
    fn write_file_keeps_permissions_when_overwriting() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("perms");
        let target = dir.join("game.pgn");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(write_file(&target, "new", true), Ok(()));
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_file_created_while_saving_is_never_replaced() {
        let dir = TempDir::new("race");
        let target = dir.join("game.pgn");
        // Another program creates the file after the check, just before the save links
        // its temp file into place.
        let result = write_file_with(&target, "ours\n", false, |tmp, path| {
            fs::write(path, "theirs\n")?;
            fs::hard_link(tmp, path)
        });
        assert_eq!(result, Err(SaveError::Exists));
        assert_eq!(fs::read_to_string(&target).unwrap(), "theirs\n");
        assert_eq!(dir.entries(), ["game.pgn"], "no temp file is left behind");
    }

    #[test]
    fn without_hard_links_a_new_file_is_renamed_into_place() {
        // Unsupported, and PermissionDenied: Linux's EPERM for FAT and the like.
        for kind in [ErrorKind::Unsupported, ErrorKind::PermissionDenied] {
            let dir = TempDir::new("no-links");
            let target = dir.join("game.pgn");
            let no_links = |_: &Path, _: &Path| Err(io::Error::from(kind));
            assert_eq!(
                write_file_with(&target, "one\n", false, no_links),
                Ok(()),
                "{kind:?}"
            );
            assert_eq!(fs::read_to_string(&target).unwrap(), "one\n", "{kind:?}");
            assert_eq!(dir.entries(), ["game.pgn"], "{kind:?}");
            // The check before the rename still refuses an existing file.
            assert_eq!(
                write_file_with(&target, "two\n", false, no_links),
                Err(SaveError::Exists),
                "{kind:?}"
            );
            assert_eq!(fs::read_to_string(&target).unwrap(), "one\n", "{kind:?}");
            assert_eq!(dir.entries(), ["game.pgn"], "{kind:?}");
        }
    }

    #[test]
    fn a_failed_link_is_reported_and_cleaned_up() {
        let dir = TempDir::new("link-fails");
        let target = dir.join("game.pgn");
        let full = |_: &Path, _: &Path| Err(io::Error::from(ErrorKind::StorageFull));
        assert_eq!(
            write_file_with(&target, "x", false, full),
            Err(SaveError::Io {
                path: target.clone(),
                reason: "disk is full".to_string(),
            })
        );
        assert!(dir.entries().is_empty(), "{:?}", dir.entries());
    }

    #[cfg(unix)]
    #[test]
    fn saving_over_a_symlink_writes_to_its_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let dir = TempDir::new("symlink");
        let games = dir.join("games");
        fs::create_dir(&games).unwrap();
        let target = games.join("real.pgn");
        fs::write(&target, "old\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        // A relative link, and a link to that link.
        let link = dir.join("game.pgn");
        symlink("games/real.pgn", &link).unwrap();
        let outer = dir.join("latest.pgn");
        symlink("game.pgn", &outer).unwrap();

        assert_eq!(write_file(&link, "new\n", false), Err(SaveError::Exists));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old\n");

        assert_eq!(write_file(&link, "new\n", true), Ok(()));
        assert!(
            fs::symlink_metadata(&link).unwrap().is_symlink(),
            "still a link"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "new\n");
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640, "the target keeps its mode");

        assert_eq!(write_file(&outer, "newer\n", true), Ok(()));
        assert!(fs::symlink_metadata(&outer).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "newer\n");
        assert_eq!(dir.entries(), ["game.pgn", "games", "latest.pgn"]);
        let in_games: Vec<_> = fs::read_dir(&games).unwrap().collect();
        assert_eq!(in_games.len(), 1, "no temp file is left beside the target");
    }

    #[cfg(unix)]
    #[test]
    fn saving_over_a_dangling_symlink_creates_its_target() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("dangling");
        let link = dir.join("game.pgn");
        symlink(dir.join("made.pgn"), &link).unwrap();
        assert_eq!(write_file(&link, "x\n", false), Err(SaveError::Exists));
        assert_eq!(write_file(&link, "x\n", true), Ok(()));
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(dir.join("made.pgn")).unwrap(), "x\n");
    }

    #[cfg(unix)]
    #[test]
    fn saving_over_a_symlink_to_a_folder_is_an_io_error() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("link-to-folder");
        fs::create_dir(dir.join("sub")).unwrap();
        let link = dir.join("game.pgn");
        symlink("sub", &link).unwrap();
        assert_eq!(
            write_file(&link, "x", true),
            Err(SaveError::Io {
                path: link.clone(),
                reason: "it is a folder".to_string(),
            })
        );
        assert!(fs::read_dir(dir.join("sub")).unwrap().next().is_none());
    }

    #[test]
    fn describe_names_each_common_failure() {
        let described = |kind: ErrorKind| describe(&io::Error::from(kind));
        assert_eq!(described(ErrorKind::NotFound), "folder does not exist");
        assert_eq!(described(ErrorKind::PermissionDenied), "permission denied");
        assert_eq!(described(ErrorKind::IsADirectory), "it is a folder");
        assert_eq!(
            described(ErrorKind::NotADirectory),
            "part of the path is not a folder"
        );
        assert_eq!(
            described(ErrorKind::ReadOnlyFilesystem),
            "read-only file system"
        );
        assert_eq!(described(ErrorKind::StorageFull), "disk is full");
        // Anything else keeps the system's own words.
        let other = io::Error::other("quota exceeded on /home");
        assert_eq!(describe(&other), "quota exceeded on /home");
    }

    #[test]
    fn a_file_in_the_way_of_a_folder_is_reported() {
        let dir = TempDir::new("not-a-folder");
        fs::write(dir.join("notes"), "a file").unwrap();
        let target = dir.join("notes").join("game.pgn");
        assert_eq!(
            write_file(&target, "x", false),
            Err(SaveError::Io {
                path: target.clone(),
                reason: "part of the path is not a folder".to_string(),
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_without_write_permission_is_reported() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new("read-only");
        let folder = dir.join("locked");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();
        // Root may write anyway: then no save can be refused here, and the test says it
        // was skipped rather than passing without checking anything.
        if fs::write(folder.join("probe"), "").is_ok() {
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
            eprintln!(
                "skipped a_folder_without_write_permission_is_reported: a folder without \
                 write permission is still writable here (running as root?)"
            );
            return;
        }
        let target = folder.join("game.pgn");
        let result = write_file(&target, "x", false);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            result,
            Err(SaveError::Io {
                path: target,
                reason: "permission denied".to_string(),
            })
        );
    }

    #[test]
    fn save_error_display() {
        assert_eq!(SaveError::Exists.to_string(), "file already exists");
        let denied = SaveError::Io {
            path: PathBuf::from("games/x.pgn"),
            reason: "permission denied".to_string(),
        };
        // The reason comes first, so a path cut short on screen never hides it.
        assert_eq!(
            denied.to_string(),
            "cannot save (permission denied): games/x.pgn"
        );
    }

    #[test]
    fn tilde_path_writes_the_home_folder_as_a_tilde() {
        let home = Some(Path::new("/home/ana"));
        let tilde = |path: &str, home| tilde_path(Path::new(path), home);
        assert_eq!(tilde("/home/ana/games/x.pgn", home), "~/games/x.pgn");
        assert_eq!(tilde("/home/ana/x.pgn", home), "~/x.pgn");
        assert_eq!(
            tilde("/home/anabel/x.pgn", home),
            "/home/anabel/x.pgn",
            "whole folder names only"
        );
        assert_eq!(tilde("games/x.pgn", home), "games/x.pgn");
        assert_eq!(tilde("/home/ana/x.pgn", None), "/home/ana/x.pgn");
        assert_eq!(
            tilde("/home/ana/x.pgn", Some(Path::new(""))),
            "/home/ana/x.pgn"
        );
    }

    #[test]
    fn civil_date_known_values() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(-1), (1969, 12, 31));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(20_723), (2026, 9, 27));
        assert_eq!(civil_date(-135_080), (1600, 3, 1));
        assert_eq!(civil_date(-719_162), (1, 1, 1));
        assert_eq!(civil_date(-719_163), (0, 12, 31));
    }

    #[test]
    fn civil_date_steps_one_day_at_a_time_across_eras() {
        fn days_in_month(year: i32, month: u32) -> u32 {
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            match month {
                2 if leap => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            }
        }
        // Two full 400-year eras around the epoch, crossing the era boundary at 2000-03-01.
        let mut previous = civil_date(-146_097);
        for days in -146_096..=146_097 {
            let (year, month, day) = previous;
            let expected = if day < days_in_month(year, month) {
                (year, month, day + 1)
            } else if month < 12 {
                (year, month + 1, 1)
            } else {
                (year + 1, 1, 1)
            };
            let current = civil_date(days);
            assert_eq!(current, expected, "day {days}");
            previous = current;
        }
    }

    #[test]
    fn civil_date_saturates_instead_of_panicking() {
        assert_eq!(civil_date(i64::MAX).0, i32::MAX);
        assert_eq!(civil_date(i64::MIN).0, i32::MIN);
    }

    #[test]
    fn pgn_dates() {
        assert_eq!(pgn_date(0), "1970.01.01");
        assert_eq!(pgn_date(20_723), "2026.09.27");
        assert_eq!(pgn_date(-719_162), "0001.01.01");
        assert_eq!(pgn_date(-800_000), UNKNOWN_DATE);
        assert_eq!(pgn_date(3_000_000), UNKNOWN_DATE);
    }

    #[test]
    fn pgn_date_at_counts_whole_days_in_utc() {
        let day = Duration::from_secs(SECS_PER_DAY);
        let at = |since_epoch: Duration| pgn_date_at(UNIX_EPOCH + since_epoch);
        assert_eq!(at(Duration::ZERO), "1970.01.01");
        assert_eq!(at(day - Duration::from_nanos(1)), "1970.01.01");
        assert_eq!(at(day * 20_723), "2026.09.27");
        assert_eq!(at(day * 20_724 - Duration::from_secs(1)), "2026.09.27");
        let before = |until_epoch: Duration| pgn_date_at(UNIX_EPOCH - until_epoch);
        assert_eq!(before(Duration::from_nanos(1)), "1969.12.31");
        assert_eq!(before(Duration::from_secs(1)), "1969.12.31");
        assert_eq!(before(day), "1969.12.31");
        assert_eq!(before(day + Duration::from_secs(1)), "1969.12.30");
        assert_eq!(at(day * 3_000_000), UNKNOWN_DATE);
    }

    #[test]
    fn today_is_a_pgn_date_whatever_the_clock_says() {
        // Only the shape: the clock may be anywhere (a VM with a reset clock, say).
        let today = today();
        let shape = |c: u8| {
            if c == b'.' {
                '.'
            } else if c.is_ascii_digit() || c == b'?' {
                '9'
            } else {
                'x'
            }
        };
        let shape: String = today.bytes().map(shape).collect();
        assert_eq!(shape, "9999.99.99", "{today}");
    }

    #[test]
    fn pgn_export_fills_the_seven_tag_roster() {
        let game = game_from(START_FEN, &["f2f3", "e7e5", "g2g4", "d8h4"]);
        assert_eq!(
            pgn_export(&game, "You", "Jev", "2026.09.27"),
            "[Event \"rchess game\"]\n\
             [Site \"?\"]\n\
             [Date \"2026.09.27\"]\n\
             [Round \"?\"]\n\
             [White \"You\"]\n\
             [Black \"Jev\"]\n\
             [Result \"0-1\"]\n\
             \n\
             1. f3 e5 2. g4 Qh4# 0-1\n"
        );
    }

    #[test]
    fn pgn_export_escapes_and_defaults_tag_values() {
        let pgn = pgn_export(&Game::new(), "Ann \"The Rook\" \\o/", "  ", "");
        assert!(
            pgn.contains("[White \"Ann \\\"The Rook\\\" \\\\o/\"]\n"),
            "{pgn}"
        );
        assert!(pgn.contains("[Black \"?\"]\n"), "{pgn}");
        assert!(pgn.contains("[Date \"????.??.??\"]\n"), "{pgn}");
        assert!(pgn.ends_with("[Result \"*\"]\n\n*\n"), "{pgn}");

        let pgn = pgn_export(&Game::new(), "line\nbreak\ttab", "Jev", "2026.09.27");
        assert!(pgn.contains("[White \"line break tab\"]\n"), "{pgn}");
    }

    #[test]
    fn pgn_export_keeps_setup_tags_after_the_roster() {
        let fen = "4k3/8/8/8/8/8/4P3/4K3 b - - 0 12";
        let game = game_from(fen, &["e8d7", "e2e4"]);
        let pgn = pgn_export(&game, "Local search", "You", "2026.09.27");
        let (headers, movetext) = pgn.split_once("\n\n").unwrap();
        let names: Vec<&str> = headers.lines().filter_map(tag_name).collect();
        assert_eq!(
            names,
            [
                "Event", "Site", "Date", "Round", "White", "Black", "Result", "SetUp", "FEN"
            ]
        );
        assert!(headers.contains(&format!("[FEN \"{fen}\"]")), "{headers}");
        assert_eq!(movetext, "12... Kd7 13. e4 *\n");
    }

    #[test]
    fn pgn_export_keeps_the_tags_to_pgn_writes_and_only_fills_in_three() {
        let game = game_from(START_FEN, &["e2e4"]);
        let exported = pgn_export(&game, "You", "Jev", "2026.09.27");
        let original = game.to_pgn();
        let lines = |pgn: &str| -> Vec<String> {
            let (headers, _) = pgn.split_once("\n\n").unwrap();
            headers.lines().map(str::to_string).collect()
        };
        let (exported, original) = (lines(&exported), lines(&original));
        assert_eq!(exported.len(), original.len());
        for (new, old) in exported.iter().zip(&original) {
            match tag_name(old) {
                Some("Date") => assert_eq!(new, "[Date \"2026.09.27\"]"),
                Some("White") => assert_eq!(new, "[White \"You\"]"),
                Some("Black") => assert_eq!(new, "[Black \"Jev\"]"),
                _ => assert_eq!(new, old),
            }
        }
    }

    /// A deterministic pseudo-random game of `plies` legal moves that is still in progress.
    fn long_game(plies: usize) -> Game {
        let mut game = Game::new();
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        while game.moves().len() < plies {
            let moves = game.position().legal_moves();
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let start = (seed >> 33) as usize % moves.len();
            let mv = (0..moves.len())
                .map(|i| moves[(start + i) % moves.len()])
                .find(|&mv| {
                    let mut next = game.clone();
                    next.play(mv).is_ok() && next.outcome().is_none()
                })
                .expect("some move keeps the game going");
            game.play(mv).unwrap();
        }
        game
    }

    #[test]
    fn pgn_export_wraps_long_movetext_between_tokens() {
        let game = long_game(130);
        let pgn = pgn_export(&game, "You", "Jev", "2026.09.27");

        for line in pgn.lines() {
            assert!(line.chars().count() < 80, "line too long: {line:?}");
            assert_eq!(line, line.trim(), "no padding: {line:?}");
        }

        let (_, movetext) = pgn.split_once("\n\n").unwrap();
        let lines: Vec<&str> = movetext.lines().collect();
        assert!(lines.len() > 5, "{movetext}");
        // Greedy fill: the next line's first token would not have fitted on this line.
        for pair in lines.windows(2) {
            let next_token = pair[1].split(' ').next().unwrap();
            assert!(
                pair[0].len() + 1 + next_token.len() > PGN_MAX_LINE,
                "{pair:?}"
            );
        }

        // Only whitespace changed: the tokens are exactly those of `to_pgn`.
        let original = game.to_pgn();
        let (_, original_movetext) = original.split_once("\n\n").unwrap();
        assert!(
            original_movetext.trim_end().len() > 80,
            "to_pgn is one long line"
        );
        assert!(
            movetext
                .split_whitespace()
                .eq(original_movetext.split_whitespace())
        );

        // And the wrapped movetext replays to the same position.
        let mut replay = Game::new();
        let tokens: Vec<&str> = movetext.split_whitespace().collect();
        let (result, sans) = tokens.split_last().unwrap();
        assert_eq!(*result, "*");
        for san in sans.iter().filter(|token| !token.ends_with('.')) {
            let mv = replay.position().parse_san(san).unwrap();
            replay.play(mv).unwrap();
        }
        assert_eq!(replay.moves(), game.moves());
        assert_eq!(replay.position(), game.position());
        assert_ne!(*game.position(), Position::startpos());
    }

    #[test]
    fn wrap_tokens_edge_cases() {
        assert_eq!(wrap_tokens("", 10), "\n");
        assert_eq!(wrap_tokens("  a   b  ", 10), "a b\n");
        assert_eq!(wrap_tokens("aaa bbb ccc", 7), "aaa bbb\nccc\n");
        assert_eq!(wrap_tokens("a verylongtoken b", 5), "a\nverylongtoken\nb\n");
    }
}
