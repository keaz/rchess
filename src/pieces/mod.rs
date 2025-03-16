use std::fmt::{Debug, Display};

use crate::{Position, board::BoardTrait};

use self::pawn::pawn_move_to;

pub mod bishop;
pub mod king;
pub mod knight;
pub mod pawn;
pub mod queen;
pub mod rook;

pub type Value = u8;

pub trait Piece: Debug + CloneAsPiece + 'static {
    fn move_to(
        &mut self,
        position: Position,
        board: &mut dyn BoardTrait,
    ) -> Result<Option<PieceType>, ChessError>;

    fn can_move_to(&self, position: Position, board: &dyn BoardTrait) -> Result<(), ChessError>;

    fn color(&self) -> &Color;

    fn possible_moves(&self, board: &dyn BoardTrait) -> Vec<Position>;
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum PieceType {
    Pawn(Color, Position, bool),
    Rook(Color, Position),
    Bishop(Color, Position),
    Knight(Color, Position),
    Queen(Color, Position),
    King(Color, Position),
}

impl PieceType {
    pub fn value(&self) -> Value {
        match self {
            PieceType::Pawn(_, _, _) => 1,
            PieceType::Rook(_, _) => 5,
            PieceType::Bishop(_, _) => 3,
            PieceType::Knight(_, _) => 3,
            PieceType::Queen(_, _) => 9,
            PieceType::King(_, _) => u8::MAX,
        }
    }

    pub fn position(&self) -> &Position {
        match self {
            PieceType::Pawn(_, position, _) => position,
            PieceType::Rook(_, position) => position,
            PieceType::Bishop(_, position) => position,
            PieceType::Knight(_, position) => position,
            PieceType::Queen(_, position) => position,
            PieceType::King(_, position) => position,
        }
    }
}

impl Piece for PieceType {
    fn move_to(
        &mut self,
        position: Position,
        board: &mut dyn BoardTrait,
    ) -> Result<Option<PieceType>, ChessError> {
        match self {
            PieceType::Pawn(_, _, _) => pawn_move_to(self, position, board),
            PieceType::Rook(_, _) => rook::move_to(self, position, board),
            PieceType::Bishop(_, _) => bishop::move_to(self, position, board),
            PieceType::Knight(_, _) => knight::move_to(self, position, board),
            PieceType::Queen(_, _) => queen::move_to(self, position, board),
            PieceType::King(_, _) => king::move_to(self, position, board),
        }
    }

    fn can_move_to(&self, position: Position, board: &dyn BoardTrait) -> Result<(), ChessError> {
        match self {
            PieceType::Pawn(color, current_position, is_first_move) => {
                pawn::can_move_to(current_position, color, *is_first_move, position, board)
            }
            PieceType::Rook(color, current_position) => {
                rook::can_move_to(current_position, color, position, board)
            }
            PieceType::Bishop(color, current_position) => {
                bishop::can_move_to(current_position, color, position, board)
            }
            PieceType::Knight(color, current_position) => {
                knight::can_move_to(current_position, color, position, board)
            }
            PieceType::Queen(color, current_position) => {
                queen::can_move_to(current_position, color, position, board)
            }
            PieceType::King(color, current_position) => {
                king::can_move_to(current_position, color, position, board)
            }
        }
    }

    fn color(&self) -> &Color {
        match self {
            PieceType::Pawn(color, _, _) => color,
            PieceType::Rook(color, _) => color,
            PieceType::Bishop(color, _) => color,
            PieceType::Knight(color, _) => color,
            PieceType::Queen(color, _) => color,
            PieceType::King(color, _) => color,
        }
    }

    fn possible_moves(&self, board: &dyn BoardTrait) -> Vec<Position> {
        match self {
            PieceType::Pawn(color, position, is_first_move) => {
                pawn::possible_moves(position, color, *is_first_move, board)
            }
            PieceType::Rook(color, position) => rook::possible_moves(position, color, board),
            PieceType::Bishop(color, position) => bishop::possible_moves(position, color, board),
            PieceType::Knight(color, position) => knight::possible_moves(position, color, board),
            PieceType::Queen(color, position) => queen::possible_moves(position, color, board),
            PieceType::King(color, position) => king::possible_moves(position, color, board),
        }
    }
}

impl Display for PieceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PieceType::Pawn(color, _, _) => write!(f, "{} Pawn", color),
            PieceType::Rook(color, _) => write!(f, "{} Rook", color),
            PieceType::Bishop(color, _) => write!(f, "{} Bishop", color),
            PieceType::Knight(color, _) => write!(f, "{} Knight", color),
            PieceType::Queen(color, _) => write!(f, "{} Queen", color),
            PieceType::King(color, _) => write!(f, "{} King", color),
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
