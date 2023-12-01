use std::{fmt::Debug, borrow::BorrowMut};

use crate::{Board, Position};

pub mod pawn;
pub mod rook;
pub mod bishop;

pub trait Piece : Debug + CloneAsPiece + 'static {

    fn move_to(&self, position: Position,  board: Board) -> Result<Board,ChessError>;

    fn color(&self) -> &Color;

    
}


pub trait CloneAsPiece {
    fn clone_as_a(&self) -> Box<dyn Piece>;
}

impl<T: Piece + Clone> CloneAsPiece for T {
    fn clone_as_a(&self) -> Box<dyn Piece> {
        Box::new(self.clone())
    }
}

#[derive(Copy,Clone)]
pub enum Color {
    Black,
    White,
}

#[derive(Debug,PartialEq)]
pub enum ChessError {
    InvalidMove,
    BlockedMove,
    InvalidCapture,
    NoPiece
}

impl <'a> PartialEq<Color> for &'a Color {
    fn eq(&self, other: &Color) -> bool {
        match self {
            Color::Black => {
                match other {
                    Color::Black => true,
                    _ => false,
                }
            },
            Color::White => {
                match other {
                    Color::White => true,
                    _ => false,
                }
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

#[derive(Clone, Copy, Debug)]
pub struct Queen {
    pub color: Color,
    pub position: Position,
}

#[derive(Clone, Copy, Debug)]
pub struct King {
    pub color: Color,
    pub position: Position,
}

#[derive(Clone, Copy, Debug)]
pub struct Knight {
    pub color: Color,
    pub position: Position,
}


impl Knight {
    pub fn new(color: Color, position: Position) -> Self {
        Knight {
            color,
            position,
        }
    }
    
}

impl Piece for Knight {

    fn move_to(&self, position: Position,  mut board: Board) -> Result<Board,ChessError>{
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump != 6 && jump != 10 && jump != 15 && jump != 17 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.borrow_mut().squares[new_index as usize];

        println!("{:?} Knight move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

impl Queen {
    pub fn new(color: Color, position: Position) -> Self {
        Queen {
            color,
            position,
        }
    }
    
}

impl Piece for Queen {

    fn move_to(&self, position: Position,  mut board: Board) -> Result<Board,ChessError> {
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump % 7 != 0 && jump % 9 != 0 && jump % 8 != 0 && jump / 8 != 0 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.borrow_mut().squares[new_index as usize];

        println!("{:?} Queen move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

impl King {
    pub fn new(color: Color, position: Position) -> Self {
        King {
            color,
            position,
        }
    }
}

impl Piece for King {

    fn move_to(&self, position: Position,  mut board: Board) -> Result<Board,ChessError> {
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump % 7 != 0 && jump % 9 != 0 && jump % 8 != 0 && jump / 8 != 0 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.borrow_mut().squares[new_index as usize];

        println!("{:?} King move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

