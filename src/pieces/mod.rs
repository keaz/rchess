use std::{fmt::Debug, sync::Arc, rc::Rc, cell::RefCell, borrow::BorrowMut};

use log::{debug, info};

use crate::{Board, Position};

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
pub struct Pawn {
    pub color: Color,
    pub position: Position,
    pub is_first_move: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Rook {
    pub color: Color,
    pub position: Position,
}

#[derive(Clone, Copy, Debug)]
pub struct Bishop {
    pub color: Color,
    pub position: Position,
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

impl Pawn {
    pub fn new(color: Color, position: Position) -> Self {
        Pawn {
            color,
            position,
            is_first_move: true,
        }
    }
}

impl Piece for Pawn {

    fn move_to(&self, position: Position, mut board: Board) -> Result<Board,ChessError> {
        if position.to_index() <= self.position.to_index() {
            return Err(ChessError::InvalidMove);
        }

        let new_index = position.to_index();
        let old_index = self.position.to_index();
        let jump: usize = new_index - old_index;
        info!("Pawn jump is {}", jump);
        if jump <= 0 {
            debug!("Invalid move jump is {}", jump );
            return Err(ChessError::InvalidMove);
        }
        if jump != 8 && jump != 16 && jump != 7 && jump != 9 {
            return Err(ChessError::InvalidMove);
        }

        if !self.is_first_move && jump == 16 {
            return Err(ChessError::InvalidMove);
        }
        
        // let cloned_board = board.clone();
        let square =  &board.squares[new_index];
        if (jump == 8 || jump == 16) && square.piece.is_some(){
            return Err(ChessError::InvalidMove);
        }

        if (jump == 7 || jump == 9) && square.piece.is_none(){
            return Err(ChessError::InvalidMove);
        }
        
        board.squares[self.position.to_index()].piece = None;
        println!("{:?} Pawn moved to {:?}", self.color, position);
        board.borrow_mut().squares[new_index].piece = Some(Box::new(Pawn {
            color: self.color,
            position,
            is_first_move: false,
        }));

        
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}


impl Rook {
    pub fn new(color: Color, position: Position) -> Self {
        Rook {
            color,
            position,
        }
    }
}

impl Piece for Rook {

    fn move_to(&self, position: Position, mut board: Board) -> Result<Board,ChessError> {
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump % 8 != 0 && jump / 8 != 0 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.borrow_mut().squares[new_index];

        println!("{:?} Rook move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

impl Bishop {
    pub fn new(color: Color, position: Position) -> Self {
        Bishop {
            color,
            position,
        }
    }
    
}

impl Piece for Bishop {

    fn move_to(&self, position: Position,  mut board: Board) -> Result<Board,ChessError> {
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump % 7 != 0 && jump % 9 != 0 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.borrow_mut().squares[new_index];

        println!("{:?} Bishop move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
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

        let square = &mut board.borrow_mut().squares[new_index];

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

        let square = &mut board.borrow_mut().squares[new_index];

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

        let square = &mut board.borrow_mut().squares[new_index];

        println!("{:?} King move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

#[cfg(test)]
mod test {
    use std::{sync::Arc, rc::Rc, cell::RefCell};

    use crate::{Board, Position, pieces::ChessError};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_pawn_first_two_moves() {
        init();
        let board = Board::new();
        let pawn = board.get_piece(Position::new('a', 2)).unwrap().as_ref();
        
        let new_board = pawn.move_to(Position::new('a', 4), board.clone());
        assert!(new_board.is_ok());

        let board = new_board.unwrap();
        let pawn = board.get_piece(Position::new('a', 4)).unwrap().as_ref();
        let new_board = pawn.move_to(Position::new('a', 6), board.clone()).err().unwrap();
        assert_eq!(new_board, ChessError::InvalidMove);
    }

    #[test]
    fn test_pawn_first_one_move() {
        init();
        let board = Board::new();
        let pawn = board.get_piece(Position::new('a', 2)).unwrap().as_ref();
        
        let new_board = pawn.move_to(Position::new('a', 3), board.clone());
        assert!(new_board.is_ok());
    }

    #[test]
    fn test_pawn_invalid_moves() {
        init();
        let board = Board::new();
        let pawn = board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        
        let new_board = pawn.move_to(Position::new('b', 4), board.clone()).unwrap();
        let pawn = new_board.get_piece(Position::new('b', 4)).unwrap().as_ref();

        let same_position = pawn.move_to(Position::new('b', 4), new_board.clone()).err().unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn.move_to(Position::new('c', 4), new_board.clone()).err().unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn.move_to(Position::new('a', 4), new_board.clone()).err().unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn.move_to(Position::new('b', 3), new_board.clone()).err().unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

    }



}