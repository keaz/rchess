//! Computer player: local search and annotation feed one Jev `choice` question.

mod annotate;
mod config;
mod describe;
mod eval;
mod jev;
mod player;
mod search;
mod see;

pub use annotate::{Annotation, Bucket};
pub use config::{EngineConfig, Provider};
pub use describe::JevState;
/// A really recorded, redacted exchange for the TUI's key tests.
#[cfg(test)]
pub(crate) use jev::tests::recorded_exchange;
pub use jev::{
    ChoiceAnswer, ChoiceOption, ChoiceRequest, JEV_ENDPOINT, JevAttempt, JevClient, JevError,
    JevExchange, MoveChooser, printable,
};
pub use player::{ComputerMove, ComputerPlayer, MoveSource};
pub use search::{MATE, ScoredMove, analyse};
