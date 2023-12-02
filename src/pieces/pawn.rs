
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

    fn move_to(&mut self, position: Position, mut board: Board) -> Result<Board,ChessError> {
        
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
        
        self.position = position.clone();
        board.borrow_mut().squares[new_index as usize].piece = board.squares[old_index as usize].piece.take();

        //TODO: Pawn promotion
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}


#[cfg(test)]
mod test {
    use crate::{Board, Position, pieces::{ChessError, Color, pawn::Pawn, Piece}};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_pawn_first_two_moves() {
        init();
        let mut board = Board::new();
        let mut pawn = Pawn::new(Color::White, Position::new('a', 2));

        board.squares[Position::new('a', 2).to_index() as usize].piece = Some(Box::new(pawn));
        let new_board = pawn.move_to(Position::new('a', 4), board.clone());
        assert!(new_board.is_ok());

        let mut board = new_board.unwrap();
        let mut pawn = Pawn::new(Color::Black, Position::new('a', 4));
        board.squares[Position::new('a', 4).to_index() as usize].piece = Some(Box::new(pawn));
        let new_board = pawn.move_to(Position::new('a', 6), board.clone()).err().unwrap();
        assert_eq!(new_board, ChessError::InvalidMove);
    }

    #[test]
    fn test_pawn_first_one_move() {
        init();
        let mut board = Board::new();
        let mut pawn = Pawn::new(Color::White, Position::new('a', 2));
        board.squares[Position::new('a', 2).to_index() as usize].piece = Some(Box::new(pawn));
        
        let new_board = pawn.move_to(Position::new('a', 3), board.clone());
        assert!(new_board.is_ok());
    }

    #[test]
    fn test_white_pawn_invalid_moves() {
        init();
        let mut board = Board::new();
        let mut pawn = Pawn::new(Color::White, Position::new('b', 2));
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(Box::new(pawn));
        
        let mut new_board = pawn.move_to(Position::new('b', 4), board.clone()).unwrap();
        let mut black_pawn = Pawn::new(Color::Black, Position::new('b', 7));
        new_board.squares[Position::new('b', 7).to_index() as usize].piece = Some(Box::new(black_pawn));
        
        let mut new_board = black_pawn.move_to(Position::new('b', 5), new_board.clone()).unwrap();
        let mut pawn = Pawn::new(Color::White, Position::new('b', 4));
        new_board.squares[Position::new('b', 4).to_index() as usize].piece = Some(Box::new(pawn));
        
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
        let mut board = Board::new();
        let mut pawn = Pawn::new(Color::Black, Position::new('c', 5));
        pawn.is_first_move = false;
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(Box::new(pawn));
        
        let same_position = pawn.move_to(Position::new('c', 5), board.clone()).err().unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn.move_to(Position::new('d', 5), board.clone()).err().unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn.move_to(Position::new('b', 5), board.clone()).err().unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn.move_to(Position::new('c', 6), board.clone()).err().unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

    }

    #[test]
    fn test_white_pawn_no_to_piece_capture() {
        init();
        let mut board = Board::new();
        let mut white_pawn = Pawn::new(Color::White, Position::new('b', 2));
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(Box::new(white_pawn));
        
        let new_board = white_pawn.move_to(Position::new('c', 3), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on right side of the white pawn");
        
        let new_board = white_pawn.move_to(Position::new('a', 3), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on left side of the white pawn");

    }

    #[test]
    fn test_black_pawn_no_to_piece_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = Pawn::new(Color::Black, Position::new('b', 7));
        board.squares[Position::new('b', 7).to_index() as usize].piece = Some(Box::new(black_pawn));
                
        let new_board = black_pawn.move_to(Position::new('c', 6), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on right side of the black pawn");

        let new_board = black_pawn.move_to(Position::new('a', 6), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"No piece to capture on left side of the black pawn");
    }

    #[test]
    fn test_white_pawn_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut white_pawn = Pawn::new(Color::White, Position::new('b', 2));
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(Box::new(white_pawn));
                
        let mut other_white_pawn = Pawn::new(Color::White, Position::new('c', 2));
        board.squares[Position::new('c', 2).to_index() as usize].piece = Some(Box::new(other_white_pawn));
        
        let new_board = other_white_pawn.move_to(Position::new('c', 3), board.clone()).unwrap();

        let failed_board = white_pawn.move_to(Position::new('c', 3), new_board.clone());
        assert_eq!(failed_board.err().unwrap(), ChessError::InvalidMove,"Capturing same color piece");
    }

    #[test]
    fn test_black_pawn_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = Pawn::new(Color::Black, Position::new('b', 7));
        board.squares[Position::new('b', 7).to_index() as usize].piece = Some(Box::new(black_pawn));
        
        let mut other_black_pawn = Pawn::new(Color::Black, Position::new('c', 7));
        board.squares[Position::new('c', 7).to_index() as usize].piece = Some(Box::new(other_black_pawn));

        let new_board = other_black_pawn.move_to(Position::new('c', 6), board.clone()).unwrap();

        let failed_board = black_pawn.move_to(Position::new('c', 6), new_board.clone());
        assert_eq!(failed_board.err().unwrap(), ChessError::InvalidMove,"Capturing same color piece");
    }


    #[test]
    fn test_white_pawn_capture() {
        init();
        let mut board = Board::new();
        let black_pawn = Pawn::new(Color::Black, Position::new('c', 5));
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(Box::new(black_pawn));
        
        let mut white_pawn = Pawn::new(Color::White, Position::new('b', 4));
        white_pawn.is_first_move = false;
        board.squares[Position::new('b', 2).to_index() as usize].piece = None;
        board.squares[Position::new('b', 4).to_index() as usize].piece = Some(Box::new(white_pawn));

        let new_board = white_pawn.move_to(Position::new('c', 5), board.clone());
        assert!(new_board.is_ok(), "White pawn should capture black pawn");

        let new_board = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('c', 5));
        assert_eq!(white_pawn.unwrap().color(),Color::White, "White pawn should be in c5");
    }

    #[test]
    fn test_black_pawn_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = Pawn::new(Color::Black, Position::new('c', 5));
        black_pawn.is_first_move = false;
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(Box::new(black_pawn));
        
        let white_pawn = Pawn::new(Color::White, Position::new('b', 4));
        board.squares[Position::new('b', 2).to_index() as usize].piece = None;
        board.squares[Position::new('b', 4).to_index() as usize].piece = Some(Box::new(white_pawn));
        
        let new_board = black_pawn.move_to(Position::new('b', 4), board.clone());
        assert!(new_board.is_ok(), "Black pawn should capture black pawn");

        let new_board = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('b', 4));
        assert_eq!(white_pawn.unwrap().color(),Color::Black, "Black pawn should be in b4");
    }
}