//! Chess rules core: bitboards, legal move generation, FEN/SAN, and game state.
//!
//! Pure logic only: no I/O and no trait objects. Everything above this layer
//! (engine, TUI) talks to it through `Position`, `Move` and `Game`.

mod bitboard;
mod error;
mod piece;
mod square;

pub use bitboard::Bitboard;
pub use error::ChessError;
pub use piece::{Color, Piece, PieceKind};
pub use square::Square;
