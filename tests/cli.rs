//! The command-line flags that print and exit, run on the real binary: they never touch the
//! terminal, so they need none (stdout and stderr are pipes here).

use std::io;
use std::process::{Command, Output, Stdio};

/// Runs the `chess` binary with `args`, no Jev key and the given stdout.
fn chess(args: &[&str], stdout: Stdio) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chess"))
        .args(args)
        .env_remove("JEV_API_KEY")
        .env_remove("TYPESAFE_API_KEY")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .output()
        .expect("run chess")
}

#[test]
fn version_prints_the_name_and_version() {
    for flag in ["--version", "-V"] {
        let output = chess(&[flag], Stdio::piped());
        assert!(output.status.success(), "{flag}: {:?}", output.status);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("chess {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
        assert!(output.stderr.is_empty(), "{flag}");
    }
    // It wins over the other options, as --help does.
    let output = chess(&["--glyphs", "ascii", "-V", "--bogus"], Stdio::piped());
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("chess "));
}

#[test]
fn help_and_version_into_a_closed_pipe_exit_quietly() {
    // `chess --help | true`: the reader is gone before anything is written.
    for flag in ["--help", "--version"] {
        let (reader, writer) = io::pipe().expect("pipe");
        drop(reader);
        let output = chess(&[flag], Stdio::from(writer));
        assert!(output.status.success(), "{flag}: {:?}", output.status);
        assert_eq!(String::from_utf8_lossy(&output.stderr), "", "{flag}");
    }
}
