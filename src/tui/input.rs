//! The command box: a hand-rolled single-line editor and the command parser.
//!
//! [`LineEditor`] holds the text being typed and a cursor. It never stores control
//! characters, so what is drawn is exactly what is parsed. [`parse_command`] turns a
//! submitted line into a [`Command`]: lines starting with `:` are commands, anything else is
//! move text for [`super::movetext::parse_move`].

use super::glyphs::shorten;

/// Single-line text editor with a cursor, for the command box and dialog inputs.
///
/// The cursor is a position between characters (Unicode scalar values), so non-ASCII input
/// is safe everywhere. Control characters and invisible format characters (zero-width
/// spaces and joiners, bidirectional overrides, byte-order marks, line separators, soft
/// hyphens) are never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    /// Byte offset into `text`; always on a char boundary.
    cursor: usize,
}

impl LineEditor {
    /// An empty editor.
    pub fn new() -> LineEditor {
        LineEditor::default()
    }

    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the text is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The cursor position as a character index: 0 is before the first character,
    /// `text().chars().count()` is after the last.
    pub fn cursor(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    /// The text before the cursor. Its display width is the cursor's column offset, which
    /// differs from [`cursor`](Self::cursor) when the text contains wide characters.
    pub fn before_cursor(&self) -> &str {
        &self.text[..self.cursor]
    }

    /// Inserts `c` at the cursor and moves the cursor past it. Control and invisible
    /// format characters are ignored.
    pub fn insert(&mut self, c: char) {
        if is_ignored(c) {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Inserts pasted text at the cursor and moves the cursor past it.
    ///
    /// Blank lines at the start are skipped: a line copied from a terminal or an editor
    /// often begins with the previous line's break. A tab becomes a space (so a pasted
    /// `:fen\t<FEN>` keeps its words apart); other control and invisible format characters
    /// are stripped. Insertion stops at the next line break (`\n`, `\r`, U+0085, U+2028 or
    /// U+2029); the rest of `text` is dropped. Returns `true` when that line break was seen,
    /// so the caller can submit the line. Text made only of blank lines inserts nothing and
    /// returns `true` when the editor already holds text (the paste is an Enter), else
    /// `false`.
    pub fn insert_str(&mut self, text: &str) -> bool {
        if text.contains(is_line_break) && text.chars().all(|c| c.is_whitespace() || is_ignored(c))
        {
            return !self.text.is_empty();
        }
        let text = skip_blank_lines(text);
        let (line, saw_line_break) = match text.find(is_line_break) {
            Some(end) => (&text[..end], true),
            None => (text, false),
        };
        let clean: String = line
            .chars()
            .map(|c| if c == '\t' { ' ' } else { c })
            .filter(|&c| !is_ignored(c))
            .collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        saw_line_break
    }

    /// Deletes the character before the cursor, if any.
    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
            self.text.remove(self.cursor);
        }
    }

    /// Deletes the character after the cursor, if any.
    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// Moves the cursor one character left, if possible.
    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.cursor].chars().next_back() {
            self.cursor -= c.len_utf8();
        }
    }

    /// Moves the cursor one character right, if possible.
    pub fn right(&mut self) {
        if let Some(c) = self.text[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }

    /// Moves the cursor to the start of the line.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end of the line.
    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Empties the editor.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Returns the text and empties the editor.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

/// Characters that end a pasted line.
fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `text` after the blank lines it starts with (lines of whitespace and characters the
/// editor ignores, each ended by a line break).
fn skip_blank_lines(mut text: &str) -> &str {
    while let Some(end) = text.find(is_line_break) {
        if !text[..end]
            .chars()
            .all(|c| c.is_whitespace() || is_ignored(c))
        {
            break;
        }
        let break_len = text[end..].chars().next().map_or(1, char::len_utf8);
        text = &text[end + break_len..];
    }
    text
}

/// Characters the editor never stores: C0/C1 controls, line and paragraph separators, and
/// invisible format characters (soft hyphen, combining grapheme joiner, Arabic letter mark,
/// Mongolian vowel separator, zero-width space/joiners, bidi marks, embeddings, overrides
/// and isolates, word joiner and invisible operators, byte-order mark, interlinear
/// annotation marks). These would make the drawn text differ from the parsed text.
fn is_ignored(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// A submitted command-box line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Move text (trimmed, non-empty) for [`super::movetext::parse_move`].
    Move(String),
    /// `:undo`: take back the last move (two plies against Jev).
    Undo,
    /// `:flip`: turn the board around.
    Flip,
    /// `:new`: start a new game in the current mode.
    New,
    /// `:fen <FEN>`: load a position. Carries the FEN text, trimmed.
    Fen(String),
    /// `:savefen <path>`: save the current position. Carries the raw path.
    SaveFen(String),
    /// `:savepgn <path>`: save the game. Carries the raw path.
    SavePgn(String),
    /// `:resign`: resign the game.
    Resign,
    /// `:glyphs`: cycle the piece glyph set.
    Glyphs,
    /// `:help`: show the help dialog.
    Help,
    /// `:quit`: leave the program.
    Quit,
}

/// Parses a submitted command-box line.
///
/// Lines starting with `:` are commands; the name is case-insensitive and is separated
/// from its argument by whitespace. `:fen` needs a FEN, `:savefen` and `:savepgn` need a
/// path (one pair of matching surrounding quotes is removed from paths), and the other
/// commands take no argument. Any other non-empty line is [`Command::Move`] with the
/// trimmed text.
///
/// # Errors
///
/// A short message for the status line: `"type a move or :help"` for a blank line,
/// `"unknown command: :x (try :help)"` (a long name is cut by [`shorten`]),
/// `"usage: :fen <FEN>"` (and likewise for the save
/// commands) when the argument is missing, or `":undo takes no argument"` when one is given
/// to a command that takes none.
pub fn parse_command(line: &str) -> Result<Command, String> {
    let line = line.trim();
    if line.is_empty() {
        return Err("type a move or :help".to_string());
    }
    let Some(rest) = line.strip_prefix(':') else {
        return Ok(Command::Move(line.to_string()));
    };
    let rest = rest.trim_start();
    let (name, arg) = match rest.find(char::is_whitespace) {
        Some(end) => (&rest[..end], rest[end..].trim()),
        None => (rest, ""),
    };
    if name.is_empty() {
        return Err("type a command after ':' (try :help)".to_string());
    }

    let key = name.to_ascii_lowercase();
    let bare = |command: Command| {
        if arg.is_empty() {
            Ok(command)
        } else {
            Err(format!(":{key} takes no argument"))
        }
    };
    let with_arg = |make: fn(String) -> Command, value: &str, what: &str| {
        if value.is_empty() {
            Err(format!("usage: :{key} {what}"))
        } else {
            Ok(make(value.to_string()))
        }
    };

    match key.as_str() {
        "undo" => bare(Command::Undo),
        "flip" => bare(Command::Flip),
        "new" => bare(Command::New),
        "resign" => bare(Command::Resign),
        "glyphs" => bare(Command::Glyphs),
        "help" => bare(Command::Help),
        "quit" => bare(Command::Quit),
        "fen" => with_arg(Command::Fen, arg, "<FEN>"),
        "savefen" => with_arg(Command::SaveFen, unquote(arg), "<path>"),
        "savepgn" => with_arg(Command::SavePgn, unquote(arg), "<path>"),
        _ => Err(format!("unknown command: :{} (try :help)", shorten(name))),
    }
}

/// Removes one pair of matching surrounding `"` or `'` quotes, then trims inside them.
/// `arg` is expected trimmed. Used for the paths of `:savefen`, `:savepgn` and the save
/// dialog alike.
pub fn unquote(arg: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = arg
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.trim();
        }
    }
    arg
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn editor(text: &str) -> LineEditor {
        let mut e = LineEditor::new();
        assert!(!e.insert_str(text));
        e
    }

    #[test]
    fn starts_empty() {
        let e = LineEditor::default();
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);
        assert!(e.is_empty());
        assert_eq!(e.before_cursor(), "");
    }

    #[test]
    fn insert_appends_and_advances() {
        let mut e = LineEditor::new();
        for c in "Nf3".chars() {
            e.insert(c);
        }
        assert_eq!(e.text(), "Nf3");
        assert_eq!(e.cursor(), 3);
        assert!(!e.is_empty());
    }

    #[test]
    fn insert_in_the_middle() {
        let mut e = editor("e24");
        e.left();
        e.insert('e');
        assert_eq!(e.text(), "e2e4");
        assert_eq!(e.cursor(), 3);
        assert_eq!(e.before_cursor(), "e2e");
    }

    #[test]
    fn insert_ignores_control_and_invisible_characters() {
        let mut e = LineEditor::new();
        for c in [
            '\n', '\r', '\t', '\x1b', '\x7f', '\0', '\u{85}', '\u{200B}', '\u{200D}', '\u{202E}',
            '\u{2028}', '\u{2066}', '\u{FEFF}',
        ] {
            e.insert(c);
        }
        assert_eq!(e.text(), "");
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn multi_byte_editing() {
        // 2-, 3- and 4-byte characters, including a double-width one.
        let mut e = editor("é♞日🙂");
        assert_eq!(e.cursor(), 4);
        e.left();
        e.left();
        assert_eq!(e.cursor(), 2);
        assert_eq!(e.before_cursor(), "é♞");
        e.insert('ß');
        assert_eq!(e.text(), "é♞ß日🙂");
        e.backspace();
        e.backspace();
        assert_eq!(e.text(), "é日🙂");
        assert_eq!(e.cursor(), 1);
        e.delete();
        assert_eq!(e.text(), "é🙂");
        e.right();
        assert_eq!(e.cursor(), 2);
        e.right();
        assert_eq!(e.cursor(), 2);
        e.backspace();
        assert_eq!(e.text(), "é");
        e.home();
        e.delete();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn backspace_and_delete_at_the_edges_do_nothing() {
        let mut e = editor("ab");
        e.delete();
        assert_eq!((e.text(), e.cursor()), ("ab", 2));
        e.home();
        e.backspace();
        assert_eq!((e.text(), e.cursor()), ("ab", 0));
        e.delete();
        assert_eq!((e.text(), e.cursor()), ("b", 0));
        e.end();
        e.backspace();
        assert_eq!((e.text(), e.cursor()), ("", 0));
        e.backspace();
        e.delete();
        assert_eq!((e.text(), e.cursor()), ("", 0));
    }

    #[test]
    fn cursor_movement_is_clamped() {
        let mut e = editor("ab");
        e.right();
        assert_eq!(e.cursor(), 2);
        e.left();
        e.left();
        e.left();
        assert_eq!(e.cursor(), 0);
        e.end();
        assert_eq!(e.cursor(), 2);
        e.home();
        assert_eq!(e.cursor(), 0);
        let mut empty = LineEditor::new();
        empty.left();
        empty.right();
        empty.home();
        empty.end();
        assert_eq!(empty.cursor(), 0);
    }

    #[test]
    fn insert_str_without_line_break() {
        let mut e = LineEditor::new();
        assert!(!e.insert_str("e2e4"));
        assert_eq!(e.text(), "e2e4");
        assert_eq!(e.cursor(), 4);
        assert!(!e.insert_str(""));
        assert_eq!(e.text(), "e2e4");
    }

    #[test]
    fn insert_str_stops_at_the_first_line_break() {
        for (pasted, kept) in [
            ("e2e4\ne7e5", "e2e4"),
            ("e2e4\r\ne7e5", "e2e4"),
            ("e2e4\re7e5", "e2e4"),
            ("Nf3\u{2028}Nc6", "Nf3"),
            ("d4\n", "d4"),
            ("\ne4\n", "e4"),
            ("\r\n  \n\u{200B}\ne4\ne5", "e4"),
        ] {
            let mut e = LineEditor::new();
            assert!(e.insert_str(pasted), "{pasted:?}");
            assert_eq!(e.text(), kept, "{pasted:?}");
            assert_eq!(e.cursor(), kept.chars().count(), "{pasted:?}");
        }
    }

    #[test]
    fn insert_str_skips_leading_blank_lines() {
        // A copied line that starts with the previous line's break is not lost.
        let mut e = LineEditor::new();
        assert!(!e.insert_str("\nlater"));
        assert_eq!(e.text(), "later");
        // Only blank lines into an empty editor: nothing is inserted or submitted.
        let mut e = LineEditor::new();
        for blank in ["\n", "\r\n", " \n\t\n", "\u{2028}"] {
            assert!(!e.insert_str(blank), "{blank:?}");
        }
        assert_eq!(e.text(), "");
        // Leading spaces on the first real line are kept: they may separate words.
        let mut e = editor(":fen");
        assert!(!e.insert_str(" 8/8/8/8/8/8/8/K6k w - - 0 1"));
        assert_eq!(e.text(), ":fen 8/8/8/8/8/8/8/K6k w - - 0 1");
        assert_eq!(skip_blank_lines("\n\n x\ny"), " x\ny");
        assert_eq!(skip_blank_lines("x\n\ny"), "x\n\ny");
    }

    #[test]
    fn insert_str_strips_control_characters() {
        let mut e = LineEditor::new();
        assert!(!e.insert_str("\x1b[31me2\te4\x07\u{200B}\u{FEFF}"));
        assert_eq!(e.text(), "[31me2 e4");
        assert_eq!(e.cursor(), 9);
    }

    #[test]
    fn a_pasted_tab_becomes_a_space() {
        let mut e = LineEditor::new();
        assert!(e.insert_str(":fen\t8/8/8/8/8/8/8/K6k\tw\t-\t-\t0\t1\n"));
        assert_eq!(e.text(), ":fen 8/8/8/8/8/8/8/K6k w - - 0 1");
        assert_eq!(
            parse_command(e.text()),
            Ok(Command::Fen("8/8/8/8/8/8/8/K6k w - - 0 1".to_string()))
        );
    }

    #[test]
    fn a_paste_of_only_line_breaks_submits_the_text_in_the_editor() {
        for blank in ["\n", "\r\n", "\n\n", " \n\t\n", "\u{2028}", "\u{200B}\r"] {
            let mut e = editor("e4");
            assert!(e.insert_str(blank), "{blank:?}");
            assert_eq!(e.text(), "e4", "{blank:?}");
            assert_eq!(e.cursor(), 2, "{blank:?}");
        }
        // Without a line break it is text to insert.
        let mut e = editor("e4");
        assert!(!e.insert_str(" "));
        assert_eq!(e.text(), "e4 ");
    }

    #[test]
    fn invisible_format_characters_are_ignored() {
        for c in [
            '\u{00AD}', '\u{034F}', '\u{061C}', '\u{180E}', '\u{FFF9}', '\u{FFFA}', '\u{FFFB}',
        ] {
            assert!(is_ignored(c), "{c:?}");
            let mut e = LineEditor::new();
            e.insert(c);
            assert_eq!(e.text(), "", "{c:?}");
            assert!(!e.insert_str(&format!("e{c}4")));
            assert_eq!(e.text(), "e4", "{c:?}");
        }
    }

    #[test]
    fn insert_str_at_the_cursor() {
        let mut e = editor(":fen  w - - 0 1");
        e.home();
        for _ in 0..5 {
            e.right();
        }
        assert!(!e.insert_str("8/8/8/8/8/8/8/K6k"));
        assert_eq!(e.text(), ":fen 8/8/8/8/8/8/8/K6k w - - 0 1");
        assert_eq!(e.cursor(), 22);
    }

    #[test]
    fn insert_str_multi_byte() {
        let mut e = editor("ab");
        e.left();
        assert!(e.insert_str("♚日\n♛"));
        assert_eq!(e.text(), "a♚日b");
        assert_eq!(e.cursor(), 3);
        assert_eq!(e.before_cursor(), "a♚日");
    }

    #[test]
    fn clear_and_take() {
        let mut e = editor("Nf3");
        e.left();
        e.clear();
        assert_eq!((e.text(), e.cursor()), ("", 0));

        let mut e = editor("e4");
        e.home();
        assert_eq!(e.take(), "e4");
        assert_eq!((e.text(), e.cursor()), ("", 0));
        assert_eq!(e.take(), "");
        e.insert('d');
        assert_eq!((e.text(), e.cursor()), ("d", 1));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Insert(char),
        InsertStr(String),
        Backspace,
        Delete,
        Left,
        Right,
        Home,
        End,
        Clear,
        Take,
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            any::<char>().prop_map(Op::Insert),
            any::<String>().prop_map(Op::InsertStr),
            Just(Op::Backspace),
            Just(Op::Delete),
            Just(Op::Left),
            Just(Op::Right),
            Just(Op::Home),
            Just(Op::End),
            Just(Op::Clear),
            Just(Op::Take),
        ]
    }

    proptest! {
        /// Any edit sequence keeps the cursor in range and the text free of ignored
        /// characters, and never panics on multi-byte input.
        #[test]
        fn edits_keep_invariants(ops in proptest::collection::vec(op(), 0..64)) {
            let mut e = LineEditor::new();
            for op in ops {
                match op {
                    Op::Insert(c) => e.insert(c),
                    Op::InsertStr(s) => {
                        let saw = e.insert_str(&s);
                        prop_assert_eq!(saw, skip_blank_lines(&s).contains(is_line_break));
                    }
                    Op::Backspace => e.backspace(),
                    Op::Delete => e.delete(),
                    Op::Left => e.left(),
                    Op::Right => e.right(),
                    Op::Home => e.home(),
                    Op::End => e.end(),
                    Op::Clear => e.clear(),
                    Op::Take => {
                        let before = e.text().to_string();
                        prop_assert_eq!(e.take(), before);
                    }
                }
                prop_assert!(e.cursor() <= e.text().chars().count());
                prop_assert!(e.text().starts_with(e.before_cursor()));
                prop_assert!(!e.text().chars().any(is_ignored));
            }
        }
    }

    #[test]
    fn move_lines() {
        assert_eq!(parse_command("e4"), Ok(Command::Move("e4".to_string())));
        assert_eq!(
            parse_command("  Nf3  "),
            Ok(Command::Move("Nf3".to_string()))
        );
        assert_eq!(
            parse_command("e2e4 e7e5"),
            Ok(Command::Move("e2e4 e7e5".to_string()))
        );
    }

    #[test]
    fn empty_lines() {
        assert_eq!(parse_command(""), Err("type a move or :help".to_string()));
        assert_eq!(
            parse_command(" \t "),
            Err("type a move or :help".to_string())
        );
        assert_eq!(
            parse_command(":"),
            Err("type a command after ':' (try :help)".to_string())
        );
        assert_eq!(
            parse_command(" :  "),
            Err("type a command after ':' (try :help)".to_string())
        );
    }

    #[test]
    fn commands_without_arguments() {
        for (line, command) in [
            (":undo", Command::Undo),
            (":flip", Command::Flip),
            (":new", Command::New),
            (":resign", Command::Resign),
            (":glyphs", Command::Glyphs),
            (":help", Command::Help),
            (":quit", Command::Quit),
            ("  :undo  ", Command::Undo),
            (": flip", Command::Flip),
            (":HELP", Command::Help),
            (":Quit", Command::Quit),
        ] {
            assert_eq!(parse_command(line), Ok(command), "{line}");
        }
    }

    #[test]
    fn commands_reject_unexpected_arguments() {
        assert_eq!(
            parse_command(":undo 2"),
            Err(":undo takes no argument".to_string())
        );
        assert_eq!(
            parse_command(":New game"),
            Err(":new takes no argument".to_string())
        );
    }

    #[test]
    fn fen_command() {
        let fen = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1";
        assert_eq!(
            parse_command(&format!(":fen {fen}")),
            Ok(Command::Fen(fen.to_string()))
        );
        assert_eq!(
            parse_command(&format!(":FEN   {fen}  ")),
            Ok(Command::Fen(fen.to_string()))
        );
        assert_eq!(
            parse_command(&format!(":fen\t{fen}")),
            Ok(Command::Fen(fen.to_string()))
        );
        assert_eq!(parse_command(":fen"), Err("usage: :fen <FEN>".to_string()));
        assert_eq!(
            parse_command(":fen   "),
            Err("usage: :fen <FEN>".to_string())
        );
    }

    #[test]
    fn save_commands() {
        assert_eq!(
            parse_command(":savefen ~/pos"),
            Ok(Command::SaveFen("~/pos".to_string()))
        );
        assert_eq!(
            parse_command(":savepgn games/my game.pgn"),
            Ok(Command::SavePgn("games/my game.pgn".to_string()))
        );
        assert_eq!(
            parse_command(r#":savepgn "my game.pgn""#),
            Ok(Command::SavePgn("my game.pgn".to_string()))
        );
        assert_eq!(
            parse_command(":savefen 'a b'"),
            Ok(Command::SaveFen("a b".to_string()))
        );
        // Unmatched quotes are part of the path.
        assert_eq!(
            parse_command(r#":savefen "odd"#),
            Ok(Command::SaveFen(r#""odd"#.to_string()))
        );
        assert_eq!(
            parse_command(":savefen"),
            Err("usage: :savefen <path>".to_string())
        );
        assert_eq!(
            parse_command(":savepgn  "),
            Err("usage: :savepgn <path>".to_string())
        );
        assert_eq!(
            parse_command(r#":savepgn """#),
            Err("usage: :savepgn <path>".to_string())
        );
    }

    #[test]
    fn unknown_commands() {
        assert_eq!(
            parse_command(":x"),
            Err("unknown command: :x (try :help)".to_string())
        );
        assert_eq!(
            parse_command(":Save foo"),
            Err("unknown command: :Save (try :help)".to_string())
        );
        assert_eq!(
            parse_command(":日本"),
            Err("unknown command: :日本 (try :help)".to_string())
        );
        assert_eq!(
            parse_command(&format!(":{}", "x".repeat(30))),
            Err(format!("unknown command: :{}… (try :help)", "x".repeat(24)))
        );
    }
}
