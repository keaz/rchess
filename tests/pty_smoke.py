#!/usr/bin/env python3
"""Pseudo-terminal smoke test for the rchess TUI (spec 6.6/6.7 and 9.3/9.4).

Not run by cargo: `python3 tests/pty_smoke.py [--release] [--no-build]`.

Each scenario starts the binary on a fresh 80x24 pty whose environment has no
JEV_API_KEY, TYPESAFE_API_KEY or LAYA_API_KEY, and no LAYA_URL unless a scenario
sets it (so the computer players are local search and nothing touches the
network), drives it with keystrokes or signals, and checks:

* setup writes ?1049h, then ?1000h ?1002h ?1006h (click-and-drag mouse, SGR)
  and ?2004h (bracketed paste), and never ?1003h (any-motion) or ?1015h;
* RCHESS_IMAGES=off is set unless a scenario is about the graphics query, so
  the query is skipped and none of its bytes are written;
* the menu and the game render (a tiny terminal emulator rebuilds the screen);
* `1`, `/e4<Enter>`, `Esc`, `q`, `y` plays 1. e4 and quits after confirmation,
  and `qh5<Enter>` typed on the board does not quit (the question defaults to No);
  before a move is played, `q` quits at once (nothing to lose);
* `3` (play Black against the computer) gets a first move from the engine
  thread, and without a key the screen calls the computer "Local search";
  without debug mode `d` says so;
* `7` (Jev vs Laya) with LAYA_URL pointing at a closed port on 127.0.0.1: the
  menu says Laya is ready, Laya's moves fall back to the local search with a
  "Laya unavailable" note, the game goes on, and quitting restores the terminal;
* --debug (and RCHESS_DEBUG=1) against the local search: the Status border says
  DEBUG, `d` shows an exchange view with "no Jev requests yet", and no debug log
  is created, since no Jev request was made;
* the graphics query (spec 9.3), on a pty that
  - never answers: the query is written once, in raw mode, between ?1049h and
    ?1000h; start-up goes on after about 1 s with a menu warning, keys typed
    afterwards work, the Image style (half blocks) is in the `g` cycle, showing
    solid glyphs on 80x24's small squares and pictures at 200x60, and the
    terminal is restored;
  - answers late, while the menu is up: the answer does not act as key presses
    (its `3` would start a game);
  - answers late in two reads, the status report alone in the second: the query
    ends with the device attributes after the status request, whose answer comes
    last, so the UI still reacts to a resize without a key;
  - answers partly before the deadline and the rest just after it: the rest is
    read and dropped, so its `3` does not start a game either;
  - gets SIGTERM or hangs up while the query waits: the process ends by that
    signal at once (before the query's 1 s deadline has run out), SIGTERM
    with the terminal restored, and answers that come
    right after the signal are read and dropped, not left for the shell;
  - runs in WezTerm (from the environment): only the cell size, the status and
    then the device attributes are asked, and a late answer, which ends with the
    device attributes, does not leave the UI waiting for a key;
  - answers like a Sixel terminal while the environment names iTerm2: the
    pictures are iTerm2 pictures, not Sixel;
  - answers like Kitty: start-up goes on at once, the answer is not echoed,
    Image is the starting style and pieces are kitty pictures (unicode
    placeholders) of 3x2 cells of 9x18; a font zoom to 8x16 (a resize) asks for
    the cell size again, as Kitty and Ghostty size a placeholder picture from its
    pixels and the current cell, and with the answer sends new pictures with new
    ids whose pixel size (s, v) follows the new font, and the old pictures are
    deleted by id right before the new ones are sent; a quit, and a SIGTERM,
    delete by id every picture still shown;
  - answers like a Sixel terminal with a 10x20 font, then zooms out to 8x16
    (more cells, and SIGWINCH): the resize asks for the cell size again (CSI 16 t,
    the status request and the device attributes, nothing else), and with the
    answer the pictures are re-encoded for 8x16 cells, so none spills out of its
    square; the window reports no pixels, so only the answer can give the new
    size; a resize whose query is not answered keeps 8x16 and the game goes on
    after about 1 s, and its late answers, which end with the device attributes,
    do not leave the UI waiting for a key (a resize right after them is handled
    at once); a resize that comes after a query gave up but before its late
    answers arrive takes its own answer (11x22), not the late one, and the UI
    still reacts to the next resize without a key;
* exit writes ?1006l ?1002l ?1000l ?2004l and then ?1049l (after kitty pictures,
  one delete-by-id command for each picture still shown comes first, naming the
  ids the transmissions used, and only then), then only shows the
  cursor again (?25h) and draws nothing on the main screen, exits with status 0,
  and leaves the pty's termios exactly as it was before the program started
  (ICANON and ECHO back on);
* SIGTERM (and SIGINT, SIGHUP) mid-game restore the terminal the same way and
  the process then dies by that signal; two SIGTERMs in a row do too;
* a real hangup (the terminal closes its end of the pty, as a closed window or a
  dropped SSH session does) ends the process by SIGHUP, both with the UI idle
  (crossterm keeps polling the dead tty, so the stuck-UI watchdog restores,
  within about a second) and with the UI blocked writing a frame (a window
  large enough that its frames overflow any pty buffer, which on Linux is
  checked in /proc before the hangup; the write fails and the main thread
  restores, within 1 s): restoring a dead tty must not panic, so there is no SIGABRT from a panic inside the panic hook; every
  hangup scenario closes the pty and checks the exit the same way (`hang_up`);
* a terminal that closes without sending SIGHUP (the program is not in the
  tty's session, as under a supervisor): stdin reports the hangup, and the
  process ends by SIGHUP within about a second instead of spinning;
* debug-build fault injection (RCHESS_FAULT): an engine that panics still gets a
  local search move played without disturbing the screen; a panic on the UI
  thread restores the terminal before the message is printed (status 101); a
  hung UI thread is still restored and ended by SIGTERM;
* --help, a non-terminal stdout and a non-terminal stdin (`chess < /dev/null`)
  never touch the terminal, and errors are printed readably;
* as a control, SIGKILL (which cannot be caught) does leave the pty raw, which
  shows the termios check can fail.

Timing: a check that something happens waits for it for seconds, so a slow
machine (a shared CI runner) only makes the run longer. A check that something
happens soon has an upper bound that sits below a fixed wait in the program
which the bug it looks for would fall into (the query's 1 s deadline, the 1 s
stuck-UI grace), measured from a moment the program cannot have passed yet
(when the query was seen, not when the signal was sent), with room to spare.
There is no setting to widen them: much wider, they would pass that bug. A
check that depends on when the harness itself acted (it saw the query late, or
slept too long) is skipped with a NOTE when that timing was missed, rather than
failed.

Exits 0 when every check passes, 1 otherwise.
"""

import argparse
import codecs
import fcntl
import json
import os
import platform
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty
import unicodedata

ROWS, COLS = 24, 80
CRATE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SETUP = [b"\x1b[?1049h", b"\x1b[?1000h", b"\x1b[?1002h", b"\x1b[?1006h", b"\x1b[?2004h"]
TEARDOWN = [b"\x1b[?1006l", b"\x1b[?1002l", b"\x1b[?1000l", b"\x1b[?2004l", b"\x1b[?1049l"]
FORBIDDEN = [b"\x1b[?1003h", b"\x1b[?1015h"]
SHOW_CURSOR = b"\x1b[?25h"
# The graphics query (ratatui-image's Parser::query): the kitty probe comes first
# and the status report request ends it.
QUERY_START = b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
QUERY_END = b"\x1b[5n"
QUERY_PARTS = [QUERY_START, b"\x1b[c", b"\x1b[16t", QUERY_END]
# The query ends with the device attributes again, after the status request: the query
# stops reading at the status report, and the answer after it wakes crossterm when the
# answers come too late for the query.
QUERY_TAIL = QUERY_END + b"\x1b[c"
# What Kitty answers: graphics OK, device attributes without sixel, a 9x18 pixel
# cell and the status report.
KITTY_ANSWER = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;c\x1b[6;18;9t\x1b[0n"
# What a Sixel terminal answers: no kitty graphics, device attributes with sixel
# (4), a 10x20 pixel cell and the status report.
SIXEL_ANSWER = b"\x1b[?62;4c\x1b[6;20;10t\x1b[0n"
# A Sixel picture's raster attributes: its width and height in pixels.
SIXEL_RASTER = re.compile(rb'\x1bP[0-9;]*q"1;1;(\d+);(\d+)')
QUERY_WARNING = "graphics query: no answer within 1 s"
# What the app asks after a resize while its pictures are Sixel, iTerm2 or Kitty: the cell
# size and the status report, which ends the answers the app reads, then the device
# attributes, whose answer comes last and is left to crossterm.
FONT_QUERY = b"\x1b[16t\x1b[5n"
ATTRIBUTES_REQUEST = b"\x1b[c"
FULL_FONT_QUERY = FONT_QUERY + ATTRIBUTES_REQUEST
# A Sixel terminal's answer to the device attributes request.
ATTRIBUTES_ANSWER = b"\x1b[?62;4c"
# A Sixel terminal's answer to the font query after a zoom to an 8x16 pixel cell.
ZOOMED_ANSWER = b"\x1b[6;16;8t\x1b[0n" + ATTRIBUTES_ANSWER
# Kitty's command to delete one image by id and free its data (uppercase I), written
# at exit once for every kitty picture sent. Pictures are virtual placements, which
# Kitty deletes only by id (a=d,d=A leaves them).
KITTY_DELETE = re.compile(rb"\x1b_Ga=d,d=I,i=(\d+)\x1b\\")
# Any kitty delete command, whatever it names.
KITTY_ANY_DELETE = re.compile(rb"\x1b_Ga=d")
# The first chunk of a kitty picture's transmission, which names its id.
KITTY_TRANSMIT = re.compile(rb"\x1b_Gq=2,i=(\d+),a=T,U=1")
# The same, with the picture's width (s) and height (v) in pixels.
KITTY_TRANSMIT_SIZE = re.compile(rb"\x1b_Gq=2,i=\d+,a=T,U=1,[^;]*?s=(\d+),v=(\d+),")
# The start-up query in WezTerm and Konsole: no kitty or sixel probe, the cell size, the
# status report, then the device attributes, whose answer comes last.
WEZTERM_QUERY = b"\x1b[16t\x1b[5n\x1b[c"
# WezTerm's answers to the cell size and status requests (an 8x16 cell), and to the
# device attributes request.
WEZTERM_ANSWER = b"\x1b[6;16;8t\x1b[0n"
WEZTERM_ATTRIBUTES_ANSWER = b"\x1b[?65;4;6;18;22c"
# The start of an iTerm2 picture (OSC 1337 File).
ITERM2_PICTURE = b"\x1b]1337;File="
# Kitty's answer to the device attributes request (no sixel).
KITTY_ATTRIBUTES_ANSWER = b"\x1b[?62;c"
# Kitty's unicode placeholder: every cell of a kitty picture holds one.
PLACEHOLDER = "\U0010EEEE"
SECRET_VARS = ("JEV_API_KEY", "TYPESAFE_API_KEY", "LAYA_API_KEY")

failures = []


def check(ok, what, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {what}" + (f"  ({detail})" if detail else ""))
    if not ok:
        failures.append(what)
    return ok


def ordered(stream, needles, start=0):
    """Offsets of each needle, each found after the previous one; None when missing."""
    offsets, pos = [], start
    for needle in needles:
        found = stream.find(needle, pos)
        if found < 0:
            return None
        offsets.append(found)
        pos = found + len(needle)
    return offsets


# ----- a minimal terminal emulator: enough of VT100 to read ratatui's frames -----

CSI = re.compile(r"\x1b\[([0-9;?]*)[ -/]*([@-~])")


class Screen:
    """Replays the byte stream onto a grid: cursor moves, clears and printable text.

    Colours are ignored. Zero-width characters (such as U+FE0E after the pawn, or
    the diacritics after a kitty placeholder) join the previous cell. Kitty and
    sixel pictures (APC and DCS strings) are skipped; their placeholder cells are
    text. Only the alternate screen is kept.
    """

    def __init__(self, text):
        self.grid = self._blank()
        self.row = self.col = 0
        self._feed(text)

    @staticmethod
    def _blank():
        return [[" "] * COLS for _ in range(ROWS)]

    def _put(self, ch):
        if unicodedata.category(ch) in ("Mn", "Me", "Cf"):
            if 0 < self.col <= COLS and 0 <= self.row < ROWS:
                self.grid[self.row][self.col - 1] += ch
            return
        if 0 <= self.row < ROWS and 0 <= self.col < COLS:
            self.grid[self.row][self.col] = ch
        self.col += 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1

    def _csi(self, params, final):
        private = params.startswith("?")
        nums = [int(p) if p.isdigit() else 0 for p in params.lstrip("?").split(";")]
        n = nums[0] or 1
        if private:
            if nums[0] == 1049 and final in "hl":
                self.grid, self.row, self.col = self._blank(), 0, 0
        elif final in "Hf":
            self.row = (nums[0] or 1) - 1
            self.col = ((nums[1] if len(nums) > 1 else 0) or 1) - 1
        elif final == "J":
            if nums[0] in (2, 3):
                self.grid = self._blank()
            elif nums[0] == 0:
                for c in range(self.col, COLS):
                    self.grid[self.row][c] = " "
                for r in range(self.row + 1, ROWS):
                    self.grid[r] = [" "] * COLS
        elif final == "K":
            span = range(self.col, COLS) if nums[0] == 0 else range(COLS)
            for c in span:
                self.grid[self.row][c] = " "
        elif final == "A":
            self.row = max(0, self.row - n)
        elif final == "B":
            self.row = min(ROWS - 1, self.row + n)
        elif final == "C":
            self.col = min(COLS - 1, self.col + n)
        elif final == "D":
            self.col = max(0, self.col - n)
        elif final == "G":
            self.col = n - 1
        elif final == "d":
            self.row = n - 1

    def _feed(self, text):
        i = 0
        while i < len(text):
            ch = text[i]
            if ch == "\x1b":
                if text.startswith("[", i + 1):
                    m = CSI.match(text, i)
                    if not m:
                        return  # incomplete sequence at the end of the stream
                    self._csi(m.group(1), m.group(2))
                    i = m.end()
                elif text.startswith("]", i + 1):
                    ends = [e for e in (text.find("\x07", i), text.find("\x1b\\", i)) if e >= 0]
                    if not ends:
                        return
                    i = min(ends) + (1 if text[min(ends)] == "\x07" else 2)
                elif text[i + 1 : i + 2] in ("_", "P", "^", "X"):
                    # APC (kitty graphics), DCS (sixel), PM and SOS run to ST.
                    end = text.find("\x1b\\", i + 2)
                    if end < 0:
                        return
                    i = end + 2
                else:
                    i += 2
                continue
            if ch == "\r":
                self.col = 0
            elif ch == "\n":
                self.row = min(ROWS - 1, self.row + 1)
            elif ch == "\b":
                self.col = max(0, self.col - 1)
            elif ch >= " ":
                self._put(ch)
            i += 1

    def lines(self):
        return ["".join(cells) for cells in self.grid]

    def text(self):
        return "\n".join(self.lines())

    def show(self, title):
        print(f"  --- screen: {title} ---")
        for line in self.lines():
            print("  |" + line.rstrip())


# ----- running the binary on a pty -----


HOME = tempfile.TemporaryDirectory(prefix="rchess-pty-home-")


def child_env(extra=None):
    """The child's whole environment. Images are off, so the graphics query is
    skipped, unless `extra` maps RCHESS_IMAGES to None (None removes a variable)."""
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": HOME.name,
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8",
        "LC_ALL": "en_US.UTF-8",
        "COLORTERM": "truecolor",
        "RCHESS_IMAGES": "off",
    }
    for name, value in (extra or {}).items():
        if value is None:
            env.pop(name, None)
        else:
            env[name] = value
    assert not any(v in env for v in SECRET_VARS)
    return env


# The environment for the scenarios that ask the terminal about graphics.
QUERY_ENV = {"RCHESS_IMAGES": None}


class App:
    """The binary running on its own pty, with everything it wrote recorded."""

    def __init__(self, binary, args=(), stdout_pipe=False, stdin_null=False, env=None, ctty=True):
        self.master, self.slave = os.openpty()
        # Anything the child writes comes after this.
        self.started_at = time.monotonic()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        # Read through the master: on macOS the slave is revoked when the child's
        # session ends, but the master still reports the pty's termios.
        self.termios_before = termios.tcgetattr(self.master)
        self.stream = b""
        self.pipe_r = None
        pipe_w = None
        if stdout_pipe:
            self.pipe_r, pipe_w = os.pipe()
        self.pid = os.fork()
        if self.pid == 0:  # child
            try:
                os.close(self.master)
                if ctty:
                    os.login_tty(self.slave)  # setsid, controlling tty, stdin/stdout/stderr
                else:
                    # A session of its own without a controlling tty, as under a supervisor:
                    # the pty is only its stdin, stdout and stderr, and its hangup sends no
                    # SIGHUP here.
                    os.setsid()
                    for fd in (0, 1, 2):
                        os.dup2(self.slave, fd)
                    os.close(self.slave)
                if pipe_w is not None:
                    os.dup2(pipe_w, 1)
                if stdin_null:
                    os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
                os.execve(binary, [binary, *args], child_env(env))
            finally:
                os._exit(127)
        # The parent keeps the slave open, so the pty stays readable after the child exits.
        if pipe_w is not None:
            os.close(pipe_w)
        self.status = None
        self.exited_at = None

    def pump(self, timeout=0.02):
        ready, _, _ = select.select([self.master], [], [], timeout)
        if ready:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                data = b""
            self.stream += data
            return bool(data)
        return False

    def text(self):
        return codecs.getincrementaldecoder("utf-8")("replace").decode(self.stream)

    def screen(self):
        return Screen(self.text())

    def send(self, data, settle=0.15):
        os.write(self.master, data)
        self.idle(settle)

    def idle(self, seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            self.pump(0.01)

    def wait_screen(self, needle, timeout=5.0):
        """Waits until `needle` is on screen, then lets the rest of the frame arrive."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump(0.02)
            if needle in self.screen().text():
                self.idle(0.1)
                return True
        return False

    def wait_screen_without(self, needle, timeout=5.0):
        """Waits until `needle` is no longer on screen, then lets the rest of the frame arrive."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if needle not in self.screen().text():
                self.idle(0.1)
                return True
            self.pump(0.02)
        return False

    def wait_bytes(self, needle, timeout=5.0, start=0):
        """Waits until `needle` is in the stream from offset `start` on."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if needle in self.stream[start:]:
                return True
            self.pump(0.02)
        return needle in self.stream[start:]

    def wait_exit(self, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump(0.01)
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.exited_at = time.monotonic()
                self.status = os.waitstatus_to_exitcode(status)
                self.idle(0.2)  # drain what is left in the pty
                return True
        return False

    def kill(self):
        if self.status is None:
            try:
                os.kill(self.pid, signal.SIGKILL)
                os.waitpid(self.pid, 0)
            except OSError:
                pass

    def pipe_output(self):
        chunks = []
        while True:
            data = os.read(self.pipe_r, 65536)
            if not data:
                return b"".join(chunks)
            chunks.append(data)

    def close(self):
        self.kill()
        for fd in (self.master, self.slave, self.pipe_r):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass


# ----- checks shared by the scenarios -----


def check_setup(app, query=False):
    """Checks the setup sequences; with `query`, that the graphics query is written
    once, after ?1049h and before the first mouse or paste sequence (?1000h to
    ?2004h), and otherwise that none of it is written."""
    check(app.wait_bytes(SETUP[-1]), "setup sequences arrive")
    offsets = ordered(app.stream, SETUP)
    check(
        offsets is not None,
        "setup order: ?1049h, ?1000h, ?1002h, ?1006h, ?2004h",
        f"offsets {offsets}",
    )
    if query:
        offsets = ordered(app.stream, SETUP[:1] + QUERY_PARTS + SETUP[1:2])
        check(
            offsets is not None,
            "graphics query between ?1049h and ?1000h, status report last",
            f"offsets {offsets}",
        )
        check(app.stream.count(QUERY_START) == 1, "the query is written once")
        check(QUERY_TAIL in app.stream, "the status request is followed by the device attributes")
        query_end = app.stream.find(QUERY_END)
        early = [seq for seq in SETUP[1:] if 0 <= app.stream.find(seq) < query_end]
        check(not early, "no mouse or paste sequence before the query", f"found {early}")
    else:
        sent = [part for part in QUERY_PARTS if part in app.stream]
        check(not sent, "images off: no graphics query", f"found {sent}")


def kitty_ids(stream):
    """The ids of the kitty pictures transmitted in `stream`."""
    return {int(i) for i in KITTY_TRANSMIT.findall(stream)}


def check_kitty_deletes(app, after, label, live_from=0):
    """Checks that every kitty picture sent is deleted by its id once, with uppercase I
    (which frees its data), and that no other kind of delete is sent; and that exit
    (after `after`) deletes the pictures still shown (those transmitted from `live_from`
    on), all before ?1006l, and nothing else."""
    sent = kitty_ids(app.stream[:after])
    check(bool(sent), f"{label}: kitty pictures were sent")
    check(0 not in sent, f"{label}: no kitty picture has id 0", f"ids {sorted(sent)}")
    deletes = list(KITTY_DELETE.finditer(app.stream))
    deleted = [int(m.group(1)) for m in deletes]
    check(
        set(deleted) == sent and len(deleted) == len(sent),
        f"{label}: each kitty picture sent is deleted by its id, once",
        f"sent {sorted(sent)}, deleted {sorted(deleted)}",
    )
    check(
        len(KITTY_ANY_DELETE.findall(app.stream)) == len(deletes),
        f"{label}: kitty pictures are deleted by id only (no a=d,d=A)",
    )
    live = kitty_ids(app.stream[live_from:after])
    at_exit = [m for m in deletes if m.start() >= after]
    teardown = ordered(app.stream, TEARDOWN, after)
    check(
        {int(m.group(1)) for m in at_exit} == live
        and teardown is not None
        and all(m.start() < teardown[0] for m in at_exit),
        f"{label}: exit deletes the pictures still shown, before ?1006l and ?1049l",
        f"live {sorted(live)}, deleted at exit {[int(m.group(1)) for m in at_exit]}, "
        f"teardown at {teardown}",
    )


def check_dropped_kitty_deleted(app, old_from, old_to, new_from, label, before_new=True):
    """Checks that the kitty pictures transmitted between `old_from` and `old_to`, which
    other pictures replaced, are deleted by id after `new_from` (not at exit): with
    `before_new`, before the first new picture is sent. A new font drops the pictures
    between two frames, so they are deleted before the next one; a new square size is
    found while a frame is drawn, so they are deleted right after it."""
    old = kitty_ids(app.stream[old_from:old_to])
    first_new = KITTY_TRANSMIT.search(app.stream, new_from)
    upto = first_new.start() if before_new and first_new else len(app.stream)
    deleted = {int(i) for i in KITTY_DELETE.findall(app.stream[new_from:upto])}
    when = "before the new ones are sent" if before_new else "right after the new ones"
    check(
        bool(old) and deleted == old,
        f"{label}: the replaced pictures are deleted by id {when}",
        f"old {sorted(old)}, deleted {sorted(deleted)}",
    )


def termios_flags(attrs):
    """The iflag, oflag, cflag and lflag of `attrs`, without PENDIN: macOS sets it on
    every switch back to canonical mode and clears it at the next input or read, so it
    says nothing about the restore (it stays set when nothing is typed afterwards)."""
    iflag, oflag, cflag, lflag = attrs[:4]
    return [iflag, oflag, cflag, lflag & ~getattr(termios, "PENDIN", 0)]


def check_teardown(app, after, label, kitty=False, live_from=0):
    offsets = ordered(app.stream, TEARDOWN, after)
    check(
        offsets is not None,
        f"{label}: teardown order ?1006l ?1002l ?1000l ?2004l then ?1049l",
        f"offsets {offsets}",
    )
    if kitty:
        check_kitty_deletes(app, after, label, live_from)
    else:
        check(not KITTY_ANY_DELETE.search(app.stream), f"{label}: no kitty delete command")
    if offsets is not None:
        tail = app.stream[offsets[-1] + len(TEARDOWN[-1]):]
        # The cursor comes back (it is hidden while the UI runs) and nothing else follows.
        check(
            tail == SHOW_CURSOR,
            f"{label}: only ?25h (cursor shown) after ?1049l",
            f"tail {tail!r}",
        )
    for needle in FORBIDDEN:
        check(needle not in app.stream, f"{label}: never sends {needle.decode()[2:]}")
    after_attrs = termios.tcgetattr(app.master)
    lflag = after_attrs[3]
    check(
        termios_flags(after_attrs) == termios_flags(app.termios_before),
        f"{label}: termios flags restored exactly",
        f"ICANON={bool(lflag & termios.ICANON)} ECHO={bool(lflag & termios.ECHO)}",
    )


# ----- scenarios -----


def scenario_play_and_quit(binary):
    print("scenario: Human vs Human, 1. e4 through the command box, quit with confirmation")
    app = App(binary)
    try:
        check_setup(app)
        menu_ok = app.wait_screen("1. Human vs Human")
        check(menu_ok, "menu renders")
        check("No JEV_API_KEY — local search" in app.screen().text(), "menu shows local search status")
        app.screen().show("menu")

        app.send(b"1")
        check(app.wait_screen("White to move"), "Human vs Human starts (White to move)")
        check("Board" in app.screen().text() and "Command" in app.screen().text(), "board and command box drawn")

        app.send(b"/")
        app.send(b"e4")
        check(app.wait_screen("e4"), "typed text shows in the command box")
        app.send(b"\r")
        check(app.wait_screen("1. e4"), "e4 is played and listed as 1. e4")
        check(app.wait_screen("Black to move"), "Black to move after e4")
        app.screen().show("after 1. e4")

        app.send(b"\x1b", settle=0.3)  # leave the command box
        # Lowercase SAN typed on the board: q opens the quit question, which
        # starts on No, so the Enter after it must not quit.
        app.send(b"qh5")
        check(app.wait_screen("Quit the game in progress?"), "q on the board asks")
        app.send(b"\r", settle=0.3)
        check(app.status is None and os.waitpid(app.pid, os.WNOHANG)[0] == 0, "qh5 + Enter does not quit")
        check(app.wait_screen_without("Quit the game in progress?"), "Enter answered No")
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        app.screen().show("quit confirmation")
        check(app.status is None and os.waitpid(app.pid, os.WNOHANG)[0] == 0, "still running while asking")

        quit_at = len(app.stream)
        sent = time.monotonic()
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        if app.status is not None:
            check(app.status == 0, "exit status 0", f"status {app.status}, {app.exited_at - sent:.2f}s after y")
        check_teardown(app, quit_at, "quit")
        print(f"  info  {len(app.stream)} bytes written in total")
    finally:
        app.close()


def scenario_computer_opens(binary):
    print("scenario: menu 3 (play Black), the offline computer opens as Local search")
    app = App(binary)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"3", settle=0)
        check(app.wait_screen("Black to move", timeout=10.0), "the computer played White's first move")
        text = app.screen().text()
        check(re.search(r"1\. \S+", text.split("Moves", 1)[-1]) is not None, "its move is in the move list")
        check("Local search" in text, "the computer's panel is titled Local search")
        check(
            "Jev" not in text.replace("JEV_API_KEY", ""),
            "without a key the screen never says Jev",
        )
        check("DEBUG" not in text, "no DEBUG tag without debug mode")
        app.screen().show("computer opened")
        app.send(b"d")
        check(
            app.wait_screen("debug mode is off (start with"),
            "d says debug mode is off",
        )
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit vs computer")
    finally:
        app.close()


def scenario_laya_offline(binary):
    print("scenario: menu 7 (Jev vs Laya) with laya-serve down plays on and quits cleanly")
    # Port 1 is privileged and never listened on, so every Laya attempt is refused.
    app = App(binary, env={"LAYA_URL": "http://127.0.0.1:1/v1/systemone"})
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        text = app.screen().text()
        check("Laya ready (" in text, "the menu shows Laya ready with LAYA_URL set")
        check("No JEV_API_KEY — local search" in text, "Jev is local search without a key")
        app.send(b"7", settle=0)
        check(app.wait_screen("Laya unavailable", timeout=15.0), "Laya's move falls back with a note")
        check(app.wait_screen("3.", timeout=20.0), "the game keeps going")
        text = app.screen().text()
        check("Local search vs Laya" in text, "the title names both sides")
        app.screen().show("laya offline")
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit Jev vs Laya")
    finally:
        app.close()


def scenario_signal(binary, signum, repeat=1):
    name = signal.Signals(signum).name
    times = "" if repeat == 1 else f" x{repeat}"
    print(f"scenario: {name}{times} mid-game restores the terminal, then dies by {name}")
    app = App(binary)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"1")
        check(app.wait_screen("White to move"), "game started")
        signal_at = len(app.stream)
        sent = time.monotonic()
        for _ in range(repeat):
            os.kill(app.pid, signum)
        exited = app.wait_exit(timeout=3.0)
        check(exited, f"process exits after {name}{times}")
        if exited:
            # os.waitstatus_to_exitcode gives -N for "terminated by signal N".
            check(
                app.status == -signum,
                f"terminated by {name}, so the shell sees the interruption",
                f"status {app.status}, {app.exited_at - sent:.3f}s after the signal",
            )
        check_teardown(app, signal_at, f"{name}{times}")
    finally:
        app.close()


def wait_status(app, seconds):
    """Waits up to `seconds` for the process to end without reading the pty (it may be
    closed); its exit status (negative: the signal that ended it), or None."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        pid, raw = os.waitpid(app.pid, os.WNOHANG)
        if pid:
            app.status = os.waitstatus_to_exitcode(raw)
            return app.status
        time.sleep(0.01)
    return None


def hang_up(app, within, why, since=None):
    """Closes both ends of the pty, as a terminal emulator or sshd does when it goes away:
    nobody holds the tty any more, so the kernel hangs it up (SIGHUP to the session when
    the pty is the program's controlling tty) and every later read or write fails. Checks
    that the process then ends by SIGHUP, never SIGABRT (restoring a dead terminal must
    not panic), and within `within` seconds of the hangup (or of `since`, an earlier
    moment), which `why` explains."""
    for name in ("slave", "master"):
        os.close(getattr(app, name))
        setattr(app, name, None)
    sent = time.monotonic() if since is None else since
    status = wait_status(app, max(5.0, 2 * within))
    took = time.monotonic() - sent
    check(status is not None, "process exits after the hangup", f"{took:.2f}s")
    if status is None:
        return
    check(
        status != -signal.SIGABRT,
        "no SIGABRT: restoring a dead terminal never panics",
        f"status {status}",
    )
    check(status == -signal.SIGHUP, "terminated by SIGHUP", f"status {status}")
    check(took < within, why, f"{took:.2f}s")


# The `write` and `writev` system call numbers, by machine, for `blocked_in_write`.
WRITE_SYSCALLS = {"x86_64": (1, 20), "aarch64": (64, 66)}


def blocked_in_write(pid):
    """Whether the main thread of `pid` (the UI thread) is blocked in a write: True or
    False where /proc tells the system call it waits in (Linux), None elsewhere."""
    calls = WRITE_SYSCALLS.get(platform.machine())
    try:
        with open(f"/proc/{pid}/syscall") as f:
            fields = f.read().split()
    except OSError:
        return None
    if calls is None or not fields:
        return None
    if fields[0] == "running":
        return False
    try:
        return int(fields[0]) in calls
    except ValueError:
        return None


def scenario_hangup(binary, busy):
    state = "blocked writing a frame" if busy else "idle"
    print(f"scenario: the terminal hangs up mid-game with the UI {state}; it dies by SIGHUP")
    app = App(binary)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        if busy:
            # Start a game and read nothing more: the UI blocks in write(), which fails
            # once the tty is gone, and the main thread restores the terminal itself.
            # The frames must overflow the pty's buffer, which is small on macOS but can
            # hold hundreds of KiB on Linux: on a 250x800 window a game frame is about
            # 180 KB, and each of the 20 resizes after it draws the whole screen again
            # (about 3.6 MB in all), unless the UI is already stuck in a write.
            set_window(app, 250, 800, 0, 0)
            app.idle(0.5)  # read the menu drawn for the new size
            os.write(app.master, b"1")
            for rows, cols in ((240, 790), (250, 800)) * 10:
                time.sleep(0.05)
                set_window(app, rows, cols, 0, 0)
            time.sleep(0.3)
            blocked = blocked_in_write(app.pid)
            if blocked is None:
                # macOS: its pty buffer holds far less than one game frame.
                print("  NOTE  the blocked write is not checked here (no /proc)")
            else:
                check(blocked, "the UI is blocked writing a frame before the hangup")
        else:
            app.send(b"1")
            check(app.wait_screen("White to move"), "game started")
        if busy:
            # The failed write ends the UI, which waits up to 0.1 s for the SIGHUP.
            hang_up(app, 1.0, "at once: the failed write meets the SIGHUP")
        else:
            # crossterm's reader keeps reading the dead tty and never hands the loop back,
            # so the restore on the signal thread ends it after the stuck-UI grace (1 s).
            hang_up(app, 3.0, "within about a second")
    finally:
        app.close()


def scenario_engine_panic(binary):
    print("scenario: RCHESS_FAULT=engine-panic, the worker plays the local search move")
    app = App(binary, env={"RCHESS_FAULT": "engine-panic"})
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        before = len(app.stream)
        app.send(b"3", settle=0)
        check(app.wait_screen("Black to move", timeout=10.0), "the computer's move is played anyway")
        text = app.screen().text()
        check("engine error — local search" in text, "the screen says engine error — local search")
        check(b"panicked" not in app.stream, "the engine panic prints nothing over the UI")
        check(ordered(app.stream, TEARDOWN[:1], before) is None, "the terminal is left alone")
        app.screen().show("after an engine panic")
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after engine panic")
    finally:
        app.close()


def scenario_ui_panic(binary):
    print("scenario: RCHESS_FAULT=ui-panic, the panic hook restores the terminal first")
    app = App(binary, env={"RCHESS_FAULT": "ui-panic"})
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        panic_at = len(app.stream)
        app.send(b"1", settle=0)
        check(app.wait_exit(), "process exits after the panic")
        check(app.status == 101, "exit status 101 (panic)", f"status {app.status}")
        offsets = ordered(app.stream, TEARDOWN, panic_at)
        check(offsets is not None, "panic: teardown order ?1006l ?1002l ?1000l ?2004l then ?1049l", f"offsets {offsets}")
        if offsets is not None:
            tail = app.stream[offsets[-1]:]
            check(b"injected panic on the UI thread" in tail, "the panic message comes after the restore")
        after = termios.tcgetattr(app.master)
        check(after[:4] == app.termios_before[:4], "panic: termios flags restored exactly")
    finally:
        app.close()


def scenario_ui_hang(binary):
    print("scenario: RCHESS_FAULT=ui-hang, SIGTERM still restores the terminal")
    app = App(binary, env={"RCHESS_FAULT": "ui-hang"})
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"1", settle=0.3)
        check(app.status is None and os.waitpid(app.pid, os.WNOHANG)[0] == 0, "the UI thread is hung, not dead")
        signal_at = len(app.stream)
        sent = time.monotonic()
        os.kill(app.pid, signal.SIGTERM)
        exited = app.wait_exit(timeout=5.0)
        check(exited, "a hung UI still exits after SIGTERM")
        if exited:
            took = app.exited_at - sent
            check(app.status == -signal.SIGTERM, "terminated by SIGTERM", f"status {app.status}, {took:.2f}s")
            check(took < 3.0, "within the grace period plus the restore deadline", f"{took:.2f}s")
        check_teardown(app, signal_at, "hung UI")
    finally:
        app.close()


def scenario_control_sigkill(binary):
    print("control: SIGKILL cannot be caught, so the checks above must see a broken pty")
    app = App(binary)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        killed_at = len(app.stream)
        os.kill(app.pid, signal.SIGKILL)
        check(app.wait_exit(), "process is gone")
        check(ordered(app.stream, TEARDOWN, killed_at) is None, "no teardown sequences were written")
        lflag = termios.tcgetattr(app.master)[3]
        check(
            not lflag & termios.ICANON and not lflag & termios.ECHO,
            "the pty is left raw, so the termios check can detect a missed restore",
        )
    finally:
        app.close()


def scenario_arguments(binary):
    print("scenario: --glyphs=ascii plus an unknown argument, Ctrl+C quit")
    app = App(binary, args=["--glyphs=ascii", "--frobnicate"])
    try:
        check_setup(app)
        check(app.wait_screen('ignored unknown argument "--frobnicate"'), "unknown argument is a menu warning")
        app.send(b"1")
        check(app.wait_screen("White to move"), "game started")
        lines = app.screen().lines()
        check(any(re.search(r"\br +n +b +q +k +b +n +r\b", l) for l in lines), "ascii glyphs on the board")
        app.screen().show("ascii board")
        app.send(b"/e4\r")
        check(app.wait_screen("1. e4"), "a move is played, so the game is in progress")
        app.send(b"\x03")
        check(app.wait_screen("Quit the game in progress?"), "Ctrl+C asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"\x03", settle=0)
        check(app.wait_exit(), "a second Ctrl+C quits")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "ctrl+c")
    finally:
        app.close()


def scenario_debug(binary, via_env):
    how = "RCHESS_DEBUG=1" if via_env else "--debug"
    print(f"scenario: {how} against the local search: DEBUG tag, an empty exchange view, no log")
    state = tempfile.TemporaryDirectory(prefix="rchess-pty-debug-")
    log = os.path.join(state.name, "logs", "jev.jsonl")
    env = {"RCHESS_DEBUG_LOG": log}
    if via_env:
        env["RCHESS_DEBUG"] = "1"
    app = App(binary, args=[] if via_env else ["--debug"], env=env)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"3", settle=0)
        check(app.wait_screen("Black to move", timeout=10.0), "the computer played White's first move")
        check("DEBUG" in app.screen().text(), "the Status border says DEBUG")
        app.send(b"d")
        check(app.wait_screen("no Jev requests yet"), "d opens the exchange view, which has nothing yet")
        app.screen().show("exchange view")
        app.send(b"\x1b", settle=0)
        check(
            app.wait_screen_without("no Jev requests yet") and "Black to move" in app.screen().text(),
            "Esc goes back to the board",
        )
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, f"quit in {how}")
        check(
            not os.path.exists(os.path.dirname(log)),
            "no Jev request, so no debug log (not even its folder)",
        )
        check(
            not os.path.exists(os.path.join(HOME.name, ".local")),
            "nothing under ~/.local/state either",
        )
    finally:
        app.close()
        state.cleanup()


def wait_for_query(app, timeout=5.0):
    """Waits for the whole graphics query; the time it arrived, or None. `app.query_lag`
    is how much earlier it may have been written: it came after the harness last read
    the pty without finding it."""
    end = time.monotonic() + timeout
    looked = app.started_at
    while QUERY_END not in app.stream:
        if time.monotonic() >= end:
            return None
        app.pump(0.02)
        if QUERY_END not in app.stream:
            looked = time.monotonic()
    arrived = time.monotonic()
    app.query_lag = arrived - looked
    app.wait_bytes(QUERY_TAIL, timeout=0.5)
    return arrived


def timing_holds(late, limit, what):
    """Whether a check that depends on when the harness acted can still tell anything:
    the harness was `late` seconds behind the moment it meant (it saw the query late, or
    slept too long), and the check allows `limit`. Otherwise the check is skipped with a
    note rather than failed: the harness, not the program, missed the timing."""
    if late < limit:
        return True
    print(f"  NOTE  skipped: {what}; the harness was {late:.2f}s late (allowed {limit:.2f}s)")
    return False


def answer_query(app, answer, attributes=KITTY_ATTRIBUTES_ANSWER):
    """Answers the start-up query as a terminal does: `answer` (up to the status report),
    then the device attributes (`attributes`) if they were asked for after it."""
    tail = attributes if QUERY_TAIL in app.stream else b""
    os.write(app.master, answer + tail)


def check_no_query_text(app, label):
    """The rebuilt screen shows no piece of the query or of an answer as text."""
    text = app.screen().text()
    parts = ("Gi=31", "AAAA", "[16t", "[5n", "i=31;OK", "62;c", "6;18;9t", "[0n")
    shown = [part for part in parts if part in text]
    check(not shown, f"{label}: no query or answer text on screen", f"found {shown}")


def scenario_query_unanswered(binary):
    print("scenario: the graphics query on a pty that never answers; start-up goes on after 1 s")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        lflag = termios.tcgetattr(app.master)[3]
        check(
            not lflag & termios.ECHO and not lflag & termios.ICANON,
            "the query is written in raw mode, so no answer would be echoed",
        )
        check(app.wait_bytes(SETUP[1]), "mouse capture follows the query")
        if asked is not None:
            waited = time.monotonic() - asked
            check(0.9 <= waited <= 2.0, "start-up goes on after about 1 s", f"{waited:.2f}s")
        check_setup(app, query=True)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        check(QUERY_WARNING in app.screen().text(), "the menu warns that the query got no answer")
        check_no_query_text(app, "menu")
        app.screen().show("menu after an unanswered query")
        app.send(b"1")
        check(app.wait_screen("White to move"), "keys typed after start-up work (1 starts a game)")
        app.send(b"g")
        check(app.wait_screen("glyphs: outline"), "the style was Solid (g goes on to Outline)")
        app.send(b"gg")
        check(app.wait_screen("glyphs: image"), "Image is still in the g cycle")
        check(
            "squares too" in app.screen().text(),
            "the status says the squares are too small for pictures",
            app.screen().text().split("Status", 1)[-1][:200],
        )
        text = app.screen().text()
        # 80x24 gives 5x2 squares, too small for half-block pictures (11x5 at least).
        check(
            "♜" in text and "▀" not in text and "▄" not in text,
            "at 80x24 the squares are too small for half-block pictures: solid glyphs",
        )
        app.screen().show("image style on small squares")
        resized_at = len(app.stream)
        set_window(app, 60, 200, 0, 0)
        end = time.monotonic() + 5
        while "▀".encode() not in app.stream[resized_at:] and time.monotonic() < end:
            app.pump(0.05)
        drawn = app.stream[resized_at:]
        check(
            "▀".encode() in drawn or "▄".encode() in drawn,
            "at 200x60 the pieces are drawn with half-blocks",
        )
        set_window(app, ROWS, COLS, 0, 0)
        app.idle(0.3)
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits at once: no move was played")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after an unanswered query")
    finally:
        app.close()


def scenario_query_late_answer(binary):
    print("scenario: the terminal answers the graphics query after the deadline, on the menu")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        check(app.wait_screen(QUERY_WARNING), "the menu is up, with the query warning")
        if asked is not None:
            app.idle(max(0.0, asked + 1.3 - time.monotonic()))
        # Read as keys, the answer's `3` would start Human vs Jev as Black.
        answer_query(app, KITTY_ANSWER)
        app.idle(1.0)
        text = app.screen().text()
        check(
            "1. Human vs Human" in text and "to move" not in text,
            "the late answer does not act as key presses",
        )
        check_no_query_text(app, "menu after the late answer")
        quit_at = len(app.stream)
        sent = time.monotonic()
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits from the menu")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        if app.exited_at is not None:
            check(app.exited_at - sent < 1.0, "at once", f"{app.exited_at - sent:.2f}s")
        check_teardown(app, quit_at, "quit after a late answer")
    finally:
        app.close()


def scenario_query_signal(binary):
    print("scenario: SIGTERM while the graphics query waits; restored at once, dies by SIGTERM")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        asked = asked or time.monotonic()
        app.idle(0.05)
        signal_at = len(app.stream)
        os.kill(app.pid, signal.SIGTERM)
        exited = app.wait_exit(timeout=3.0)
        check(exited, "process exits after SIGTERM")
        if exited:
            check(app.status == -signal.SIGTERM, "terminated by SIGTERM", f"status {app.status}")
            # The query checks for a quit signal every 50 ms instead of waiting out its 1 s,
            # which started before the query was seen here.
            took = app.exited_at - asked
            check(took < 1.0, "before the query's deadline", f"{took:.2f}s after the query")
        check_teardown(app, signal_at, "SIGTERM during the query")
    finally:
        app.close()


def scenario_query_hangup(binary):
    print("scenario: the terminal hangs up while the graphics query waits; it dies by SIGHUP")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        app.idle(0.05)
        # The query checks for a quit signal every 50 ms instead of waiting out its 1 s,
        # which started before the query was seen here.
        hang_up(app, 1.0, "at once, before the query's deadline", since=asked)
    finally:
        app.close()


def scenario_query_kitty(binary):
    print("scenario: the pty answers the graphics query like Kitty; pieces are pictures")
    app = App(binary, env=QUERY_ENV)
    try:
        check(wait_for_query(app) is not None, "the graphics query is written")
        answered_at = len(app.stream)
        answered = time.monotonic()
        answer_query(app, KITTY_ANSWER)
        check(app.wait_bytes(SETUP[1]), "mouse capture follows the answer")
        waited = time.monotonic() - answered
        # Well below the 1 s deadline that an unread answer would leave it waiting for.
        check(waited < 0.8, "start-up goes on as soon as the answer is complete", f"{waited:.2f}s")
        check_setup(app, query=True)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        check(QUERY_WARNING not in app.screen().text(), "no query warning")
        app.send(b"1")
        check(app.wait_screen("White to move"), "Human vs Human starts")
        check(
            b"i=31;OK" not in app.stream and b"6;18;9t" not in app.stream,
            "the answer is not echoed",
        )
        check(b"a=T,U=1" in app.stream[answered_at:], "pieces are sent as kitty pictures")
        check_no_query_text(app, "board")
        board = app.screen().text()
        check(board.count(PLACEHOLDER) > 0, "the board shows the pictures' placeholder cells")
        check("♜" not in board, "no solid glyphs while the pictures are shown")
        app.screen().show("kitty pictures (placeholders show as their character)")
        # 80x24 gives 5x2 squares, image areas of 3x2 cells: 27x36 pixels at 9x18.
        first = set(kitty_sizes(app.stream[answered_at:]))
        check(first == {(27, 36)}, "pictures fit 3x2 cells of 9x18", f"sizes {first}")
        # A font zoom to 8x16: more cells in the same window. Kitty and Ghostty size a
        # placeholder picture from its pixels and the current cell, so a picture made for
        # 9x18 cells would show cropped; the font must be measured again.
        resized_at = len(app.stream)
        set_window(app, 30, 100, 800, 480)
        check(
            app.wait_bytes(FONT_QUERY, start=resized_at),
            "a resize asks for the cell size again with kitty pictures",
        )
        app.idle(0.1)
        check(b"Gi=31" not in app.stream[resized_at:], "without the kitty probe")
        answered_zoom_at = len(app.stream)
        answer_font(app, resized_at, 8, 16, attributes=KITTY_ATTRIBUTES_ANSWER)
        wait_kitty(app, answered_zoom_at)
        check(b"a=T,U=1" in app.stream[resized_at:], "a resize redraws the kitty pictures")
        before = set(KITTY_TRANSMIT.findall(app.stream[:resized_at]))
        after = set(KITTY_TRANSMIT.findall(app.stream[resized_at:]))
        check(
            bool(after) and not (before & after),
            "the redrawn pictures have new ids (the old ones must be deleted too)",
            f"before {sorted(before)}, after {sorted(after)}",
        )
        # 100x30 gives 7x3 squares, image areas of 5x3 cells: 40x48 pixels at 8x16. At the
        # old font they would be 45x54, which the terminal would crop.
        zoomed = set(kitty_sizes(app.stream[answered_zoom_at:]))
        check(
            zoomed == {(40, 48)},
            "the pictures follow the new font (5x3 cells of 8x16)",
            f"sizes {zoomed}",
        )
        check_dropped_kitty_deleted(app, 0, resized_at, answered_zoom_at, "zoom")
        check_no_query_text(app, "board after the zoom")
        second_at = len(app.stream)
        set_window(app, ROWS, COLS, 0, 0)
        check(app.wait_bytes(FONT_QUERY, start=second_at), "a second resize asks again")
        answer_font(app, second_at, 8, 16, attributes=KITTY_ATTRIBUTES_ANSWER)
        wait_kitty(app, second_at)
        kept = set(kitty_sizes(app.stream[second_at:]))
        check(kept == {(24, 32)}, "and the pictures fit 3x2 cells of 8x16", f"sizes {kept}")
        # The same font (8x16) on smaller squares: the frame finds the new size.
        check_dropped_kitty_deleted(
            app, answered_zoom_at, second_at, second_at, "second resize", before_new=False
        )
        app.send(b"g")
        check(app.wait_screen("glyphs: solid"), "the style was Image (g goes on to Solid)")
        check("♜" in app.screen().text(), "then the pieces are solid glyphs")
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits at once: no move was played")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after kitty pictures", kitty=True, live_from=second_at)
    finally:
        app.close()


def scenario_query_kitty_signal(binary):
    print("scenario: SIGTERM after kitty pictures; they are deleted by id before the restore")
    app = App(binary, env=QUERY_ENV)
    try:
        check(wait_for_query(app) is not None, "the graphics query is written")
        answer_query(app, KITTY_ANSWER)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"1")
        check(app.wait_screen("White to move"), "Human vs Human starts")
        check(KITTY_TRANSMIT.search(app.stream) is not None, "pieces are sent as kitty pictures")
        signal_at = len(app.stream)
        os.kill(app.pid, signal.SIGTERM)
        exited = app.wait_exit(timeout=3.0)
        check(exited, "process exits after SIGTERM")
        if exited:
            check(app.status == -signal.SIGTERM, "terminated by SIGTERM", f"status {app.status}")
        check_teardown(app, signal_at, "SIGTERM after kitty pictures", kitty=True)
    finally:
        app.close()


def pending_input(app):
    """What waits in the pty's input queue once the program has ended: what the shell
    would read next. Raw mode first, as a canonical tty holds an unfinished line back."""
    attrs = termios.tcgetattr(app.slave)
    tty.setraw(app.slave, termios.TCSANOW)  # TCSAFLUSH would drop the input
    data = b""
    while select.select([app.slave], [], [], 0.2)[0]:
        chunk = os.read(app.slave, 4096)
        if not chunk:
            break
        data += chunk
    termios.tcsetattr(app.slave, termios.TCSANOW, attrs)
    return data


def sleep_until(moment):
    time.sleep(max(0.0, moment - time.monotonic()))


def scenario_query_split_answer(binary):
    print("scenario: the deadline falls inside the kitty answer; its tail is drained, not typed")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        if asked is not None:
            # The head well before the 1 s deadline; the tail 50 ms after it at the latest
            # (the deadline is at most 1 s after the query was seen), and within the 200 ms
            # the query then keeps reading.
            sleep_until(asked + 0.5)
            os.write(app.master, KITTY_ANSWER[:5])
            sleep_until(asked + 1.05)
            tail_at = time.monotonic()
            answer_query(app, KITTY_ANSWER[5:])
            # The query may have been written up to `query_lag` before it was seen, which
            # moves its deadline and drain earlier; the tail must still fall in the drain.
            late = app.query_lag + (tail_at - (asked + 1.05))
            if not timing_holds(late, 0.15, "the split-answer checks"):
                return
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.idle(0.5)
        text = app.screen().text()
        check(QUERY_WARNING in text, "the query gave up at its deadline (the answer was split)")
        # Typed, the tail `31;OK...` would start Human vs Jev as Black with its `3`.
        check(
            "1. Human vs Human" in text and "to move" not in text,
            "the tail of the answer does not act as key presses",
        )
        check_no_query_text(app, "menu after the split answer")
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits from the menu")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after a split answer")
    finally:
        app.close()


def scenario_query_late_split(binary):
    print("scenario: a late answer in two reads, the status report alone; the UI does not wait for a key")
    app = App(binary, env=QUERY_ENV)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        check(app.wait_screen(QUERY_WARNING), "no answer in time: the menu warns")
        if asked is not None:
            sleep_until(asked + 1.5)
        # Past the query and its drain: crossterm reads the answers. The first read ends
        # in a report it drops (the cell size); the second holds the status report, which
        # it drops too, and, when asked for, the device attributes, which end its read.
        os.write(app.master, b"\x1b[?62;4c\x1b[6;20;10t")
        app.idle(0.2)
        answer_query(app, b"\x1b[0n", ATTRIBUTES_ANSWER)
        app.idle(0.3)
        resized_at = len(app.stream)
        set_window(app, 30, 100, 0, 0)
        end = time.monotonic() + 3.0
        while len(app.stream) == resized_at and time.monotonic() < end:
            app.pump(0.02)
        check(
            len(app.stream) > resized_at,
            "a resize after the late answer is drawn without a key",
        )
        app.idle(0.3)
        text = app.screen().text()
        check("to move" not in text, "the answers do not act as key presses")
        check_no_query_text(app, "menu after the late answer")
        set_window(app, ROWS, COLS, 0, 0)
        app.idle(0.3)
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits from the menu")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after a late answer in two reads")
    finally:
        app.close()


def scenario_query_signal_answers(binary):
    print("scenario: SIGTERM while the graphics query waits, answers right after; none reach the shell")
    # Without a controlling tty, so the pty's input queue can still be read after the exit.
    app = App(binary, env=QUERY_ENV, ctty=False)
    try:
        asked = wait_for_query(app)
        check(asked is not None, "the graphics query is written")
        timed = asked is not None
        asked = asked or time.monotonic()
        app.idle(0.05)
        signal_at = len(app.stream)
        sent = time.monotonic()
        os.kill(app.pid, signal.SIGTERM)
        # The query stops within 50 ms of the signal, then reads for up to 200 ms.
        sleep_until(sent + 0.1)
        answered_at = time.monotonic()
        answer_query(app, KITTY_ANSWER)
        # Meaningful only when the signal came well before the query's 1 s deadline (the
        # query may have been written up to `query_lag` before it was seen) and the
        # answers well inside the 200 ms the query reads after the signal.
        what = "the timing checks of SIGTERM with answers in flight"
        timely = (
            timed
            and timing_holds(sent - (asked - app.query_lag) - 0.05, 0.6, what)
            and timing_holds(answered_at - sent - 0.1, 0.05, what)
        )
        exited = app.wait_exit(timeout=3.0)
        check(exited, "process exits after SIGTERM")
        if exited:
            check(app.status == -signal.SIGTERM, "terminated by SIGTERM", f"status {app.status}")
            took = app.exited_at - asked
            if timely:
                check(took < 1.0, "before the query's deadline", f"{took:.2f}s after the query")
        check_teardown(app, signal_at, "SIGTERM during the query")
        left = pending_input(app)
        if timely:
            check(not left, "the answers were read, so the shell does not get them", f"left {left!r}")
    finally:
        app.close()


def scenario_query_wezterm(binary):
    print("scenario: WezTerm answers the graphics query late; the UI does not wait for a key")
    env = dict(QUERY_ENV, WEZTERM_EXECUTABLE="/usr/bin/wezterm-gui", TERM_PROGRAM="WezTerm")
    app = App(binary, env=env)
    try:
        asked = None
        if app.wait_bytes(WEZTERM_QUERY):
            asked = time.monotonic()
        check(asked is not None, "the query asks for the cell size, the status, then the device attributes")
        check(QUERY_START not in app.stream, "without the kitty probe")
        check(app.wait_screen(QUERY_WARNING), "no answer in time: the menu warns")
        if asked is not None:
            sleep_until(asked + 1.5)
        # Late, after the query stopped reading: the cell size and status reports are
        # dropped by crossterm, and the device attributes after them end its read. A
        # terminal answers only what it was asked.
        asked_attributes = app.stream.find(ATTRIBUTES_REQUEST) >= 0
        os.write(app.master, WEZTERM_ANSWER + (WEZTERM_ATTRIBUTES_ANSWER if asked_attributes else b""))
        app.idle(0.3)
        resized_at = len(app.stream)
        set_window(app, 30, 100, 0, 0)
        end = time.monotonic() + 3.0
        while len(app.stream) == resized_at and time.monotonic() < end:
            app.pump(0.02)
        check(
            len(app.stream) > resized_at,
            "a resize after the late answer is drawn without a key",
        )
        app.idle(0.3)
        check_no_query_text(app, "menu after the late answer")
        set_window(app, ROWS, COLS, 0, 0)
        app.idle(0.3)
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits from the menu")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit in WezTerm")
    finally:
        app.close()


def scenario_query_iterm2(binary):
    print("scenario: iTerm2 (from the environment) answers the sixel probe; pictures are iTerm2's")
    app = App(binary, env=dict(QUERY_ENV, TERM_PROGRAM="iTerm.app"))
    try:
        check(wait_for_query(app) is not None, "the graphics query is written")
        answered_at = len(app.stream)
        answer_query(app, SIXEL_ANSWER, ATTRIBUTES_ANSWER)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        check(QUERY_WARNING not in app.screen().text(), "no query warning")
        app.send(b"1")
        check(app.wait_screen("White to move"), "Human vs Human starts")
        check(app.wait_bytes(ITERM2_PICTURE, start=answered_at), "pieces are iTerm2 pictures")
        app.idle(0.3)
        drawn = app.stream[answered_at:]
        check(not SIXEL_RASTER.search(drawn), "and not Sixel pictures")
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits at once: no move was played")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after iTerm2 pictures")
    finally:
        app.close()


def scenario_closed_without_sighup(binary):
    print("scenario: the terminal closes but no SIGHUP arrives; it still ends by SIGHUP")
    app = App(binary, ctty=False)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        app.send(b"1")
        check(app.wait_screen("White to move"), "game started")
        # The pty is not the program's controlling tty, so closing it sends no SIGHUP:
        # stdin only reports the hangup (and reads end of file or fail with EIO), and the
        # program raises SIGHUP itself. About 1 s: the hangup is seen within 0.3 s, then
        # the stuck UI gets 1 s.
        hang_up(app, 3.0, "within about a second")
    finally:
        app.close()


def set_window(app, rows, cols, width_px, height_px):
    """Gives the pty a new size in cells and pixels and tells the child, as a
    terminal window does when it is resized or its font zoomed."""
    fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, width_px, height_px))
    os.kill(app.pid, signal.SIGWINCH)


def sixel_sizes(stream):
    """The pixel size of every Sixel picture in `stream`, in order."""
    return [(int(w), int(h)) for w, h in SIXEL_RASTER.findall(stream)]


def wait_sixels(app, start, count, timeout=5.0):
    """Waits for `count` Sixel pictures after `start` in the stream; their sizes."""
    end = time.monotonic() + timeout
    while len(sixel_sizes(app.stream[start:])) < count and time.monotonic() < end:
        app.pump(0.05)
    app.idle(0.3)
    return sixel_sizes(app.stream[start:])


def kitty_sizes(stream):
    """The pixel size of every kitty picture transmitted in `stream`, in order."""
    return [(int(w), int(h)) for w, h in KITTY_TRANSMIT_SIZE.findall(stream)]


def wait_kitty(app, start, timeout=5.0):
    """Waits for kitty pictures after `start` in the stream, then for the frame to end;
    their sizes."""
    end = time.monotonic() + timeout
    while not kitty_sizes(app.stream[start:]) and time.monotonic() < end:
        app.pump(0.05)
    app.idle(0.3)
    return kitty_sizes(app.stream[start:])


def answer_font(app, asked_at, width, height, upto=None, attributes=ATTRIBUTES_ANSWER):
    """Answers the font measurement written after `asked_at` (and before `upto`) as a
    terminal does: a cell of `width` x `height` pixels and the status report, then the
    device attributes (`attributes`) if they were asked for too."""
    asked = app.stream[asked_at:upto]
    answer = b"\x1b[6;%d;%dt\x1b[0n" % (height, width)
    if ATTRIBUTES_REQUEST in asked:
        answer += attributes
    os.write(app.master, answer)


def scenario_query_sixel_zoom(binary):
    print("scenario: Sixel pictures follow a font zoom (10x20 to 8x16), measured again")
    app = App(binary, env=QUERY_ENV)
    try:
        fcntl.ioctl(app.slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 800, 480))
        check(wait_for_query(app) is not None, "the graphics query is written")
        answer_query(app, SIXEL_ANSWER, ATTRIBUTES_ANSWER)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        check(QUERY_WARNING not in app.screen().text(), "no query warning")
        app.send(b"1")
        check(app.wait_screen("White to move"), "Human vs Human starts")
        before = wait_sixels(app, 0, 32)
        # 80x24 gives 5x2 squares, image areas of 3x2 cells: 30x40 pixels at 10x20.
        check(len(before) == 32, "every piece is a Sixel picture", f"{len(before)} pictures")
        check(set(before) == {(30, 40)}, "pictures fit 3x2 cells of 10x20", f"sizes {set(before)}")
        zoomed_at = len(app.stream)
        # The zoom: more cells, and a window that reports no pixels, so the size can
        # come only from the answer.
        set_window(app, 30, 100, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, start=zoomed_at), "the resize asks for the cell size again"
        )
        app.idle(0.1)
        asked = app.stream[zoomed_at:]
        check(
            asked.count(b"[16t") == 1
            and asked.count(FULL_FONT_QUERY) == 1
            and asked.count(ATTRIBUTES_REQUEST) == 1
            and b"Gi=31" not in asked,
            "once, and only for the cell size, the status and the device attributes",
        )
        answered_at = len(app.stream)
        answered = time.monotonic()
        os.write(app.master, ZOOMED_ANSWER)
        after = wait_sixels(app, answered_at, 32)
        redrawn = time.monotonic() - answered
        # 100x30 gives 7x3 squares, image areas of 5x3 cells: 40x48 pixels at 8x16. At the
        # old font they would be 50x60, 1.25 columns and 0.75 rows too big.
        check(len(after) == 32, "the zoom redraws every picture", f"{len(after)} pictures")
        check(set(after) == {(40, 48)}, "pictures fit 5x3 cells of 8x16", f"sizes {set(after)}")
        check(redrawn < 2.0, "right after the answer", f"{redrawn:.2f}s")
        check("White to move" in app.screen().text(), "the game is still shown")
        check_no_query_text(app, "board after the zoom")

        # A resize the terminal does not answer keeps the font, after about 1 s. The wait is
        # timed from the resize: the query is written after it, so the measurement cannot give
        # up sooner, and noticing the query late cannot shorten the time measured.
        resized_at = len(app.stream)
        resized = time.monotonic()
        set_window(app, ROWS, COLS, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, timeout=2.0, start=resized_at), "the next resize asks again"
        )
        kept = wait_sixels(app, resized_at, 32)
        waited = time.monotonic() - resized
        check(len(kept) == 32, "the pictures are redrawn", f"{len(kept)} pictures")
        check(
            kept and all(w % 8 == 0 and h % 16 == 0 for w, h in kept),
            "still for 8x16 cells",
            f"sizes {set(kept)}",
        )
        check(waited >= 0.9, "once the query gave up", f"{waited:.2f}s")
        # Its answers come late, as a terminal sends them: in order, so the device
        # attributes come last and make crossterm an event. Alone, the dropped cell size
        # and status reports would leave crossterm's reader waiting for the next input.
        os.write(app.master, ZOOMED_ANSWER)
        app.idle(0.3)
        late_at = len(app.stream)
        set_window(app, 30, 100, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, timeout=3.0, start=late_at),
            "the UI still reacts without a key after the late answers (a resize asks at once)",
        )
        answer_font(app, late_at, 8, 16)
        # This measurement still counts the late answers as owed (crossterm read them), so
        # it waits out the deadline for them before it takes its own.
        check(len(wait_sixels(app, late_at, 32)) == 32, "and redraws the pictures")

        # A slow link: a resize comes after a measurement gave up but before its answers
        # arrive. The next measurement skips those and takes its own answer, and leaves
        # nothing to crossterm that would make it wait for a key.
        gave_up_at = len(app.stream)
        set_window(app, ROWS, COLS, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, timeout=2.0, start=gave_up_at),
            "a resize asks, and gets no answer in time",
        )
        wait_sixels(app, gave_up_at, 32)
        second_at = len(app.stream)
        set_window(app, 26, 90, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, timeout=2.0, start=second_at),
            "a resize right after the one that gave up asks again",
        )
        # The late answers to the first measurement (still 8x16), then 0.2 s later the
        # second one's own answer (a zoom to 11x22).
        answer_font(app, gave_up_at, 8, 16, upto=second_at)
        app.idle(0.2)
        own_at = len(app.stream)
        answer_font(app, second_at, 11, 22)
        fresh = wait_sixels(app, own_at, 32)
        check(
            len(fresh) == 32 and all(w % 11 == 0 and h % 22 == 0 for w, h in fresh),
            "the pictures follow the measurement's own answer (11x22), not the late one",
            f"{len(fresh)} pictures, sizes {set(fresh)}",
        )
        after_at = len(app.stream)
        set_window(app, ROWS, COLS, 0, 0)
        check(
            app.wait_bytes(FONT_QUERY, timeout=2.0, start=after_at),
            "the UI reacts to a resize without a key after the late answers",
        )
        answer_font(app, after_at, 8, 16)
        check(len(wait_sixels(app, after_at, 32)) == 32, "and redraws the pictures")
        app.send(b"g")
        check(app.wait_screen("glyphs: solid"), "keys work after the unanswered query")
        quit_at = len(app.stream)
        app.send(b"q", settle=0)
        check(app.wait_exit(), "q quits at once: no move was played")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit after the zoom")
    finally:
        app.close()


def scenario_help(binary):
    print("scenario: --help prints usage without touching the terminal")
    app = App(binary, args=["--help"])
    try:
        check(app.wait_exit(), "process exits")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check(b"Usage: chess" in app.stream, "usage printed")
        check(b"\x1b[" not in app.stream, "no escape sequences")
        check(termios.tcgetattr(app.master)[:4] == app.termios_before[:4], "termios untouched")
    finally:
        app.close()


def scenario_not_a_terminal(binary):
    print("scenario: stdout is a pipe -> error, terminal untouched")
    app = App(binary, stdout_pipe=True)
    try:
        check(app.wait_exit(), "process exits")
        check(app.status == 1, "exit status 1", f"status {app.status}")
        piped = app.pipe_output()
        check(b"stdout is not a terminal" in app.stream, "error names the problem", app.text().strip()[:100])
        check(b"chess: stdout is not a terminal" in app.stream, "error is printed readably, not as Debug")
        check(b"Custom {" not in app.stream, "no Debug formatting")
        check(b"\x1b[" not in app.stream and b"\x1b[" not in piped, "no escape sequences anywhere")
        check(termios.tcgetattr(app.master)[:4] == app.termios_before[:4], "termios untouched")
    finally:
        app.close()


def scenario_stdin_not_a_terminal(binary):
    print("scenario: stdin is /dev/null -> error before the terminal is touched")
    app = App(binary, stdin_null=True)
    try:
        check(app.wait_exit(), "process exits")
        check(app.status == 1, "exit status 1", f"status {app.status}")
        check(
            b"chess: stdin is not a terminal" in app.stream,
            "error names the problem",
            app.text().strip()[:100],
        )
        check(b"\x1b[" not in app.stream, "no escape sequences (no alternate screen, no menu)")
        check(termios.tcgetattr(app.master)[:4] == app.termios_before[:4], "termios untouched")
    finally:
        app.close()


def target_dir():
    """Where cargo puts its builds, as cargo itself reports it (CARGO_TARGET_DIR and
    build.target-dir included); without cargo, CARGO_TARGET_DIR or target/."""
    try:
        metadata = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=CRATE,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        return json.loads(metadata)["target_directory"]
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError):
        return os.path.join(CRATE, os.environ.get("CARGO_TARGET_DIR") or "target")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--release", action="store_true", help="build and run the release binary")
    parser.add_argument("--no-build", action="store_true", help="run the existing binary as is")
    opts = parser.parse_args()

    profile = "release" if opts.release else "debug"
    if not opts.no_build:
        cmd = ["cargo", "build", "--quiet", "--bin", "chess"] + (["--release"] if opts.release else [])
        subprocess.run(cmd, cwd=CRATE, check=True)
    binary = os.path.join(target_dir(), profile, "chess")
    if not os.access(binary, os.X_OK):
        sys.exit(f"no binary at {binary}; build it first")
    print(f"binary: {binary}")
    print(f"pty: {COLS}x{ROWS}, env without {' / '.join(SECRET_VARS)}; RCHESS_IMAGES=off but for the query")

    scenario_play_and_quit(binary)
    scenario_computer_opens(binary)
    scenario_laya_offline(binary)
    for via_env in (False, True):
        scenario_debug(binary, via_env)
    scenario_query_unanswered(binary)
    scenario_query_late_answer(binary)
    scenario_query_late_split(binary)
    scenario_query_split_answer(binary)
    scenario_query_signal(binary)
    scenario_query_signal_answers(binary)
    scenario_query_hangup(binary)
    scenario_query_wezterm(binary)
    scenario_query_iterm2(binary)
    scenario_query_kitty(binary)
    scenario_query_kitty_signal(binary)
    scenario_query_sixel_zoom(binary)
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        scenario_signal(binary, signum)
    scenario_signal(binary, signal.SIGTERM, repeat=2)
    for busy in (False, True, True, True):
        scenario_hangup(binary, busy)
    scenario_closed_without_sighup(binary)
    if opts.release:
        print("skipping the fault scenarios: RCHESS_FAULT works only in debug builds")
    else:
        scenario_engine_panic(binary)
        scenario_ui_panic(binary)
        scenario_ui_hang(binary)
    scenario_control_sigkill(binary)
    scenario_arguments(binary)
    scenario_help(binary)
    scenario_not_a_terminal(binary)
    scenario_stdin_not_a_terminal(binary)

    HOME.cleanup()
    print()
    if failures:
        print(f"FAILED: {len(failures)} check(s): " + "; ".join(failures))
        sys.exit(1)
    print("ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
