//! Computer player: local search and annotation feed one Jev `choice` question.

mod eval;
mod search;
mod see;

pub use search::{MATE, ScoredMove, analyse};
