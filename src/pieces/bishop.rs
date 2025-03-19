use std::ops::ControlFlow;

use crate::{
    Position,
    board::{BOARD_SQUARES, BoardTrait},
    pieces::Color,
};

use super::{ChessError, Piece, PieceType};

pub fn move_to(
    bishop: &PieceType,
    position: Position,
    board: &mut dyn BoardTrait,
) -> Result<Option<PieceType>, ChessError> {
    match bishop {
        PieceType::Bishop(color, current_position) => {
            can_move_to(&current_position, &color, position, board)?;

            let captured_piece = board.square(&position).piece;
            board.square_mut(&current_position).piece = None;
            board.square_mut(&position).piece = Some(PieceType::Bishop(*color, position));

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
    if jump % 7 != 0 && jump % 9 != 0 {
        return Err(ChessError::InvalidMove);
    }

    bishop_move(board, old_index, new_index, jump)?;

    let square = board.square(&position);
    if let Some(piece) = &square.piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn bishop_move(
    board: &dyn BoardTrait,
    old_index: i32,
    new_index: i32,
    jump: i32,
) -> Result<(), ChessError> {
    if jump % 7 == 0 {
        let mut index = old_index;
        if new_index > old_index {
            index += 7;
            while index != new_index {
                let square = board.square(&Position::from_index(index));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 7;
            }
        } else {
            index -= 7;
            while index != new_index {
                let square = board.square(&Position::from_index(index.abs()));
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
                let square = board.square(&Position::from_index(index));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index += 9;
            }
        } else {
            index -= 9;
            while index != new_index {
                let square = board.square(&Position::from_index(index.abs()));
                if square.piece.is_some() {
                    return Err(ChessError::BlockedMove);
                }
                index -= 9;
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
    let mut positions = vec![];

    let current_index = current_position.to_index();
    let mut next_inndex = current_index + 7;
    while next_inndex <= BOARD_SQUARES {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex += 7;
    }

    let mut next_inndex = current_index + 9;
    while next_inndex <= BOARD_SQUARES {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex += 9;
    }

    let mut next_inndex = current_index - 7;
    while next_inndex >= 0 {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex -= 7;
    }

    let mut next_inndex = current_index - 9;
    while next_inndex >= 0 {
        if let ControlFlow::Break(_) = valide_move(color, board, next_inndex, &mut positions) {
            break;
        }
        next_inndex -= 9;
    }
    positions
}

fn valide_move(
    color: &Color,
    board: &dyn BoardTrait,
    next_inndex: i32,
    positions: &mut Vec<Position>,
) -> ControlFlow<()> {
    let square = board.square(&Position::from_index(next_inndex));
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
        BoardTrait, Position, board,
        pieces::{ChessError, Color, Piece, PieceType, bishop::possible_moves},
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_bishop_invalid_move() {
        init();

        let mut board = board::empty_board();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('c', 1));
        board.square_mut(&Position::new('c', 1)).piece = Some(bishop);

        let board = bishop.move_to(Position::new('c', 5), &mut board);
        assert_eq!(
            board.err().unwrap(),
            ChessError::InvalidMove,
            "c1 Beshop should not be able to move to c5"
        );
    }

    #[test]
    fn test_bishop_blocked_move() {
        init();

        let mut board = board::new_board();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('c', 1));
        board.square_mut(&Position::new('c', 1)).piece = Some(bishop);

        let board = bishop.move_to(Position::new('f', 4), &mut board);
        assert_eq!(
            board.err().unwrap(),
            ChessError::BlockedMove,
            "c1 Beshop should not be able to move to f4, blocked by d2"
        );
    }

    #[test]
    fn test_bishop_valid_move() {
        init();

        let mut board = board::empty_board();
        let mut bishop = PieceType::Bishop(Color::White, Position::new('d', 2));
        board.square_mut(&Position::new('d', 2)).piece = Some(bishop);

        let piece_type = bishop.move_to(Position::new('f', 4), &mut board);
        assert!(piece_type.is_ok(), "c1 Beshop should be able to move to f4");
        assert!(
            board.square(&Position::new('f', 4)).piece.is_some(),
            "c1 Beshop should be able to move to f4"
        );
        assert!(
            board.square(&Position::new('d', 2)).piece.is_none(),
            "d2 should be empty after c1 Beshop move to f4"
        );
    }

    #[test]
    fn test_bishop_invalid_capture() {
        init();

        let mut board = board::new_board();
        board.square_mut(&Position::new('c', 2)).piece = None;
        let mut bishop = PieceType::Bishop(Color::White, Position::new('e', 3));
        board.square_mut(&Position::new('e', 3)).piece = Some(bishop);

        let piece_type = bishop.move_to(Position::new('f', 2), &mut board);
        assert_eq!(
            piece_type.err().unwrap(),
            ChessError::InvalidCapture,
            "e3 Beshop should be able to capture white pawn at f2"
        );
    }

    #[test]
    fn test_bishop_valid_capture() {
        init();

        let mut board = board::new_board();
        board.square_mut(&Position::new('c', 2)).piece = None;
        let mut bishop = PieceType::Bishop(Color::White, Position::new('e', 3));
        board.square_mut(&Position::new('e', 3)).piece = Some(bishop);

        let piece_type = bishop.move_to(Position::new('a', 7), &mut board);
        assert!(
            piece_type.is_ok(),
            "e3 Beshop should be able to capture black pawn at a7"
        );

        assert!(
            board.square(&Position::new('a', 7)).piece.is_some(),
            "e3 Beshop should be able to capture black pawn at a7"
        );
        assert!(
            board.square(&Position::new('e', 3)).piece.is_none(),
            "e3 should be empty after e3 Beshop capture black pawn at a7"
        );
    }

    #[test]
    fn test_white_bishop_possible_moves_on_full_board() {
        init();

        let board = board::new_board();

        let positions = possible_moves(&Position::new('d', 4), &Color::White, &board);
        assert_eq!(
            positions.len(),
            8,
            "d4 White Bishop should have 8 possible moves"
        );
    }

    #[test]
    fn test_black_bishop_possible_moves_on_full_board() {
        init();

        let board = board::new_board();

        let positions = possible_moves(&Position::new('d', 4), &Color::Black, &board);
        assert_eq!(
            positions.len(),
            8,
            "d4 Black Bishop should have 8 possible moves"
        );
    }
}
