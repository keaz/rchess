//! Computer player: local search and annotation feed one Jev `choice` question.

mod annotate;
mod eval;
mod search;
mod see;

pub use annotate::{Annotation, Bucket};
pub use search::{MATE, ScoredMove, analyse};
