use std::{borrow::BorrowMut, ops::ControlFlow};

use crate::{board::BOARD_SQUARES, BoardTrait, Position};

use super::{ChessError, Color, Piece, PieceType};

pub fn move_to(
    rook: &PieceType,
    position: Position,
    board: &mut dyn BoardTrait,
) -> Result<Option<PieceType>, ChessError> {
    match rook {
        PieceType::Rook(color, current_position) => {
            can_move_to(&current_position, &color, position, board)?;

            let captured_piece = board.square_mut(&position).piece;
            board.square_mut(&current_position).piece = None;
            board.borrow_mut().square_mut(&position).piece =
                Some(PieceType::Rook(*color, position));

            Ok(captured_piece)
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
    board: &dyn BoardTrait,
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

    let square = &board.square(&position);
    if square.piece.is_some() {
        if square.piece.as_ref().unwrap().color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn rook_move(
    board: &dyn BoardTrait,
    old_index: i32,
    new_index: i32,
    jump: i32,
) -> Result<(), ChessError> {
    if jump % 8 == 0 {
        let mut index = old_index;
        if new_index > old_index {
            index += 8;
            while index != new_index {
                let square = &board.square(&Position::from_index(index));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 8;
            }
        } else {
            index -= 8;
            while index != new_index {
                let square = &board.square(&Position::from_index(index));
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
                let square = &board.square(&Position::from_index(index));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 1;
            }
        } else {
            index -= 1;
            while index != new_index {
                let square = &board.square(&Position::from_index(index));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index -= 1;
            }
        }
    }
    Ok(())
}

pub fn possible_moves(
    current_position: &Position,
    color: &Color,
    board: &dyn BoardTrait,
) -> Vec<Position> {
    let current_index = current_position.to_index();
    let mut next_inndex = current_index + 8;
    let mut positions = vec![];
    while next_inndex <= BOARD_SQUARES {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex += 8;
    }

    let mut next_inndex = current_index + 1;
    while next_inndex % 8 == 0 {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex += 1;
    }

    let mut next_inndex = current_index - 8;
    while next_inndex >= 0 {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex -= 8;
    }

    let mut next_inndex = current_index - 1;
    while next_inndex % 8 == 0 {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex -= 1;
    }
    positions
}

fn valide_move(
    color: &Color,
    board: &dyn BoardTrait,
    next_inndex: i32,
    positions: &mut Vec<Position>,
) -> ControlFlow<()> {
    let square = &board.square(&Position::from_index(next_inndex));
    if square.piece.is_some() {
        if square.piece.as_ref().unwrap().color() != color {
            positions.push(Position::new(square.x, square.y));
        }
        return ControlFlow::Break(());
    }
    positions.push(Position::new(square.x, square.y));
    ControlFlow::Continue(())
}

#[cfg(test)]
mod test {

    use crate::{
        board,
        pieces::{ChessError, Color, Piece, PieceType},
        BoardTrait, Position,
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_white_rook_invalid_move() {
        init();
        let mut board = board::empty_board();
        let mut rook = PieceType::Rook(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(rook);

        let new_board = rook.move_to(Position::new('e', 5), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White left Rook can't move to e5"
        );

        let new_board = rook.move_to(Position::new('e', 3), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White left Rook can't move to e3"
        );

        let new_board = rook.move_to(Position::new('c', 5), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White right Rook can't move to c4"
        );

        let new_board = rook.move_to(Position::new('c', 3), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "White right Rook can't move to c3"
        );
    }

    #[test]
    fn test_black_rook_invalid_initial_move() {
        init();
        let mut board = board::new_board();
        let mut left_rook = PieceType::Rook(Color::Black, Position::new('a', 8));
        board.square_mut(&Position::new('a', 8)).piece = Some(left_rook);

        let new_board = left_rook.move_to(Position::new('a', 7), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black left rook can't move to a7"
        );

        let new_board = left_rook.move_to(Position::new('b', 7), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "Black left Rook can't move to b7"
        );

        let mut right_rook = PieceType::Rook(Color::Black, Position::new('h', 8));
        board.square_mut(&Position::new('h', 8)).piece = Some(right_rook);

        let new_board = right_rook.move_to(Position::new('h', 7), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black right Rook can't move to h7"
        );

        let new_board = right_rook.move_to(Position::new('g', 8), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidCapture,
            "Black right Rook can't move to g8"
        );

        let new_board = right_rook.move_to(Position::new('g', 7), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "Black right Rook can't move to g7"
        );
    }

    #[test]
    fn test_rook_blocked_move() {
        init();
        let mut board = board::new_board();
        board.square_mut(&Position::new('a', 2)).piece = None;
        let mut index = 2;
        while index < 7 {
            index += 1;
            board.square_mut(&Position::from_index(index)).piece = None;
        }
        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 1));
        board.square_mut(&Position::new('a', 1)).piece = Some(left_rook);

        let new_board = left_rook.move_to(Position::new('a', 8), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::BlockedMove,
            "White left rook can't move to a8, blocked by black pawn"
        );

        let new_board = left_rook.move_to(Position::new('g', 1), &mut board);
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::BlockedMove,
            "White left Rook can't move to g1, blocked by white knight"
        );
    }

    #[test]
    fn test_rook_capture() {
        init();
        let mut board = board::new_board();
        board.square_mut(&Position::new('a', 2)).piece = None;
        let mut index = 2;
        while index < 7 {
            index += 1;
            board.square_mut(&Position::from_index(index)).piece = None;
        }
        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 1));
        board.square_mut(&Position::new('a', 1)).piece = Some(left_rook);

        let new_piece = left_rook.move_to(Position::new('a', 7), &mut board);
        assert!(
            new_piece.is_ok(),
            "White left rook should be able to capture black pawn in a7"
        );

        let _ = new_piece.unwrap();
        let left_rook = board.get_piece(Position::new('a', 7)).unwrap();
        assert_eq!(left_rook.color(), Color::White, "White left rook is in a7");

        let mut left_rook = PieceType::Rook(Color::White, Position::new('a', 7));
        board.square_mut(&Position::new('a', 7)).piece = Some(left_rook);
        let new_piece = left_rook.move_to(Position::new('b', 7), &mut board);

        assert!(
            new_piece.is_ok(),
            "White left rook should be able to capture black pawn in b7"
        );
        assert!(
            board.get_piece(Position::new('b', 7)).is_some(),
            "White left rook should be in b7"
        );
    }
}
