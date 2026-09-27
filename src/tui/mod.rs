//! Terminal user interface (spec section 6): ratatui widgets, input
//! parsing, file export, the engine worker and terminal setup, built only on
//! `crate::core` and `crate::engine`.
//!
//! [`run`] is the whole program: it reads the command line and the environment,
//! builds the computer player, sets up the terminal and runs the main loop
//! until the user quits or a signal asks it to stop.

pub mod board;
pub mod event;
pub mod files;
pub mod glyphs;
pub mod input;
pub mod movetext;
pub mod terminal;
pub mod worker;

#[cfg(test)]
mod test_support;
