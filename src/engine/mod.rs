//! Computer player: local search and annotation feed one Jev `choice` question.

mod annotate;
mod config;
mod describe;
mod eval;
mod jev;
mod search;
mod see;

pub use annotate::{Annotation, Bucket};
pub use config::EngineConfig;
pub use describe::JevState;
pub use jev::{ChoiceAnswer, ChoiceOption, ChoiceRequest, JevClient, JevError, MoveChooser};
pub use search::{MATE, ScoredMove, analyse};
