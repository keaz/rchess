
use std::borrow::BorrowMut;

use log::info;

use crate::{pieces::ChessError, Position, Board};

use super::{Piece, Color};

#[derive(Clone, Copy, Debug)]
pub struct Pawn {
    pub color: Color,
    pub position: Position,
    pub is_first_move: bool,
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

        //TODO: Pawn promotion
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
}