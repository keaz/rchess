use crate::{pieces::ChessError, Board, Position};

use super::{Color, Piece, PieceType};

pub fn move_to(
    king: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<(Board, Option<PieceType>), ChessError> {
    match king {
        PieceType::King(color, current_position) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            let captured_piece = board.squares[new_index as usize].piece;
            board.squares[old_index as usize].piece = None;
            board.squares[new_index as usize].piece = Some(PieceType::King(*color, position));

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

    if let Some(piece) = &board.squares[new_index as usize].piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn is_check(king: PieceType, board: &Board) -> bool {
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
                        if piece.can_move_to(position, &board).is_ok() {
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

pub fn can_king_move_safe_position(king: PieceType, board: &Board) -> bool {
    match king {
        PieceType::King(color, current_position) => {
            let current_index = current_position.to_index();
            //tempary board to check if king can move to safe position
            let mut tmp_board = board.clone();
            tmp_board.squares[current_index as usize].piece = None;

            for i in 7..10 {
                if let Some(square) = tmp_board.squares.get((current_index + i) as usize) {
                    if let Some(piece) = &square.piece {
                        if piece.color() != color {
                            let next_position = Position::new(square.x, square.y);
                            tmp_board.squares[next_position.to_index() as usize].piece = None;
                            if !is_check(PieceType::King(color, next_position), &tmp_board) {
                                return false;
                            }
                        }
                    }
                }

                if let Some(square) = tmp_board.squares.get((current_index - i) as usize) {
                    if let Some(piece) = &square.piece {
                        if piece.color() != color {
                            let next_position = Position::new(square.x, square.y);
                            tmp_board.squares[next_position.to_index() as usize].piece = None;
                            if !is_check(PieceType::King(color, next_position), &tmp_board) {
                                return false;
                            }
                        }
                    }
                }
            }
            if let Some(square) = tmp_board.squares.get((current_index + 1) as usize) {
                if let Some(piece) = &square.piece {
                    if piece.color() != color {
                        let next_position = Position::new(square.x, square.y);
                        tmp_board.squares[next_position.to_index() as usize].piece = None;
                        if !is_check(PieceType::King(color, next_position), &tmp_board) {
                            return false;
                        }
                    }
                }
            }

            if let Some(square) = tmp_board.squares.get((current_index - 1) as usize) {
                if let Some(piece) = &square.piece {
                    if piece.color() != color {
                        let next_position = Position::new(square.x, square.y);
                        tmp_board.squares[next_position.to_index() as usize].piece = None;
                        if !is_check(PieceType::King(color, next_position), &tmp_board) {
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

pub fn possible_moves(current_position: &Position, color: &Color, board: &Board) -> Vec<Position> {
    let mut positions = vec![];
    let moves = [7, 8, 9, 1, -7, -8, -9, -1];
    for m in moves.iter() {
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
        pieces::{king::is_check, ChessError, Color, Piece, PieceType},
        Board, Position,
    };

    #[test]
    fn test_invalid_king_move() {
        init();
        let mut board = Board::empty();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.squares[Position::new('e', 4).to_index() as usize].piece = Some(king);

        let result = king.move_to(Position::new('e', 6), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in vertical"
        );

        let result = king.move_to(Position::new('g', 6), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in diagonal"
        );

        let result = king.move_to(Position::new('e', 4), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "King can't move more than one squares in horizontal"
        );

        let result = king.move_to(Position::new('a', 5), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "e4 King can't move to a5"
        );
    }

    #[test]
    fn test_king_valid_moves() {
        init();
        let mut board = Board::empty();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.squares[Position::new('e', 4).to_index() as usize].piece = Some(king);

        let result = king.move_to(Position::new('e', 5), board.clone());
        assert_eq!(result.is_ok(), true, "e4 King can move to e5");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('e', 5)).unwrap();
        let result = king.move_to(Position::new('f', 6), board.clone());
        assert!(result.is_ok(), "e5 King can move to f6");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 6)).unwrap();
        let result = king.move_to(Position::new('g', 6), board.clone());
        assert_eq!(result.is_ok(), true, "f6 King can move to g6");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 6)).unwrap();
        let result = king.move_to(Position::new('h', 5), board.clone());
        assert_eq!(result.is_ok(), true, "g6 King can move to h5");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 5)).unwrap();
        let result = king.move_to(Position::new('h', 4), board.clone());
        assert_eq!(result.is_ok(), true, "h5 King can move to h4");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 4)).unwrap();
        let result = king.move_to(Position::new('g', 4), board.clone());
        assert_eq!(result.is_ok(), true, "h4 King can move to g4");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 4)).unwrap();
        let result = king.move_to(Position::new('f', 4), board.clone());
        assert_eq!(result.is_ok(), true, "g4 King can move to f4");

        let (board, _capture) = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 4)).unwrap();
        let result = king.move_to(Position::new('e', 3), board.clone());
        assert_eq!(result.is_ok(), true, "f4 King can move to e3");
    }

    #[test]
    fn test_king_unsafe_move() {
        init();
        let mut board = Board::empty();
        let mut king = PieceType::King(Color::White, Position::new('e', 4));
        board.squares[Position::new('e', 4).to_index() as usize].piece = Some(king);

        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 7));
        board.squares[Position::new('f', 7).to_index() as usize].piece = Some(black_queen);

        let result = king.move_to(Position::new('f', 4), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to f4, unsafe by balck queen"
        );

        let black_pawn = PieceType::Pawn(Color::Black, Position::new('d', 6), false);
        board.squares[Position::new('d', 6).to_index() as usize].piece = Some(black_pawn);
        let result = king.move_to(Position::new('e', 5), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to e5, unsafe by balck pawn"
        );

        let black_king = PieceType::King(Color::Black, Position::new('d', 2));
        board.squares[Position::new('d', 2).to_index() as usize].piece = Some(black_king);
        let result = king.move_to(Position::new('e', 5), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::UnSafeKing,
            "White King can't move to d2, unsafe by balck king"
        );
    }

    #[test]
    fn test_king_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut king = PieceType::King(Color::White, Position::new('e', 1));
        board.squares[Position::new('e', 1).to_index() as usize].piece = Some(king);

        let result = king.move_to(Position::new('e', 2), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "White King can't capture White e2 Pawn"
        );
    }

    #[test]
    fn test_valid_capture() {
        init();
        let mut board = Board::empty();
        let mut king = PieceType::King(Color::White, Position::new('e', 6));
        board.squares[Position::new('e', 6).to_index() as usize].piece = Some(king);
        board.squares[Position::new('e', 1).to_index() as usize].piece = None;
        let _black_pawn = PieceType::Pawn(Color::Black, Position::new('e', 7), false);

        let result = king.move_to(Position::new('d', 7), board.clone());
        assert_eq!(
            result.is_ok(),
            true,
            "e6 White King can capture Black d7 Pawn"
        );
        let (new_board, _capture) = result.unwrap();
        let _piece = new_board.get_piece(Position::new('d', 7)).unwrap();
    }

    #[test]
    fn test_king_check() {
        init();
        let mut board = Board::empty();
        let king = PieceType::King(Color::Black, Position::new('e', 8));
        let white_queen = PieceType::Queen(Color::White, Position::new('f', 7));
        board.squares[Position::new('e', 8).to_index() as usize].piece = Some(king);
        board.squares[Position::new('f', 7).to_index() as usize].piece = Some(white_queen);

        assert!(
            is_check(king, &board),
            "e6 White King is checked by Black f7 Queen"
        );
    }

    #[test]
    fn test_king_not_check() {
        init();
        let mut board = Board::empty();
        let king = PieceType::King(Color::White, Position::new('e', 6));
        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 8));
        board.squares[Position::new('e', 6).to_index() as usize].piece = Some(king);
        board.squares[Position::new('f', 8).to_index() as usize].piece = Some(black_queen);

        assert!(
            !is_check(king, &board),
            "e6 White King is not checked by Black f8 Queen"
        );
    }
}
