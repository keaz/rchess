// `pub mod core` shadows the built-in `core` crate at the crate root: code in
// this file that needs the standard library's `core` must write `::core::`.
pub mod core;
pub mod engine;
pub mod tui;
