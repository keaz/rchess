use std::fmt::{Debug, Display};

use crate::{Board, Position};

use self::pawn::pawn_move_to;

pub mod bishop;
pub mod king;
pub mod knight;
pub mod pawn;
pub mod queen;
pub mod rook;

pub trait Piece: Debug + CloneAsPiece + 'static {
    fn move_to(
        &mut self,
        position: Position,
        board: Board,
    ) -> Result<(Board, Option<PieceType>), ChessError>;

    fn can_move_to(&self, position: Position, board: &Board) -> Result<(), ChessError>;

    fn color(&self) -> &Color;
}

pub type Value = u8;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum PieceType {
    Pawn(Color, Position, Value, bool),
    Rook(Color, Position, Value),
    Bishop(Color, Position, Value),
    Knight(Color, Position, Value),
    Queen(Color, Position, Value),
    King(Color, Position, Value),
}

impl Piece for PieceType {
    fn move_to(
        &mut self,
        position: Position,
        board: Board,
    ) -> Result<(Board, Option<PieceType>), ChessError> {
        match self {
            PieceType::Pawn(_, _, _, _) => pawn_move_to(self, position, board),
            PieceType::Rook(_, _, _) => rook::move_to(self, position, board),
            PieceType::Bishop(_, _, _) => bishop::move_to(self, position, board),
            PieceType::Knight(_, _, _) => knight::move_to(self, position, board),
            PieceType::Queen(_, _, _) => queen::move_to(self, position, board),
            PieceType::King(_, _, _) => king::move_to(self, position, board),
        }
    }

    fn can_move_to(&self, position: Position, board: &Board) -> Result<(), ChessError> {
        match self {
            PieceType::Pawn(color, current_position, _, is_first_move) => {
                pawn::can_move_to(current_position, color, is_first_move, position, board)
            }
            PieceType::Rook(color, current_position, _) => {
                rook::can_move_to(current_position, color, position, board)
            }
            PieceType::Bishop(color, current_position, _) => {
                bishop::can_move_to(current_position, color, position, board)
            }
            PieceType::Knight(color, current_position, _) => {
                knight::can_move_to(current_position, color, position, board)
            }
            PieceType::Queen(color, current_position, _) => {
                queen::can_move_to(current_position, color, position, board)
            }
            PieceType::King(color, current_position, _) => {
                king::can_move_to(current_position, color, position, board)
            }
        }
    }

    fn color(&self) -> &Color {
        match self {
            PieceType::Pawn(color, _, _, _) => color,
            PieceType::Rook(color, _, _) => color,
            PieceType::Bishop(color, _, _) => color,
            PieceType::Knight(color, _, _) => color,
            PieceType::Queen(color, _, _) => color,
            PieceType::King(color, _, _) => color,
        }
    }
}

impl Display for PieceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PieceType::Pawn(color, _, _, _) => write!(f, "{} Pawn", color),
            PieceType::Rook(color, _, _) => write!(f, "{} Rook", color),
            PieceType::Bishop(color, _, _) => write!(f, "{} Bishop", color),
            PieceType::Knight(color, _, _) => write!(f, "{} Knight", color),
            PieceType::Queen(color, _, _) => write!(f, "{} Queen", color),
            PieceType::King(color, _, _) => write!(f, "{} King", color),
        }
    }
}

impl PieceType {
    pub fn value(&self) -> u8 {
        match self {
            PieceType::Pawn(_, _, value, _) => *value,
            PieceType::Rook(_, _, value) => *value,
            PieceType::Bishop(_, _, value) => *value,
            PieceType::Knight(_, _, value) => *value,
            PieceType::Queen(_, _, value) => *value,
            PieceType::King(_, _, value) => *value,
        }
    }
}

pub trait CloneAsPiece {
    fn clone_as_a(&self) -> Box<dyn Piece>;
}

impl<T: Piece + Clone> CloneAsPiece for T {
    fn clone_as_a(&self) -> Box<dyn Piece> {
        Box::new(self.clone())
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum Color {
    Black,
    White,
}

impl Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Color::Black => write!(f, "Black"),
            Color::White => write!(f, "White"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ChessError {
    InvalidMove,
    InvalidPiece,
    BlockedMove,
    InvalidCapture,
    UnSafeKing,
    NoPiece,
}

impl<'a> PartialEq<Color> for &'a Color {
    fn eq(&self, other: &Color) -> bool {
        match self {
            Color::Black => match other {
                Color::Black => true,
                _ => false,
            },
            Color::White => match other {
                Color::White => true,
                _ => false,
            },
        }
    }
}

impl Debug for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Color::Black => write!(f, "Black"),
            Color::White => write!(f, "White"),
        }
    }
}
