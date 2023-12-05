use crate::{pieces::Color, Board, Position};

use super::{ChessError, Piece, PieceType};

pub fn move_to(
    bishop: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<(Board, Option<PieceType>), ChessError> {
    match bishop {
        PieceType::Bishop(color, current_position, value) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            let captured_piece = board.squares[new_index as usize].piece;
            board.squares[old_index as usize].piece = None;
            board.squares[new_index as usize].piece =
                Some(PieceType::Bishop(*color, position, *value));

            Ok((board, captured_piece))
        }
        _ => {
            return Err(ChessError::InvalidPiece);
        }
    }
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
    if jump % 7 != 0 && jump % 9 != 0 {
        return Err(ChessError::InvalidMove);
    }

    bishop_move(&board, old_index, new_index, jump)?;

    let square = &board.squares[new_index as usize];
    if let Some(piece) = &square.piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn bishop_move(
    board: &Board,
    old_index: i32,
    new_index: i32,
    jump: i32,
) -> Result<(), ChessError> {
    if jump % 7 == 0 {
        let mut index = old_index;
        if new_index > old_index {
            index += 7;
            while index != new_index {
                let square = &board.squares[index as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 7;
            }
        } else {
            index -= 7;
            while index != new_index {
                let square = &board.squares[index.abs() as usize];
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
                let square = &board.squares[index as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 9;
            }
        } else {
            index -= 9;
            while index != new_index {
                let square = &board.squares[index.abs() as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index -= 9;
            }
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
    fn test_bishop_invalid_move() {
        init();

        let mut board = Board::empty();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('c', 1), 3);
        board.squares[Position::new('c', 1).to_index() as usize].piece = Some(bishop);

        let board = bishop.move_to(Position::new('c', 5), board);
        assert_eq!(
            board.err().unwrap(),
            ChessError::InvalidMove,
            "c1 Beshop should not be able to move to c5"
        );
    }

    #[test]
    fn test_bishop_blocked_move() {
        init();

        let mut board = Board::new();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('c', 1), 3);
        board.squares[Position::new('c', 1).to_index() as usize].piece = Some(bishop);

        let board = bishop.move_to(Position::new('f', 4), board);
        assert_eq!(
            board.err().unwrap(),
            ChessError::BlockedMove,
            "c1 Beshop should not be able to move to f4, blocked by d2"
        );
    }

    #[test]
    fn test_bishop_valid_move() {
        init();

        let mut board = Board::empty();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('d', 2), 3);
        board.squares[Position::new('d', 2).to_index() as usize].piece = Some(bishop);

        let board = bishop.move_to(Position::new('f', 4), board);
        assert!(board.is_ok(), "c1 Beshop should be able to move to f4");
        let (board, capture) = board.unwrap();
        assert!(
            board.squares[Position::new('f', 4).to_index() as usize]
                .piece
                .is_some(),
            "c1 Beshop should be able to move to f4"
        );
        assert!(
            board.squares[Position::new('d', 2).to_index() as usize]
                .piece
                .is_none(),
            "d2 should be empty after c1 Beshop move to f4"
        );
    }

    #[test]
    fn test_bishop_invalid_capture() {
        init();

        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let mut bishop = PieceType::Bishop(Color::White, Position::new('e', 3), 3);
        board.squares[Position::new('e', 3).to_index() as usize].piece = Some(bishop);

        let board = bishop.move_to(Position::new('f', 2), board);
        assert_eq!(
            board.err().unwrap(),
            ChessError::InvalidCapture,
            "e3 Beshop should be able to capture white pawn at f2"
        );
    }

    #[test]
    fn test_bishop_valid_capture() {
        init();

        let mut board = Board::new();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let mut bishop = PieceType::Bishop(Color::White, Position::new('e', 3), 3);
        board.squares[Position::new('e', 3).to_index() as usize].piece = Some(bishop);

        let board = bishop.move_to(Position::new('a', 7), board);
        assert!(
            board.is_ok(),
            "e3 Beshop should be able to capture black pawn at a7"
        );
        let (board, capture) = board.unwrap();

        assert!(
            board.squares[Position::new('a', 7).to_index() as usize]
                .piece
                .is_some(),
            "e3 Beshop should be able to capture black pawn at a7"
        );
        assert!(
            board.squares[Position::new('e', 3).to_index() as usize]
                .piece
                .is_none(),
            "e3 should be empty after e3 Beshop capture black pawn at a7"
        );
    }
}
