
use crate::{pieces::Color, Position, Board};

use super::{Piece, ChessError};

#[derive(Clone, Copy, Debug)]
pub struct Bishop {
    pub color: Color,
    pub position: Position,
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

        let square = &mut board.squares[new_index as usize];
        if let Some(piece) = &square.piece {
            if piece.color() == self.color {
                return Err(ChessError::InvalidCapture);
            }
        }
        
        if jump % 7 == 0 {
            let mut index = old_index;
            if new_index > old_index {
                index += 7;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index += 7;
                }
            } else {
                index -= 7;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index -= 7;
                }
            }
            
        } else {
            let mut index = old_index;
            if new_index > old_index {
                index += 9;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index += 9;
                }
            } else {
                index -= 9;
                while index != new_index {
                    let square = &mut board.squares[index as usize];
                    if square.piece.is_some() {
                        return Err(ChessError::BlockedMove);
                    }
                    index -= 9;
                }
            }
        }

        board.squares[old_index as usize].piece = None;
        board.squares[new_index as usize].piece = Some(Box::new(
            Bishop {
                color: self.color,
                position,
            }
        ));

        println!("{:?} Bishop move_to {:?}",self.color,position);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}


#[cfg(test)]
mod test{

    use crate::{Board, Position, pieces::{ChessError, Color, Piece, bishop::Bishop}};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_bishop_invalid_move(){
        init();
        
        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let bishop = Bishop::new(Color::White, Position::new('c', 1));
        
        let board = bishop.move_to(Position::new('c', 5), board);
        assert_eq!(board.err().unwrap(), ChessError::InvalidMove,"c1 Beshop should not be able to move to c5");
    }

    #[test]
    fn test_bishop_blocked_move(){
        init();

        let board = Board::new();
        let bishop = Bishop::new(Color::White, Position::new('c', 1));
        
        let board = bishop.move_to(Position::new('f', 4), board);
        assert_eq!(board.err().unwrap(), ChessError::BlockedMove,"c1 Beshop should not be able to move to f4, blocked by d2");
    }

    #[test]
    fn test_bishop_valid_move(){
        init();
        
        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let bishop = Bishop::new(Color::White, Position::new('d', 2));
        
        let board = bishop.move_to(Position::new('f', 4), board);
        assert!(board.is_ok(),"c1 Beshop should be able to move to f4");
    }

    #[test]
    fn test_bishop_invalid_capture(){
        init();
        
        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let bishop = Bishop::new(Color::White, Position::new('e', 3));
        
        let board = bishop.move_to(Position::new('f', 2), board);
        assert_eq!(board.err().unwrap(),ChessError::InvalidCapture,"e3 Beshop should be able to capture white pawn at f2");
    }

    #[test]
    fn test_bishop_valid_capture(){
        init();
        
        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let bishop = Bishop::new(Color::White, Position::new('e', 3));
        
        let board = bishop.move_to(Position::new('a', 7), board);
        assert!(board.is_ok(),"e3 Beshop should be able to capture black pawn at a7");
    }

}