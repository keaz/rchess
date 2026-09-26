//! Chess rules core: bitboards, legal move generation, FEN/SAN, and game state.
//!
//! Pure logic only: no I/O and no trait objects. Everything above this layer
//! (engine, TUI) talks to it through `Position`, `Move` and `Game`.

mod attacks;
mod bitboard;
mod error;
mod mv;
mod piece;
mod position;
mod square;
mod zobrist;

pub use attacks::{
    between, bishop_attacks, king_attacks, knight_attacks, line, pawn_attacks, queen_attacks,
    rook_attacks,
};
pub use bitboard::Bitboard;
pub use error::ChessError;
pub use mv::Move;
pub use piece::{Color, Piece, PieceKind};
pub use position::{CastleRights, Position, START_FEN};
pub use square::Square;
