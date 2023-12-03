use crate::{
    pieces::{bishop::bishop_move, rook::rook_move},
    Board, Position,
};

use super::{ChessError, Color, Piece, PieceType};

pub fn move_to(
    queen: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<Board, ChessError> {
    match queen {
        PieceType::Queen(color, current_position, value) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            board.squares[old_index as usize].piece = None;
            board.squares[new_index as usize].piece =
                Some(PieceType::Queen(*color, position, *value));
        }
        _ => {
            return Err(ChessError::InvalidPiece);
        }
    }

    Ok(board)
}

pub fn can_move_to(
    current_position: &Position,
    color: &Color,
    position: Position,
    board: &Board,
) -> Result<(), ChessError> {
    let new_index = position.to_index();
    let old_index = current_position.to_index();

    let jump = new_index - old_index;
    if jump % 7 != 0 && jump % 9 != 0 && jump % 8 != 0 && jump / 8 != 0 {
        return Err(ChessError::InvalidMove);
    }

    if jump.abs() < 8 && position.y != current_position.y {
        return Err(ChessError::InvalidMove);
    }

    if jump % 8 == 0 || jump / 8 == 0 {
        rook_move(&board, old_index, new_index, jump)?;
    }

    if jump % 7 == 0 || jump % 9 == 0 {
        bishop_move(&board, old_index, new_index, jump)?;
    }

    let square = &board.squares[new_index as usize];
    if let Some(piece) = &square.piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use crate::{
        pieces::{ChessError, Color, Piece, PieceType},
        Board, Position,
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_queen_invalid_move() {
        init();

        let mut board = Board::empty();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);

        let result = queen.move_to(Position::new('c', 2), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to c2"
        );
        let result = queen.move_to(Position::new('5', 6), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to 5e"
        );
        let result = queen.move_to(Position::new('a', 8), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to a8"
        );
        let result = queen.move_to(Position::new('h', 1), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to h1"
        );
    }

    #[test]
    fn test_queen_blocked_move() {
        init();

        let mut board = Board::new();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);
        board.squares[Position::new('d', 1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d', 8), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to d8"
        );
        let result = queen.move_to(Position::new('d', 1), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to d1"
        );
        let result = queen.move_to(Position::new('a', 1), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to a1"
        );
        let result = queen.move_to(Position::new('h', 8), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to h8"
        );
    }

    #[test]
    fn test_white_queen_invalid_capture() {
        init();

        let mut board = Board::new();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);
        board.squares[Position::new('d', 1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d', 2), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "d4 White Queen should not be able to capture d2 white pawn"
        );
    }

    #[test]
    fn test_black_queen_invalid_capture() {
        init();

        let mut board = Board::new();
        let mut queen = PieceType::Queen(Color::Black, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);
        board.squares[Position::new('d', 8).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d', 7), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "d4 Black Queen should not be able to capture d7 black pawn"
        );
    }

    #[test]
    fn test_white_queen_valid_capture() {
        init();

        let mut board = Board::new();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);
        board.squares[Position::new('d', 1).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d', 7), board.clone());
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to capture d7 black pawn"
        );
    }

    #[test]
    fn test_black_queen_valid_capture() {
        init();

        let mut board = Board::new();
        let mut queen = PieceType::Queen(Color::Black, Position::new('d', 4), 9);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(queen);
        board.squares[Position::new('d', 8).to_index() as usize].piece = None;

        let result = queen.move_to(Position::new('d', 2), board.clone());
        assert!(
            result.is_ok(),
            "d4 Black Queen should not be able to capture d2 white pawn"
        );
    }
}
