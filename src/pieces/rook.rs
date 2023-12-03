use std::borrow::BorrowMut;

use crate::{Board, Position};

use super::{ChessError, Color, Piece, PieceType};

pub fn move_to(
    rook: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<Board, ChessError> {
    match rook {
        PieceType::Rook(color, current_position, value) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            board.squares[old_index as usize].piece = None;
            board.borrow_mut().squares[new_index as usize].piece =
                Some(PieceType::Rook(*color, position, *value));
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

    if jump.abs() < 8 && position.y != current_position.y {
        return Err(ChessError::InvalidMove);
    }

    if jump % 8 != 0 && jump / 8 != 0 {
        return Err(ChessError::InvalidMove);
    }

    rook_move(board, old_index, new_index, jump)?;

    let square = &board.squares[new_index as usize];
    if square.piece.is_some() {
        if square.piece.as_ref().unwrap().color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn rook_move(
    board: &Board,
    old_index: i32,
    new_index: i32,
    jump: i32,
) -> Result<(), ChessError> {
    if jump % 8 == 0 {
        let mut index = old_index;
        if new_index > old_index {
            index += 8;
            while index != new_index {
                let square = &board.squares[index as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 8;
            }
        } else {
            index -= 8;
            while index != new_index {
                let square = &board.squares[index as usize];
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
                let square = &board.squares[index as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 1;
            }
        } else {
            index -= 1;
            while index != new_index {
                let square = &board.squares[index as usize];
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index -= 1;
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
    fn test_white_rook_invalid_move() {
        init();
        let mut board = Board::empty();
        let mut rook = PieceType::Rook(Color::White, Position::new('d', 4), 5);
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(rook);

        let new_board = rook.move_to(Position::new('e', 5), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White left Rook can't move to e5"
        );

        let new_board = rook.move_to(Position::new('e', 3), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White left Rook can't move to e3"
        );

        let new_board = rook.move_to(Position::new('c', 5), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White right Rook can't move to c4"
        );

        let new_board = rook.move_to(Position::new('c', 3), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White right Rook can't move to c3"
        );
    }

    #[test]
    fn test_black_rook_invalid_initial_move() {
        init();
        let mut board = Board::new();
        let mut left_rook = PieceType::Rook(Color::Black, Position::new('a', 8), 5);
        board.squares[Position::new('a', 8).to_index() as usize].piece = Some(left_rook);

        let new_board = left_rook.move_to(Position::new('a', 7), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black left rook can't move to a7"
        );

        let new_board = left_rook.move_to(Position::new('b', 7), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "Black left Rook can't move to b7"
        );

        let mut right_rook = PieceType::Rook(Color::Black, Position::new('h', 8), 5);
        board.squares[Position::new('h', 8).to_index() as usize].piece = Some(right_rook);

        let new_board = right_rook.move_to(Position::new('h', 7), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black right Rook can't move to h7"
        );

        let new_board = right_rook.move_to(Position::new('g', 8), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black right Rook can't move to g8"
        );

        let new_board = right_rook.move_to(Position::new('g', 7), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "Black right Rook can't move to g7"
        );
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
        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 1), 5);
        board.squares[Position::new('a', 1).to_index() as usize].piece = Some(left_rook);

        let new_board = left_rook.move_to(Position::new('a', 8), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::BlockedMove,
            "White left rook can't move to a8, blocked by black pawn"
        );

        let new_board = left_rook.move_to(Position::new('g', 1), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::BlockedMove,
            "White left Rook can't move to g1, blocked by white knight"
        );
    }

    #[test]
    fn test_rook_capture() {
        init();
        let mut board = Board::new();
        board.squares[Position::new('a', 2).to_index() as usize].piece = None;
        let mut index = 2;
        while index < 7 {
            index += 1;
            board.squares[index as usize].piece = None;
        }
        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 1), 5);
        board.squares[Position::new('a', 1).to_index() as usize].piece = Some(left_rook);

        let new_board = left_rook.move_to(Position::new('a', 7), board.clone());
        assert!(
            new_board.is_ok(),
            "White left rook should be able to capture black pawn in a7"
        );

        let mut new_board = new_board.unwrap();
        let left_rook = new_board.get_piece(Position::new('a', 7)).unwrap();
        assert_eq!(left_rook.color(), Color::White, "White left rook is in a7");

        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 7), 5);
        new_board.squares[Position::new('a', 7).to_index() as usize].piece = Some(left_rook);
        let new_board = left_rook.move_to(Position::new('b', 7), new_board.clone());

        assert!(
            new_board.is_ok(),
            "White left rook should be able to capture black pawn in b7"
        );
        assert!(
            new_board
                .unwrap()
                .get_piece(Position::new('b', 7))
                .is_some(),
            "White left rook should be in b7"
        );
    }
}
