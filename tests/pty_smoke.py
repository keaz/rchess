#!/usr/bin/env python3
"""Pseudo-terminal smoke test for the rchess TUI (spec 6.6/6.7, contract A7).

Not run by cargo: `python3 tests/pty_smoke.py [--release] [--no-build]`.

Each scenario starts the binary on a fresh 80x24 pty whose environment has no
JEV_API_KEY or TYPESAFE_API_KEY (so the computer player is local search and
nothing touches the network), drives it with keystrokes or signals, and checks:

* setup writes ?1049h, then ?1000h ?1002h ?1006h (click-and-drag mouse, SGR)
  and ?2004h (bracketed paste), and never ?1003h (any-motion) or ?1015h;
* the menu and the game render (a tiny terminal emulator rebuilds the screen);
* `1`, `/e4<Enter>`, `Esc`, `q`, `y` plays 1. e4 and quits after confirmation,
  and `qh5<Enter>` typed on the board does not quit (the question defaults to No);
* `3` (play Black against the computer) gets a first move from the engine
  thread, and without a key the screen calls the computer "Local search";
* exit writes ?1006l ?1002l ?1000l ?2004l and then ?1049l, then only shows the
  cursor again (?25h) and draws nothing on the main screen, exits with status 0,
  and leaves the pty's termios exactly as it was before the program started
  (ICANON and ECHO back on);
* SIGTERM (and SIGINT, SIGHUP) mid-game restore the terminal the same way and
  the process then dies by that signal; two SIGTERMs in a row do too;
* a real hangup (the terminal closes its end of the pty, as a closed window or a
  dropped SSH session does) ends the process by SIGHUP, both with the UI idle
  (crossterm keeps polling the dead tty, so the stuck-UI watchdog restores) and
  with the UI blocked writing a frame (the write fails and the main thread
  restores): restoring a dead tty must not panic, so there is no SIGABRT from a
  panic inside the panic hook;
* debug-build fault injection (RCHESS_FAULT): an engine that panics still gets a
  local search move played without disturbing the screen; a panic on the UI
  thread restores the terminal before the message is printed (status 101); a
  hung UI thread is still restored and ended by SIGTERM;
* --help and a non-terminal stdout never touch the terminal, and errors are
  printed readably;
* as a control, SIGKILL (which cannot be caught) does leave the pty raw, which
  shows the termios check can fail.

Exits 0 when every check passes, 1 otherwise.
"""

import argparse
import codecs
import fcntl
import os
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unicodedata

ROWS, COLS = 24, 80
CRATE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

SETUP = [b"\x1b[?1049h", b"\x1b[?1000h", b"\x1b[?1002h", b"\x1b[?1006h", b"\x1b[?2004h"]
TEARDOWN = [b"\x1b[?1006l", b"\x1b[?1002l", b"\x1b[?1000l", b"\x1b[?2004l", b"\x1b[?1049l"]
FORBIDDEN = [b"\x1b[?1003h", b"\x1b[?1015h"]
SHOW_CURSOR = b"\x1b[?25h"
SECRET_VARS = ("JEV_API_KEY", "TYPESAFE_API_KEY")

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

    Colours are ignored. Zero-width characters (such as U+FE0E after the pawn) join
    the previous cell. Only the alternate screen is kept.
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
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": HOME.name,
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8",
        "LC_ALL": "en_US.UTF-8",
        "COLORTERM": "truecolor",
    }
    env.update(extra or {})
    assert not any(v in env for v in SECRET_VARS)
    return env


class App:
    """The binary running on its own pty, with everything it wrote recorded."""

    def __init__(self, binary, args=(), stdout_pipe=False, env=None):
        self.master, self.slave = os.openpty()
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
                os.login_tty(self.slave)  # setsid, controlling tty, stdin/stdout/stderr
                if pipe_w is not None:
                    os.dup2(pipe_w, 1)
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

    def wait_bytes(self, needle, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if needle in self.stream:
                return True
            self.pump(0.02)
        return needle in self.stream

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


def check_setup(app):
    check(app.wait_bytes(SETUP[-1]), "setup sequences arrive")
    offsets = ordered(app.stream, SETUP)
    check(
        offsets is not None,
        "setup order: ?1049h, ?1000h, ?1002h, ?1006h, ?2004h",
        f"offsets {offsets}",
    )


def check_teardown(app, after, label):
    offsets = ordered(app.stream, TEARDOWN, after)
    check(
        offsets is not None,
        f"{label}: teardown order ?1006l ?1002l ?1000l ?2004l then ?1049l",
        f"offsets {offsets}",
    )
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
        after_attrs[:4] == app.termios_before[:4],
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
        check("e4" in app.screen().text(), "typed text shows in the command box")
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
        check("Quit the game in progress?" not in app.screen().text(), "Enter answered No")
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
        app.screen().show("computer opened")
        app.send(b"q")
        check(app.wait_screen("Quit the game in progress?"), "q asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"y", settle=0)
        check(app.wait_exit(), "process exits after y")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "quit vs computer")
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


def scenario_hangup(binary, busy):
    state = "blocked writing a frame" if busy else "idle"
    print(f"scenario: the terminal hangs up mid-game with the UI {state}; it dies by SIGHUP")
    app = App(binary)
    try:
        check_setup(app)
        check(app.wait_screen("1. Human vs Human"), "menu renders")
        if busy:
            # Start a game and read nothing more: the first game frame is larger than
            # the pty buffer, so the UI blocks in write(), which fails once the tty is
            # gone and the main thread restores the terminal itself.
            os.write(app.master, b"1")
            time.sleep(0.5)
        else:
            app.send(b"1")
            check(app.wait_screen("White to move"), "game started")
        # What a terminal emulator or sshd does when it goes away: its end of the pty is
        # closed and nobody else holds the slave, so the kernel hangs the tty up (SIGHUP
        # to the session) and every later write to it fails.
        for name in ("slave", "master"):
            os.close(getattr(app, name))
            setattr(app, name, None)
        sent = time.monotonic()
        status = None
        while status is None and time.monotonic() < sent + 5.0:
            pid, raw = os.waitpid(app.pid, os.WNOHANG)
            if pid:
                status = os.waitstatus_to_exitcode(raw)
            else:
                time.sleep(0.01)
        app.status = status
        check(status is not None, "process exits after the hangup")
        if status is not None:
            took = time.monotonic() - sent
            check(
                status != -signal.SIGABRT,
                "no SIGABRT: restoring a dead terminal never panics",
                f"status {status}, {took:.3f}s",
            )
            check(status == -signal.SIGHUP, "terminated by SIGHUP", f"status {status}")
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
        app.send(b"\x03")
        check(app.wait_screen("Quit the game in progress?"), "Ctrl+C asks for confirmation")
        quit_at = len(app.stream)
        app.send(b"\x03", settle=0)
        check(app.wait_exit(), "a second Ctrl+C quits")
        check(app.status == 0, "exit status 0", f"status {app.status}")
        check_teardown(app, quit_at, "ctrl+c")
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


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--release", action="store_true", help="build and run the release binary")
    parser.add_argument("--no-build", action="store_true", help="run the existing binary as is")
    opts = parser.parse_args()

    profile = "release" if opts.release else "debug"
    if not opts.no_build:
        cmd = ["cargo", "build", "--quiet", "--bin", "chess"] + (["--release"] if opts.release else [])
        subprocess.run(cmd, cwd=CRATE, check=True)
    binary = os.path.join(CRATE, "target", profile, "chess")
    if not os.access(binary, os.X_OK):
        sys.exit(f"no binary at {binary}; build it first")
    print(f"binary: {binary}")
    print(f"pty: {COLS}x{ROWS}, env without {' / '.join(SECRET_VARS)}")

    scenario_play_and_quit(binary)
    scenario_computer_opens(binary)
    for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        scenario_signal(binary, signum)
    scenario_signal(binary, signal.SIGTERM, repeat=2)
    for busy in (False, True, True, True):
        scenario_hangup(binary, busy)
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

    HOME.cleanup()
    print()
    if failures:
        print(f"FAILED: {len(failures)} check(s): " + "; ".join(failures))
        sys.exit(1)
    print("ALL CHECKS PASSED")


if __name__ == "__main__":
    main()
