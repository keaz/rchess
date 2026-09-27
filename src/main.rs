use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    // `std::env::args` would panic on an argument that is not valid Unicode.
    // No valid argument needs one, so a lossy copy just ends up as a warning.
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    match chess::tui::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Display, not Debug: "chess: stdout is not a terminal; ...". Not
            // `eprintln!`, which panics when stderr is a tty that has hung up.
            let _ = writeln!(io::stderr(), "{}: {error}", env!("CARGO_BIN_NAME"));
            ExitCode::FAILURE
        }
    }
}
