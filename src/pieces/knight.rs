use crate::{Position, Board};

use super::{Color, ChessError, Piece};

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
        let jump = jump.abs();
        if jump != 6 && jump != 10 && jump != 15 && jump != 17 {
            return Err(ChessError::InvalidMove);
        }

        let square = &mut board.squares[new_index as usize];
        if let Some(piece) = &square.piece {
            if piece.color() == self.color {
                return Err(ChessError::InvalidCapture);
            } 
        }

        board.squares[self.position.to_index() as usize].piece = None;
        board.squares[new_index as usize].piece = Some(Box::new(Knight{
            color: self.color,
            position,
        }));
        println!("{:?} Knight move_to {:?}",self.color, position);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

#[cfg(test)]
mod test {
    use crate::{Position, Board, pieces::{Color, Piece, ChessError}};

    use super::Knight;


    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_knight_invalid_move(){
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        let knight = Knight::new(Color::White, Position::new('d', 4));
        let result = knight.move_to(Position::new('c', 1), board.clone());
        assert_eq!(result.err().unwrap(),ChessError::InvalidMove,"d4 Knight should not be able to move to c1");

        let result = knight.move_to(Position::new('h', 6), board.clone());
        assert_eq!(result.err().unwrap(),ChessError::InvalidMove,"d4 Knight should not be able to move to h6");
    }


    #[test]
    fn test_knight_valid_move(){
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        board.squares[Position::new('e', 2).to_index() as usize].piece = None;
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let knight = Knight::new(Color::White, Position::new('d', 4));
        let result = knight.move_to(Position::new('e', 6), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to e6");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('f', 5), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to f5");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('f', 3), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to f3");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('e', 2), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to e2");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('c', 2), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to c2");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('b', 3), board); 
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to b3");

        let board = result.unwrap();
        let result = knight.move_to(Position::new('b', 5), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to move to b5");
    }

    #[test]
    fn test_kingh_invalid_capture(){
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        let knight = Knight::new(Color::White, Position::new('d', 4));
        let result = knight.move_to(Position::new('e', 2), board);
        assert_eq!(result.err().unwrap(),ChessError::InvalidCapture,"d4 Knight should not be able to capture white pawn at e2");
    }

    #[test]
    fn test_knight_valid_capture(){
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        board.squares[Position::new('e', 2).to_index() as usize].piece = None;
        let knight = Knight::new(Color::White, Position::new('d', 5));
        let result = knight.move_to(Position::new('e', 7), board);
        assert_eq!(result.is_ok(),true,"d4 Knight should be able to capture black pawn at e7");
    }
}