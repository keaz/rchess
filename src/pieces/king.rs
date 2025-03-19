use crate::{BoardTrait, Position, pieces::ChessError};

use super::{Color, Piece, PieceType};

pub fn move_to(
    king: &PieceType,
    position: Position,
    board: &mut dyn BoardTrait,
) -> Result<Option<PieceType>, ChessError> {
    match king {
        PieceType::King(color, current_position) => {
            can_move_to(&current_position, &color, position, board)?;

            let captured_piece = board.square(&position).piece;
            board.square_mut(&current_position).piece = None;
            board.square_mut(&position).piece = Some(PieceType::King(*color, position));

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

    let jump = (new_index - old_index).abs();
    if jump != 7 && jump != 8 && jump != 9 && jump != 1 {
        //TODO:: King castle
        return Err(ChessError::InvalidMove);
    }

    let other_pieces = match color {
        Color::Black => board.get_all_white_pieces(),
        Color::White => board.get_all_black_pieces(),
    };

    for piece in other_pieces {
        match piece {
            PieceType::King(_, other_king_position) => {
                let king_index = other_king_position.to_index();
                if king_index == new_index + 7
                    || king_index == new_index - 7
                    || king_index == new_index + 8
                    || king_index == new_index - 8
                    || king_index == new_index + 9
                    || king_index == new_index - 9
                    || king_index == new_index + 1
                    || king_index == new_index - 1
                {
                    return Err(ChessError::UnSafeKing);
                }
            }
            PieceType::Pawn(_, pawn_positionn, _) => {
                let pawn_index = pawn_positionn.to_index();
                if pawn_index == new_index + 7 || pawn_index == new_index + 9 {
                    return Err(ChessError::UnSafeKing);
                }
            }
            _ => {
                if piece.can_move_to(position, board).is_ok() {
                    return Err(ChessError::UnSafeKing);
                }
            }
        }
    }

    if let Some(piece) = &board.square(&position).piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn is_check(king: PieceType, board: &dyn BoardTrait) -> bool {
    match king {
        PieceType::King(color, position) => {
            let other_pieces = match color {
                Color::Black => board.get_all_white_pieces(),
                Color::White => board.get_all_black_pieces(),
            };

            let new_index = position.to_index();

            for piece in other_pieces {
                match piece {
                    PieceType::King(_, _) => {
                        // return false;
                    }
                    PieceType::Pawn(_, pawn_positionn, _) => {
                        let pawn_index = pawn_positionn.to_index();
                        if pawn_index == new_index + 7 || pawn_index == new_index + 9 {
                            return true;
                        }
                    }
                    _ => {
                        if piece.can_move_to(position, board).is_ok() {
                            return true;
                        }
                    }
                }
            }

            false
        }
        _ => false,
    }
}

pub fn can_king_move_safe_position(king: PieceType, board: &dyn BoardTrait) -> bool {
    match king {
        PieceType::King(color, current_position) => {
            let current_index = current_position.to_index();
            //tempary board to check if king can move to safe position
            let mut cloned = board.clone_as_a();
            let tmp_board = cloned.as_mut();

            tmp_board.square_mut(&current_position).piece = None;

            for i in 7..10 {
                if current_index < 63 {
                    let square = tmp_board.square(&Position::from_index(current_index + i));
                    if let Some(piece) = &square.piece {
                        if piece.color() != color {
                            let next_position = Position::new(square.x, square.y);
                            tmp_board.square_mut(&next_position).piece = None;
                            if !is_check(PieceType::King(color, next_position), tmp_board) {
                                return false;
                            }
                        }
                    }
                }

                if current_index > 0 {
                    let square = tmp_board.square(&Position::from_index(current_index - i));
                    if let Some(piece) = &square.piece {
                        if piece.color() != color {
                            let next_position = Position::new(square.x, square.y);
                            tmp_board.square_mut(&next_position).piece = None;
                            if !is_check(PieceType::King(color, next_position), tmp_board) {
                                return false;
                            }
                        }
                    }
                }
            }

            if current_index < 63 {
                let square = tmp_board.square(&Position::from_index(current_index + 1));
                if let Some(piece) = &square.piece {
                    if piece.color() != color {
                        let next_position = Position::new(square.x, square.y);
                        tmp_board.square_mut(&next_position).piece = None;
                        if !is_check(PieceType::King(color, next_position), tmp_board) {
                            return false;
                        }
                    }
                }
            }

            if current_index > 0 {
                let square = tmp_board.square(&Position::from_index(current_index - 1));
                if let Some(piece) = &square.piece {
                    if piece.color() != color {
                        let next_position = Position::new(square.x, square.y);
                        tmp_board.square_mut(&next_position).piece = None;
                        if !is_check(PieceType::King(color, next_position), tmp_board) {
                            return false;
                        }
                    }
                }
            }

            true
        }
        _ => false,
    }
}

pub fn possible_moves(
    current_position: &Position,
    color: &Color,
    board: &dyn BoardTrait,
) -> Vec<Position> {
    let mut positions = vec![];
    let moves = [7, 8, 9, 1, -7, -8, -9, -1];
    for m in moves.iter() {
        if current_position.to_index() + *m < 0 || current_position.to_index() + *m >= 64 {
            continue;
        }
        let next_position = Position::from_index(current_position.to_index() + *m);
        if can_move_to(current_position, color, next_position, board) == Ok(()) {
            positions.push(next_position);
        }
    }
    positions
}

#[cfg(test)]
mod test {

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    use crate::{
        BoardTrait, Position, board,
        pieces::{ChessError, Color, Piece, PieceType, king::is_check},
    };

    use super::possible_moves;

    #[test]
    fn test_invalid_king_move() {
        init();
        let mut board = board::empty_board();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.square_mut(&Position::new('e', 4)).piece = Some(king);

        let result = king.move_to(Position::new('e', 6), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in vertical"
        );

        let result = king.move_to(Position::new('g', 6), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in diagonal"
        );

        let result = king.move_to(Position::new('e', 4), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in horizontal"
        );

        let result = king.move_to(Position::new('a', 5), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "e4 King can't move to a5"
        );
    }

    #[test]
    fn test_king_valid_moves() {
        init();
        let mut board = board::empty_board();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.square_mut(&Position::new('e', 4)).piece = Some(king);

        let result = king.move_to(Position::new('e', 5), &mut board);
        assert_eq!(result.is_ok(), true, "e4 King can move to e5");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('e', 5)).unwrap();
        let result = king.move_to(Position::new('f', 6), &mut board);
        assert!(result.is_ok(), "e5 King can move to f6");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 6)).unwrap();
        let result = king.move_to(Position::new('g', 6), &mut board);
        assert_eq!(result.is_ok(), true, "f6 King can move to g6");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 6)).unwrap();
        let result = king.move_to(Position::new('h', 5), &mut board);
        assert_eq!(result.is_ok(), true, "g6 King can move to h5");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 5)).unwrap();
        let result = king.move_to(Position::new('h', 4), &mut board);
        assert_eq!(result.is_ok(), true, "h5 King can move to h4");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 4)).unwrap();
        let result = king.move_to(Position::new('g', 4), &mut board);
        assert_eq!(result.is_ok(), true, "h4 King can move to g4");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 4)).unwrap();
        let result = king.move_to(Position::new('f', 4), &mut board);
        assert_eq!(result.is_ok(), true, "g4 King can move to f4");

        let _ = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 4)).unwrap();
        let result = king.move_to(Position::new('e', 3), &mut board);
        assert_eq!(result.is_ok(), true, "f4 King can move to e3");
    }

    #[test]
    fn test_king_unsafe_move() {
        init();
        let mut board = board::empty_board();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.square_mut(&Position::new('e', 4)).piece = Some(king);

        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 7));
        board.square_mut(&Position::new('f', 7)).piece = Some(black_queen);

        let result = king.move_to(Position::new('f', 4), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to f4, unsafe by balck queen"
        );

        let black_pawn = PieceType::Pawn(Color::Black, Position::new('d', 6), false);
        board.square_mut(&Position::new('d', 6)).piece = Some(black_pawn);
        let result = king.move_to(Position::new('e', 5), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to e5, unsafe by balck pawn"
        );

        let black_king = PieceType::King(Color::Black, Position::new('d', 2));
        board.square_mut(&Position::new('d', 2)).piece = Some(black_king);
        let result = king.move_to(Position::new('e', 5), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to d2, unsafe by balck king"
        );
    }

    #[test]
    fn test_king_invalid_capture() {
        init();
        let mut board = board::new_board();
        let mut king = PieceType::King(Color::White, Position::new('e', 1));
        board.square_mut(&Position::new('e', 1)).piece = Some(king);

        let result = king.move_to(Position::new('e', 2), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "White King can't capture White e2 Pawn"
        );
    }

    #[test]
    fn test_valid_capture() {
        init();
        let mut board = board::empty_board();
        let mut king = PieceType::King(Color::White, Position::new('e', 6));
        board.square_mut(&Position::new('e', 6)).piece = Some(king);
        board.square_mut(&Position::new('e', 1)).piece = None;
        let _black_pawn = PieceType::Pawn(Color::Black, Position::new('e', 7), false);

        let result = king.move_to(Position::new('d', 7), &mut board);
        assert_eq!(
            result.is_ok(),
            true,
            "e6 White King can capture Black d7 Pawn"
        );
        let _ = result.unwrap();
        let piece = board.get_piece(Position::new('d', 7)).unwrap();
    }

    #[test]
    fn test_king_check() {
        init();
        let mut board = board::empty_board();
        let king = PieceType::King(Color::Black, Position::new('e', 8));
        let white_queen = PieceType::Queen(Color::White, Position::new('f', 7));
        board.square_mut(&Position::new('e', 8)).piece = Some(king);
        board.square_mut(&Position::new('f', 7)).piece = Some(white_queen);

        assert!(
            is_check(king, &board),
            "e6 White King is checked by Black f7 Queen"
        );
    }

    #[test]
    fn test_king_not_check() {
        init();
        let mut board = board::empty_board();
        let king = PieceType::King(Color::White, Position::new('e', 6));
        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 8));
        board.square_mut(&Position::new('e', 6)).piece = Some(king);
        board.square_mut(&Position::new('f', 8)).piece = Some(black_queen);

        assert!(
            !is_check(king, &board),
            "e6 White King is not checked by Black f8 Queen"
        );
    }

    #[test]
    fn king_test_possible_move() {
        init();
        let mut board = board::empty_board();
        let position = Position::new('e', 4);
        let king = PieceType::King(Color::White, position);
        board.square_mut(&position).piece = Some(king);

        let moves = possible_moves(&position, &Color::White, &board);
        assert_eq!(moves.len(), 8, "King can move to 8 positions");
    }

    #[test]
    fn king_test_0_possible_move() {
        init();
        let mut board = board::new_board();
        let moves = possible_moves(&Position::new('e', 1), &Color::White, &board);
        assert_eq!(moves.len(), 0, "King can not move");
    }
}
