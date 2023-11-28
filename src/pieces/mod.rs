use std::{fmt::Debug, borrow::BorrowMut};

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
        
        let new_index = position.to_index();
        let old_index = self.position.to_index();
        let jump = new_index - old_index;
        info!("Pawn jump is {}", jump);
        match self.color {
            Color::Black => {
                if jump > 0 {
                    return Err(ChessError::InvalidMove);
                }
            },
            Color::White => {
                if jump < 0 {
                    return Err(ChessError::InvalidMove);
                }
            },
        }
        let jump = jump.abs();
        if jump != 8 && jump != 16 && jump != 7 && jump != 9 {
            return Err(ChessError::InvalidMove);
        }

        if !self.is_first_move && jump == 16 {
            return Err(ChessError::InvalidMove);
        }
        
        let square =  &board.squares[new_index as usize];
        if (jump == 8 || jump == 16) && square.piece.is_some(){
            return Err(ChessError::InvalidMove);
        }

        if (jump == 7 || jump == 9) && square.piece.is_none(){
            return Err(ChessError::InvalidMove);
        }
        
       
        println!("{:?} Pawn moved to {:?}", self.color, position);
        let other_piece = &square.piece;
        if let Some(other_piece) = other_piece {
            if other_piece.color() == self.color {
                return Err(ChessError::InvalidMove);
            }
        }
        board.squares[self.position.to_index() as usize].piece = None;
        board.borrow_mut().squares[new_index as usize].piece = Some(Box::new(Pawn {
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

        if jump % 8 == 0 {
            let mut index = old_index;
            if new_index > old_index {
                while index != new_index {
                    index += 8;
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::InvalidMove);
                    }
                }
            } else {
                while index != new_index {
                    index -= 8;
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::InvalidMove);
                    }
                }
            }
            
        } else {
            let mut index = old_index;
            if new_index > old_index {
                while index != new_index {
                    index += 1;
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::InvalidMove);
                    }
                }
            } else {
                while index != new_index {
                    index -= 1;
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::InvalidMove);
                    }
                }
            }
            
        }

        let square = &mut board.borrow_mut().squares[new_index as usize];
        if square.piece.is_some() {
            if square.piece.as_ref().unwrap().color() == self.color {
                return Err(ChessError::InvalidMove);
            }
        }

        board.squares[self.position.to_index() as usize].piece = None;
        board.borrow_mut().squares[new_index as usize].piece = Some(Box::new(Rook {
            color: self.color,
            position,
        }));

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

        let square = &mut board.borrow_mut().squares[new_index as usize];

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

#[cfg(test)]
mod test {
    use crate::{Board, Position, pieces::{ChessError, Color}};

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
    fn test_white_pawn_invalid_moves() {
        init();
        let board = Board::new();
        let pawn = board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        
        let new_board = pawn.move_to(Position::new('b', 4), board.clone()).unwrap();
        let black_pawn = new_board.get_piece(Position::new('b', 7)).unwrap().as_ref();
        let new_board = black_pawn.move_to(Position::new('b', 5), new_board.clone()).unwrap();
        let pawn = new_board.get_piece(Position::new('b', 4)).unwrap().as_ref();

        let same_position = pawn.move_to(Position::new('b', 4), new_board.clone()).err().unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn.move_to(Position::new('c', 4), new_board.clone()).err().unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn.move_to(Position::new('a', 4), new_board.clone()).err().unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn.move_to(Position::new('b', 3), new_board.clone()).err().unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

        let wrong_move_two_squares = pawn.move_to(Position::new('b', 5), new_board.clone()).err().unwrap();
        assert_eq!(wrong_move_two_squares, ChessError::InvalidMove,"Pawn can't move to a already occupied square");
    }


    #[test]
    fn test_black_pawn_invalid_moves() {
        init();
        let board = Board::new();
        let pawn = board.get_piece(Position::new('c', 7)).unwrap().as_ref();
        
        let new_board = pawn.move_to(Position::new('c', 5), board.clone()).unwrap();
        let pawn = new_board.get_piece(Position::new('c', 5)).unwrap().as_ref();

        let same_position = pawn.move_to(Position::new('c', 5), new_board.clone()).err().unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn.move_to(Position::new('d', 5), new_board.clone()).err().unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn.move_to(Position::new('b', 5), new_board.clone()).err().unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn.move_to(Position::new('c', 6), new_board.clone()).err().unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

    }

    #[test]
    fn test_white_pawn_no_to_piece_capture() {
        init();
        let board = Board::new();
        let white_pawn = board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        
        let new_board = white_pawn.move_to(Position::new('c', 3), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on right side of the white pawn");
        
        let new_board = white_pawn.move_to(Position::new('a', 3), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on left side of the white pawn");

    }

    #[test]
    fn test_black_pawn_no_to_piece_capture() {
        init();
        let board = Board::new();
        let black_pawn = board.get_piece(Position::new('b', 7)).unwrap().as_ref();
        
        let new_board = black_pawn.move_to(Position::new('c', 6), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on right side of the black pawn");

        let new_board = black_pawn.move_to(Position::new('a', 6), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on left side of the black pawn");
    }

    #[test]
    fn test_white_pawn_invalid_capture() {
        init();
        let board = Board::new();
        let white_pawn = board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        
        let other_white_pawn = board.get_piece(Position::new('c', 2)).unwrap().as_ref();
        let new_board = other_white_pawn.move_to(Position::new('c', 3), board.clone()).unwrap();

        let failed_board = white_pawn.move_to(Position::new('c', 3), new_board.clone());
        assert_eq!(failed_board.err().unwrap(), ChessError::InvalidMove,"Capturing same color piece");
    }

    #[test]
    fn test_black_pawn_invalid_capture() {
        init();
        let board = Board::new();
        let black_pawn = board.get_piece(Position::new('b', 7)).unwrap().as_ref();
        
        let other_black_pawn = board.get_piece(Position::new('c', 7)).unwrap().as_ref();
        let new_board = other_black_pawn.move_to(Position::new('c', 6), board.clone()).unwrap();

        let failed_board = black_pawn.move_to(Position::new('c', 6), new_board.clone());
        assert_eq!(failed_board.err().unwrap(), ChessError::InvalidMove,"Capturing same color piece");
    }


    #[test]
    fn test_white_pawn_capture() {
        init();
        let board = Board::new();
        let black_pawn = board.get_piece(Position::new('c', 7)).unwrap().as_ref();
        let new_board = black_pawn.move_to(Position::new('c', 5), board.clone()).unwrap();
        
        let white_pawn = new_board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        let new_board = white_pawn.move_to(Position::new('b', 4), new_board.clone()).unwrap();

        let white_pawn = new_board.get_piece(Position::new('b', 4)).unwrap().as_ref();
        let new_board = white_pawn.move_to(Position::new('c', 5), new_board.clone());
        assert!(new_board.is_ok(), "White pawn should capture black pawn");

        let new_board = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('c', 5));
        assert_eq!(white_pawn.unwrap().color(),Color::White, "White pawn should be in c5");
    }

    #[test]
    fn test_black_pawn_capture() {
        init();
        let board = Board::new();
        let black_pawn = board.get_piece(Position::new('c', 7)).unwrap().as_ref();
        let new_board = black_pawn.move_to(Position::new('c', 5), board.clone()).unwrap();
        
        let white_pawn = new_board.get_piece(Position::new('b', 2)).unwrap().as_ref();
        let new_board = white_pawn.move_to(Position::new('b', 4), new_board.clone()).unwrap();

        let black_pawn = new_board.get_piece(Position::new('c', 5)).unwrap().as_ref();
        let new_board = black_pawn.move_to(Position::new('b', 4), new_board.clone());
        assert!(new_board.is_ok(), "Black pawn should capture black pawn");

        let new_board = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('b', 4));
        assert_eq!(white_pawn.unwrap().color(),Color::Black, "Black pawn should be in b4");
    }

    #[test]
    fn test_white_rook_invalid_initial_move() {
        init();
        let board = Board::new();
        let left_rook = board.get_piece(Position::new('a', 1)).unwrap().as_ref();
        let new_board = left_rook.move_to(Position::new('a', 2), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White left rook can't move to a2");

        let new_board = left_rook.move_to(Position::new('b', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White left Rook can't move to b1");
       
        let right_rook = board.get_piece(Position::new('h', 1)).unwrap().as_ref();

        let new_board = right_rook.move_to(Position::new('h', 2), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White right Rook can't move to h2");

        let new_board = right_rook.move_to(Position::new('g', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White right Rook can't move to g1");
    }

    #[test]
    fn test_black_rook_invalid_initial_move() {
        init();
        let board = Board::new();
        let left_rook = board.get_piece(Position::new('a', 8)).unwrap().as_ref();
        let new_board = left_rook.move_to(Position::new('a', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black left rook can't move to a7");

        let new_board = left_rook.move_to(Position::new('b', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black left Rook can't move to b1");
       
        let right_rook = board.get_piece(Position::new('h', 8)).unwrap().as_ref();

        let new_board = right_rook.move_to(Position::new('h', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black right Rook can't move to h2");

        let new_board = right_rook.move_to(Position::new('g', 8), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black right Rook can't move to g1");
    }

    #[test]
    fn test_rook_blocked_move() {
        init();
        let mut board = Board::new();
        board.squares[Position::new('a', 2).to_index() as usize].piece = None;
        let mut index = 2;
        while index < 7 {
            index += 1;
            board.squares[index as usize].piece = None;
        }
        let left_rook = board.get_piece(Position::new('a', 1)).unwrap().as_ref();
        
        let new_board = left_rook.move_to(Position::new('a', 8), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White left rook can't move to a6, blocked by black pawn");

        let new_board = left_rook.move_to(Position::new('g', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"White left Rook can't move to g1, blocked by white knight");
    }

    


}