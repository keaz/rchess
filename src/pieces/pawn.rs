use std::borrow::BorrowMut;

use crate::{
    pieces::{ChessError, Piece},
    Board, Position,
};

use super::{Color, PieceType};

pub fn pawn_move_to(
    pawn: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<(Board, Option<PieceType>), ChessError> {
    match pawn {
        PieceType::Pawn(color, current_position, value, is_first_move) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, is_first_move, position, &board)?;

            let captured_piece = board.squares[new_index as usize].piece;
            board.squares[old_index as usize].piece = None;
            board.borrow_mut().squares[new_index as usize].piece =
                Some(PieceType::Pawn(*color, position, *value, false));

            //TODO: Pawn promotion

            return Ok((board, captured_piece));
        }
        _ => {
            return Err(ChessError::InvalidPiece);
        }
    }
}

pub fn can_move_to(
    current_position: &Position,
    color: &Color,
    is_first_move: &bool,
    position: Position,
    board: &Board,
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

    let square = &board.squares[new_index as usize];
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
    fn test_pawn_first_two_moves() {
        init();
        let mut board = Board::new();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('a', 2), 1, true);

        board.squares[Position::new('a', 2).to_index() as usize].piece = Some(pawn);
        let new_board = pawn.move_to(Position::new('a', 4), board.clone());
        assert!(new_board.is_ok());

        let (mut board, _capture) = new_board.unwrap();
        let mut pawn = PieceType::Pawn(Color::Black, Position::new('a', 4), 1, false);
        board.squares[Position::new('a', 4).to_index() as usize].piece = Some(pawn);
        let new_board = pawn
            .move_to(Position::new('a', 6), board.clone())
            .err()
            .unwrap();
        assert_eq!(new_board, ChessError::InvalidMove);
    }

    #[test]
    fn test_pawn_first_one_move() {
        init();
        let mut board = Board::new();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('a', 2), 1, true);
        board.squares[Position::new('a', 2).to_index() as usize].piece = Some(pawn);

        let new_board = pawn.move_to(Position::new('a', 3), board.clone());
        assert!(new_board.is_ok());
    }

    #[test]
    fn test_white_pawn_invalid_moves() {
        init();
        let mut board = Board::new();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('b', 2), 1, true);
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(pawn);

        let (mut new_board, _capture) = pawn.move_to(Position::new('b', 4), board.clone()).unwrap();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), 1, true);
        new_board.squares[Position::new('b', 7).to_index() as usize].piece = Some(black_pawn);

        let (mut new_board, _capture) = black_pawn
            .move_to(Position::new('b', 5), new_board.clone())
            .unwrap();
        let mut pawn = PieceType::Pawn(Color::White, Position::new('b', 4), 1, true);
        new_board.squares[Position::new('b', 4).to_index() as usize].piece = Some(pawn);

        let same_position = pawn
            .move_to(Position::new('b', 4), new_board.clone())
            .err()
            .unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn
            .move_to(Position::new('c', 4), new_board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn
            .move_to(Position::new('a', 4), new_board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn
            .move_to(Position::new('b', 3), new_board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);

        let wrong_move_two_squares = pawn
            .move_to(Position::new('b', 5), new_board.clone())
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
        let mut board = Board::new();
        let mut pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), 1, false);
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(pawn);

        let same_position = pawn
            .move_to(Position::new('c', 5), board.clone())
            .err()
            .unwrap();
        assert_eq!(same_position, ChessError::InvalidMove);

        let wrong_right_side_move = pawn
            .move_to(Position::new('d', 5), board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_right_side_move, ChessError::InvalidMove);

        let wrong_left_side_move = pawn
            .move_to(Position::new('b', 5), board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_left_side_move, ChessError::InvalidMove);

        let wrong_back_move = pawn
            .move_to(Position::new('c', 6), board.clone())
            .err()
            .unwrap();
        assert_eq!(wrong_back_move, ChessError::InvalidMove);
    }

    #[test]
    fn test_white_pawn_no_to_piece_capture() {
        init();
        let mut board = Board::new();
        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 2), 1, true);
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(white_pawn);

        let new_board = white_pawn.move_to(Position::new('c', 3), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on right side of the white pawn"
        );

        let new_board = white_pawn.move_to(Position::new('a', 3), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on left side of the white pawn"
        );
    }

    #[test]
    fn test_black_pawn_no_to_piece_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), 1, true);
        board.squares[Position::new('b', 7).to_index() as usize].piece = Some(black_pawn);

        let new_board = black_pawn.move_to(Position::new('c', 6), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on right side of the black pawn"
        );

        let new_board = black_pawn.move_to(Position::new('a', 6), board.clone());
        assert_eq!(
            new_board.err().unwrap(),
            ChessError::InvalidMove,
            "No piece to capture on left side of the black pawn"
        );
    }

    #[test]
    fn test_white_pawn_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 2), 1, true);
        board.squares[Position::new('b', 2).to_index() as usize].piece = Some(white_pawn);

        let mut other_white_pawn = PieceType::Pawn(Color::White, Position::new('c', 2), 1, true);
        board.squares[Position::new('c', 2).to_index() as usize].piece = Some(other_white_pawn);

        let (new_board, _capture) = other_white_pawn
            .move_to(Position::new('c', 3), board.clone())
            .unwrap();

        let failed_board = white_pawn.move_to(Position::new('c', 3), new_board.clone());
        assert_eq!(
            failed_board.err().unwrap(),
            ChessError::InvalidMove,
            "Capturing same color piece"
        );
    }

    #[test]
    fn test_black_pawn_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('b', 7), 1, true);
        board.squares[Position::new('b', 7).to_index() as usize].piece = Some(black_pawn);

        let mut other_black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 7), 1, true);
        board.squares[Position::new('c', 7).to_index() as usize].piece = Some(other_black_pawn);

        let (new_board, _capture) = other_black_pawn
            .move_to(Position::new('c', 6), board.clone())
            .unwrap();

        let failed_board = black_pawn.move_to(Position::new('c', 6), new_board.clone());
        assert_eq!(
            failed_board.err().unwrap(),
            ChessError::InvalidMove,
            "Capturing same color piece"
        );
    }

    #[test]
    fn test_white_pawn_capture() {
        init();
        let mut board = Board::new();
        let black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), 1, false);
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(black_pawn);

        let mut white_pawn = PieceType::Pawn(Color::White, Position::new('b', 4), 1, false);
        board.squares[Position::new('b', 2).to_index() as usize].piece = None;
        board.squares[Position::new('b', 4).to_index() as usize].piece = Some(white_pawn);

        let new_board = white_pawn.move_to(Position::new('c', 5), board.clone());
        assert!(new_board.is_ok(), "White pawn should capture black pawn");

        let (new_board, _capture) = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('c', 5));
        assert_eq!(
            white_pawn.unwrap().color(),
            Color::White,
            "White pawn should be in c5"
        );
    }

    #[test]
    fn test_black_pawn_capture() {
        init();
        let mut board = Board::new();
        let mut black_pawn = PieceType::Pawn(Color::Black, Position::new('c', 5), 1, false);
        board.squares[Position::new('c', 7).to_index() as usize].piece = None;
        board.squares[Position::new('c', 5).to_index() as usize].piece = Some(black_pawn);

        let white_pawn = PieceType::Pawn(Color::White, Position::new('b', 4), 1, false);
        board.squares[Position::new('b', 2).to_index() as usize].piece = None;
        board.squares[Position::new('b', 4).to_index() as usize].piece = Some(white_pawn);

        let new_board = black_pawn.move_to(Position::new('b', 4), board.clone());
        assert!(new_board.is_ok(), "Black pawn should capture black pawn");

        let (new_board, _capture) = new_board.unwrap();
        let white_pawn = new_board.get_piece(Position::new('b', 4));
        assert_eq!(
            white_pawn.unwrap().color(),
            Color::Black,
            "Black pawn should be in b4"
        );
    }
}
