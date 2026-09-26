use thiserror::Error;

/// Every way external input (FEN strings, move text, move requests) can be rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChessError {
    #[error("invalid FEN: {0}")]
    InvalidFen(String),
    #[error("illegal move: {0}")]
    IllegalMove(String),
    #[error("cannot parse move: {0}")]
    ParseMove(String),
}
