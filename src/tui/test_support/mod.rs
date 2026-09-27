//! Test scaffolding shared by the TUI's test modules, compiled only for tests.
//!
//! It is split by what it builds on, so each part compiles as soon as the modules it
//! needs exist:
//!
//! - this module: square, game and raw terminal-event builders, a screen reader for a
//!   `TestBackend` buffer and a self-cleaning temporary folder, built only on
//!   `crate::core`, ratatui and crossterm;
//! - `engine` (from the worker and event step on): a scripted fake engine and `AppEvent`
//!   builders, which need `worker` and `event`;
//! - `harness` (from the app step on): a `TestBackend` harness that drives an `App` the
//!   way the run loop does, which needs `app` and `panels`.
//!
//! Nothing here touches the network or a real terminal: engine requests are answered by
//! the test itself or by a fake engine on a worker thread.

pub(crate) mod engine;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::core::{Game, Square};

/// White to move with a pawn on e7 about to promote.
pub(crate) const PROMOTION_FEN: &str = "8/4P3/8/8/8/8/k7/4K3 w - - 0 1";
/// The PGN `Date` of every `Harness` app.
pub(crate) const TEST_DATE: &str = "2026.09.27";

// ----- squares and games -----

/// The square called `name` (`"e4"`).
pub(crate) fn sq(name: &str) -> Square {
    name.parse().expect("square name")
}

/// The game after `moves` (UCI) from `fen`.
pub(crate) fn game_from(fen: &str, moves: &[&str]) -> Game {
    let mut game = Game::from_fen(fen).expect("valid FEN");
    for uci in moves {
        let mv = game.position().parse_uci(uci).expect("legal move");
        game.play(mv).expect("legal move");
    }
    game
}

/// `game`'s moves in UCI.
pub(crate) fn uci_moves(game: &Game) -> Vec<String> {
    game.moves().iter().map(|m| m.to_uci()).collect()
}

// ----- terminal events and screens -----

/// A key press without modifiers, as the terminal reports it.
pub(crate) fn key_event(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// A key press with `modifiers`, such as Ctrl+S or Alt+F, as the terminal reports it.
pub(crate) fn chord_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

/// A mouse event at cell (`column`, `row`), as the terminal reports it.
pub(crate) fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// A bracketed paste of `text`, as the terminal reports it.
pub(crate) fn paste_event(text: &str) -> Event {
    Event::Paste(text.to_string())
}

/// One key press per character of `text`, as the terminal reports them.
pub(crate) fn char_events(text: &str) -> Vec<Event> {
    text.chars().map(|c| key_event(KeyCode::Char(c))).collect()
}

/// The symbols of `buffer`, one line per row.
pub(crate) fn buffer_text(buffer: &Buffer) -> String {
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

// ----- files -----

/// A fresh, uniquely named folder under the system temp dir, removed on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    /// Creates `rchess-<tag>-<pid>-<nanos>-<n>` in the temp dir.
    pub(crate) fn new(tag: &str) -> TempDir {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "rchess-{tag}-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    /// The folder.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// `name` inside the folder.
    pub(crate) fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// Sorted names of everything in the folder, hidden files included.
    pub(crate) fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("read temp dir")
            .map(|entry| {
                entry
                    .expect("temp dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
