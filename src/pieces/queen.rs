use crate::{Position, Board, pieces::{rook::rook_move, bishop::bishop_move}};

use super::{Color, Piece, ChessError};


#[derive(Clone, Copy, Debug)]
pub struct Queen {
    pub color: Color,
    pub position: Position,
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

    fn move_to(&mut self, position: Position,  mut board: Board) -> Result<Board,ChessError> {
        let new_index = position.to_index();
        let old_index = self.position.to_index();

        let jump = new_index - old_index;
        if jump % 7 != 0 && jump % 9 != 0 && jump % 8 != 0 && jump / 8 != 0 {
            return Err(ChessError::InvalidMove);
        }

        if jump.abs() < 8 && position.y != self.position.y {
            return Err(ChessError::InvalidMove);
        }

        if jump % 8 == 0 || jump / 8 == 0 {
            rook_move(&mut board, old_index, new_index, jump)?;
        }

        if jump % 7 == 0 || jump % 9 == 0 {
            bishop_move(&mut board, old_index, new_index, jump)?;
        }

        let square = &mut board.squares[new_index as usize];
        if let Some(piece) = &square.piece {
            if piece.color() == self.color {
                return Err(ChessError::InvalidCapture);
            }
        }

        self.position = position.clone();
        board.squares[new_index as usize].piece = board.squares[old_index as usize].piece.take();

        println!("{:?} Queen move_to",self.color);
        Ok(board)
    }

    fn color(&self) -> &Color {
        &self.color
    }
    
}

#[cfg(test)]
mod test {
    use crate::{Board, pieces::{queen::Queen, Piece, Color, ChessError}, Position};



    fn init(){
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_queen_invalid_move(){
        init();
       
        let mut board = Board::empty();
        let mut queen = Queen::new(Color::White, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));

        let result = queen.move_to(Position::new('c',2), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidMove,"d4 White Queen should not be able to move to c2");
        let result = queen.move_to(Position::new('5',6), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidMove,"d4 White Queen should not be able to move to 5e");
        let result = queen.move_to(Position::new('a',8), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidMove,"d4 White Queen should not be able to move to a8");
        let result = queen.move_to(Position::new('h',1), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidMove,"d4 White Queen should not be able to move to h1");
    }

    #[test]
    fn test_queen_blocked_move(){
        init();

        let mut board = Board::new();
        let mut queen = Queen::new(Color::White, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));
        board.squares[Position::new('d',1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d',8), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::BlockedMove,"d4 White Queen should not be able to move to d8");
        let result = queen.move_to(Position::new('d',1), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::BlockedMove,"d4 White Queen should not be able to move to d1");
        let result = queen.move_to(Position::new('a',1), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::BlockedMove,"d4 White Queen should not be able to move to a1");
        let result = queen.move_to(Position::new('h',8), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::BlockedMove,"d4 White Queen should not be able to move to h8");
    }

    #[test]
    fn test_white_queen_invalid_capture(){
        init();

        let mut board = Board::new();
        let mut queen = Queen::new(Color::White, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));
        board.squares[Position::new('d',1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d',2), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidCapture,"d4 White Queen should not be able to capture d2 white pawn");
        
    }

    #[test]
    fn test_black_queen_invalid_capture(){
        init();

        let mut board = Board::new();
        let mut queen = Queen::new(Color::Black, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));
        board.squares[Position::new('d',8).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d',7), board.clone());
        assert_eq!(result.err().unwrap(), ChessError::InvalidCapture,"d4 Black Queen should not be able to capture d7 black pawn");
    }

    #[test]
    fn test_white_queen_valid_capture(){
        init();

        let mut board = Board::new();
        let mut queen = Queen::new(Color::White, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));
        board.squares[Position::new('d',1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d',7), board.clone());
        assert!(result.is_ok(),"d4 White Queen should be able to move to capture d7 black pawn");
        
    }

    #[test]
    fn test_black_queen_valid_capture(){
        init();

        let mut board = Board::new();
        let mut queen = Queen::new(Color::Black, Position::new('d',4));
        board.squares[queen.position.to_index() as usize].piece = Some(Box::new(queen));
        board.squares[Position::new('d',8).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d',2), board.clone());
        assert!(result.is_ok(),"d4 Black Queen should not be able to capture d2 white pawn");
    }

}