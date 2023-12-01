use std::borrow::BorrowMut;

use crate::{Position, Board};

use super::{Color, Piece, ChessError};


#[derive(Clone, Copy, Debug)]
pub struct Rook {
    pub color: Color,
    pub position: Position,
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
        
        if jump.abs() < 8 && position.y != self.position.y {
            return Err(ChessError::InvalidMove);
        }

        if jump % 8 != 0 && jump / 8 != 0 { 
            return Err(ChessError::InvalidMove);
        }

        if jump % 8 == 0 {
            let mut index = old_index;
            if new_index > old_index {
                index += 8;
                while index != new_index {
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index += 8;
                }
            } else {
                index -= 8;
                while index != new_index {
                    let square = &mut board.borrow_mut().squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index -= 8;
                }
            }
            
        } else {
            let mut index = old_index;
            if new_index > old_index {
                index += 1;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index += 1;
                }
            } else {
                index -= 1;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index -= 1;
                }
            }
            
        }

        let square = &mut board.squares[new_index as usize];
        if square.piece.is_some() {
            if square.piece.as_ref().unwrap().color() == self.color {
                return Err(ChessError::InvalidCapture);
            }
        }

        board.squares[self.position.to_index() as usize].piece = None;
        board.borrow_mut().squares[new_index as usize].piece = Some(Box::new(Rook {
            color: self.color,
            position,
        }));

        println!("{:?} Rook move_to {:?}",self.color, position);
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
    fn test_white_rook_invalid_initial_move() {
        init();
        let board = Board::new();
        let left_rook = board.get_piece(Position::new('a', 1)).unwrap().as_ref();
        let new_board = left_rook.move_to(Position::new('a', 2), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"White left rook can't move to a2");

        let new_board = left_rook.move_to(Position::new('b', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"White left Rook can't move to b1");
       
        let right_rook = board.get_piece(Position::new('h', 1)).unwrap().as_ref();

        let new_board = right_rook.move_to(Position::new('h', 2), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"White right Rook can't move to h2");

        let new_board = right_rook.move_to(Position::new('g', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"White right Rook can't move to g1");
    }

    #[test]
    fn test_black_rook_invalid_initial_move() {
        init();
        let board = Board::new();
        let left_rook = board.get_piece(Position::new('a', 8)).unwrap().as_ref();
        let new_board = left_rook.move_to(Position::new('a', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"Black left rook can't move to a7");

        let new_board = left_rook.move_to(Position::new('b', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black left Rook can't move to b7");
       
        let right_rook = board.get_piece(Position::new('h', 8)).unwrap().as_ref();

        let new_board = right_rook.move_to(Position::new('h', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"Black right Rook can't move to h7");

        let new_board = right_rook.move_to(Position::new('g', 8), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidCapture,"Black right Rook can't move to g8");

        let new_board = right_rook.move_to(Position::new('g', 7), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::InvalidMove,"Black right Rook can't move to g7");
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
        assert_eq!(new_board.err().unwrap(), ChessError::BlockedMove,"White left rook can't move to a8, blocked by black pawn");

        let new_board = left_rook.move_to(Position::new('g', 1), board.clone());
        assert_eq!(new_board.err().unwrap(), ChessError::BlockedMove,"White left Rook can't move to g1, blocked by white knight");
    }

    #[test]
    fn test_rook_capture(){
        init();
        let mut board = Board::new();
        board.squares[Position::new('a', 2).to_index() as usize].piece = None;
        let mut index = 2;
        while index < 7 {
            index += 1;
            board.squares[index as usize].piece = None;
        }
        let left_rook = board.get_piece(Position::new('a', 1)).unwrap().as_ref();
        
        let new_board = left_rook.move_to(Position::new('a', 7), board.clone());
        assert!(new_board.is_ok(), "White left rook should be able to capture black pawn in a7");

        let new_board = new_board.unwrap();
        let left_rook = new_board.get_piece(Position::new('a', 7)).unwrap().as_ref();
        assert_eq!(left_rook.color(), Color::White, "White left rook is in a7");

        let left_rook = new_board.get_piece(Position::new('a', 7)).unwrap().as_ref();
        let new_board = left_rook.move_to(Position::new('b', 7), new_board.clone());
        assert!(new_board.is_ok(), "White left rook should be able to capture black pawn in b7");
        assert!(new_board.unwrap().get_piece(Position::new('b', 7)).is_some(), "White left rook should be in b7");
        
    }

}