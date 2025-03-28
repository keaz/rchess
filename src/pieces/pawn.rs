use std::borrow::BorrowMut;

use crate::{
    Position,
    board::BoardTrait,
    pieces::{ChessError, Piece},
};

use super::{Color, PieceType};

pub fn pawn_move_to(
    pawn: &PieceType,
    position: Position,
    board: &mut dyn BoardTrait,
) -> Result<Option<PieceType>, ChessError> {
    match pawn {
        PieceType::Pawn(color, current_position, is_first_move) => {
            can_move_to(&current_position, &color, *is_first_move, position, board)?;

            let captured_piece = board.square_mut(&position).piece;
            board.square_mut(&current_position).piece = None;
            board.borrow_mut().square_mut(&position).piece =
                Some(PieceType::Pawn(*color, position, false));

            //TODO: Pawn promotion

            return Ok(captured_piece);
        }
        _ => {
            return Err(ChessError::InvalidPiece);
        }
    }
}

pub fn can_move_to(
    current_position: &Position,
    color: &Color,
    is_first_move: bool,
    position: Position,
    board: &dyn BoardTrait,
) -> Result<(), ChessError> {
    let new_index = position.to_index();
    let old_index = current_position.to_index();
    let jump = new_index - old_index;

    match color {
        Color::Black => {
            if jump > 0 {
                return Err(ChessError::InvalidMove);
            }
        }
        Color::White => {
            if jump < 0 {
                return Err(ChessError::InvalidMove);
            }
        }
    }
    let jump = jump.abs();
    if jump != 8 && jump != 16 && jump != 7 && jump != 9 {
        return Err(ChessError::InvalidMove);
    }

    if !is_first_move && jump == 16 {
        return Err(ChessError::InvalidMove);
    }

    let square = board.square(&position);
    if (jump == 8 || jump == 16) && square.piece.is_some() {
        return Err(ChessError::InvalidMove);
    }

    if (jump == 7 || jump == 9) && square.piece.is_none() {
        return Err(ChessError::InvalidMove);
    }

    let other_piece = &square.piece;
    if let Some(other_piece) = other_piece {
        if other_piece.color() == color {
            return Err(ChessError::InvalidMove);
        }
    }

    Ok(())
}

pub fn possible_moves(
    current_position: &Position,
    color: &Color,
    is_first_move: bool,
    board: &dyn BoardTrait,
) -> Vec<Position> {
    let mut positions = vec![];
    let moves = match (color, is_first_move) {
        (Color::Black, true) => [-8, -16, -7, -9].to_vec(),
        (Color::Black, false) => [-8, -7, -9].to_vec(),
        (Color::White, true) => [8, 16, 7, 9].to_vec(),
        (Color::White, false) => [8, 7, 9].to_vec(),
    };

    for m in moves.iter() {
        let next_position = Position::from_index(current_position.to_index() + *m);
        if can_move_to(current_position, color, is_first_move, next_position, board) == Ok(()) {
            positions.push(next_position);
        }
    }

    return positions;
}

#[cfg(test)]
mod test {
    use crate::{
        BoardTrait, Position, board,
        pieces::{ChessError, Color, Piece, PieceType, pawn::possible_moves},
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_pawn_first_two_moves() {
        init();
        let mut board = board::new_board();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('a', 2), true);

        board.square_mut(&Position::new('a', 2)).piece = Some(pawn);
        let new_board = pawn.move_to(Position::new('a', 4), &mut board);
        assert!(new_board.is_ok());

        let mut pawn = PieceType::Pawn(Color::Black, Position::new('a', 4), false);
        board.square_mut(&Position::new('a', 4)).piece = Some(pawn);
        let new_board = pawn
            .move_to(Position::new('a', 6), &mut board)
            .err()
            .unwrap();
        assert_eq!(new_board, ChessError::InvalidMove);
    }

    #[test]
    fn test_pawn_first_one_move() {
        init();
        let mut board = board::new_board();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('a', 2), true);
        board.square_mut(&Position::new('a', 2)).piece = Some(pawn);

        let new_board = pawn.move_to(Position::new('a', 3), &mut board);
        assert!(new_board.is_ok());
    }

    #[test]
    fn test_white_pawn_invalid_moves() {
        init();
        let mut board = board::new_board();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('b', 2), true);
        board.square_mut(&Position::new('b', 2)).piece = Some(pawn);

        let _ = pawn.move_to(Position::new('b', 4), &mut board).unwrap();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), true);
        board.square_mut(&Position::new('b', 7)).piece = Some(black_pawn);

        let _ = black_pawn
            .move_to(Position::new('b', 5), &mut board)
            .unwrap();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('b', 4), true);
        board.square_mut(&Position::new('b', 4)).piece = Some(pawn);

        let same_position = pawn
            .move_to(Position::new('b', 4), &mut board)
            .err()
            .unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn
            .move_to(Position::new('c', 4), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn
            .move_to(Position::new('a', 4), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn
            .move_to(Position::new('b', 3), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

        let wrong_move_two_squares = pawn
            .move_to(Position::new('b', 5), &mut board)
            .err()
            .unwrap();
        assert_eq!(
            wrong_move_two_squares,
            ChessError::InvalidMove,
            "Pawn can't move to a already occupied square"
        );
    }

    #[test]
    fn test_black_pawn_invalid_moves() {
        init();
        let mut board = board::new_board();
        let mut pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), false);
        board.square_mut(&Position::new('c', 7)).piece = None;
        board.square_mut(&Position::new('c', 5)).piece = Some(pawn);

        let same_position = pawn
            .move_to(Position::new('c', 5), &mut board)
            .err()
            .unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn
            .move_to(Position::new('d', 5), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn
            .move_to(Position::new('b', 5), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn
            .move_to(Position::new('c', 6), &mut board)
            .err()
            .unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);
    }

    #[test]
    fn test_white_pawn_no_to_piece_capture() {
        init();
        let mut board = board::new_board();
        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 2), true);
        board.square_mut(&Position::new('b', 2)).piece = Some(white_pawn);

        let piece_type = white_pawn.move_to(Position::new('c', 3), &mut board);
        assert_eq!(
            piece_type.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on right side of the white pawn"
        );

        let piece_type = white_pawn.move_to(Position::new('a', 3), &mut board);
        assert_eq!(
            piece_type.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on left side of the white pawn"
        );
    }

    #[test]
    fn test_black_pawn_no_to_piece_capture() {
        init();
        let mut board = board::new_board();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), true);
        board.square_mut(&Position::new('b', 7)).piece = Some(black_pawn);

        let piece_type = black_pawn.move_to(Position::new('c', 6), &mut board);
        assert_eq!(
            piece_type.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on right side of the black pawn"
        );

        let piece_type = black_pawn.move_to(Position::new('a', 6), &mut board);
        assert_eq!(
            piece_type.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on left side of the black pawn"
        );
    }

    #[test]
    fn test_white_pawn_invalid_capture() {
        init();
        let mut board = board::new_board();
        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 2), true);
        board.square_mut(&Position::new('b', 2)).piece = Some(white_pawn);

        let mut other_white_pawn = PieceType::Pawn(Color::White, Position::new('c', 2), true);
        board.square_mut(&Position::new('c', 2)).piece = Some(other_white_pawn);

        let _ = other_white_pawn
            .move_to(Position::new('c', 3), &mut board)
            .unwrap();

        let failed_board = white_pawn.move_to(Position::new('c', 3), &mut board);
        assert_eq!(
            failed_board.err().unwrap(),
            ChessError::InvalidMove,
            "Capturing same color piece"
        );
    }

    #[test]
    fn test_black_pawn_invalid_capture() {
        init();
        let mut board = board::new_board();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), true);
        board.square_mut(&Position::new('b', 7)).piece = Some(black_pawn);

        let mut other_black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 7), true);
        board.square_mut(&Position::new('c', 7)).piece = Some(other_black_pawn);

        let _ = other_black_pawn
            .move_to(Position::new('c', 6), &mut board)
            .unwrap();

        let failed_board = black_pawn.move_to(Position::new('c', 6), &mut board);
        assert_eq!(
            failed_board.err().unwrap(),
            ChessError::InvalidMove,
            "Capturing same color piece"
        );
    }

    #[test]
    fn test_white_pawn_capture() {
        init();
        let mut board = board::new_board();
        let black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), false);
        board.square_mut(&Position::new('c', 7)).piece = None;
        board.square_mut(&Position::new('c', 5)).piece = Some(black_pawn);

        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 4), false);
        board.square_mut(&Position::new('b', 2)).piece = None;
        board.square_mut(&Position::new('b', 4)).piece = Some(white_pawn);

        let new_piece = white_pawn.move_to(Position::new('c', 5), &mut board);
        assert!(&new_piece.is_ok(), "White pawn should capture black pawn");

        let white_pawn = board.get_piece(Position::new('c', 5));
        assert_eq!(
            white_pawn.unwrap().color(),
            Color::White,
            "White pawn should be in c5"
        );
    }

    #[test]
    fn test_black_pawn_capture() {
        init();
        let mut board = board::new_board();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), false);
        board.square_mut(&Position::new('c', 7)).piece = None;
        board.square_mut(&Position::new('c', 5)).piece = Some(black_pawn);

        let white_pawn = PieceType::Pawn(Color::White, Position::new('b', 4), false);
        board.square_mut(&Position::new('b', 2)).piece = None;
        board.square_mut(&Position::new('b', 4)).piece = Some(white_pawn);

        let new_piece = black_pawn.move_to(Position::new('b', 4), &mut board);
        assert!(&new_piece.is_ok(), "Black pawn should capture black pawn");

        let _ = new_piece.unwrap();
        let white_pawn = &board.get_piece(Position::new('b', 4));
        assert_eq!(
            white_pawn.unwrap().color(),
            Color::Black,
            "Black pawn should be in b4"
        );
    }

    #[test]
    fn test_possible_first_white_moves() {
        init();
        let board = board::new_board();
        let possible_moves = possible_moves(&Position::new('d', 2), &Color::White, true, &board);
        assert_eq!(possible_moves.len(), 2);
    }

    #[test]
    fn test_possible_second_white_moves() {
        init();
        let board = board::new_board();
        let possible_moves = possible_moves(&Position::new('d', 2), &Color::White, false, &board);
        assert_eq!(possible_moves.len(), 1);
    }

    #[test]
    fn test_possible_first_black_moves() {
        init();
        let board = board::new_board();
        let possible_moves = possible_moves(&Position::new('d', 7), &Color::Black, true, &board);
        assert_eq!(possible_moves.len(), 2);
    }

    #[test]
    fn test_possible_second_black_moves() {
        init();
        let board = board::new_board();
        let possible_moves = possible_moves(&Position::new('d', 7), &Color::Black, false, &board);
        assert_eq!(possible_moves.len(), 1);
    }
}
