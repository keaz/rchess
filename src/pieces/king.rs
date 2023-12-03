use crate::{pieces::ChessError, Board, Position};

use super::{Color, Piece, PieceType};

pub fn move_to(
    king: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<Board, ChessError> {
    match king {
        PieceType::King(color, current_position, value) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            board.squares[old_index as usize].piece = None;
            board.squares[new_index as usize].piece =
                Some(PieceType::King(*color, position, *value));
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

    let jump = (new_index - old_index).abs();
    if jump != 7 && jump != 8 && jump != 9 && jump != 1 {
        //TODO:: King castle
        println!("jump {}, new index {}, old_index {}", jump,new_index,old_index );
        return Err(ChessError::InvalidMove);
    }

    let other_pieces = match color {
        Color::Black => board.get_all_white_pieces(),
        Color::White => board.get_all_black_pieces(),
    };

    for piece in other_pieces {
        match piece {
            PieceType::King(_, _, _) => {
                //TODO:: King can't move to a position where the king can be captured
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

#[cfg(test)]
mod test {

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    use crate::{
        pieces::{ChessError, Color, Piece, PieceType},
        Board, Position,
    };

    #[test]
    fn test_invalid_king_move() {
        init();
        let mut board = Board::empty();
        let mut king = PieceType::King(Color::White, Position::new('e', 4), u8::MAX);
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
        let mut king = PieceType::King(Color::White, Position::new('e', 4), u8::MAX);
        board.squares[Position::new('e', 4).to_index() as usize].piece = Some(king);

        let result = king.move_to(Position::new('e', 5), board.clone());
        assert_eq!(result.is_ok(), true, "e4 King can move to e5");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('e', 5)).unwrap();
        let result = king.move_to(Position::new('f', 6), board.clone());
        assert!(result.is_ok(), "e5 King can move to f6");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 6)).unwrap();
        let result = king.move_to(Position::new('g', 6), board.clone());
        assert_eq!(result.is_ok(), true, "f6 King can move to g6");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 6)).unwrap();
        let result = king.move_to(Position::new('h', 5), board.clone());
        assert_eq!(result.is_ok(), true, "g6 King can move to h5");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 5)).unwrap();
        let result = king.move_to(Position::new('h', 4), board.clone());
        assert_eq!(result.is_ok(), true, "h5 King can move to h4");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('h', 4)).unwrap();
        let result = king.move_to(Position::new('g', 4), board.clone());
        assert_eq!(result.is_ok(), true, "h4 King can move to g4");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('g', 4)).unwrap();
        let result = king.move_to(Position::new('f', 4), board.clone());
        assert_eq!(result.is_ok(), true, "g4 King can move to f4");

        let board = result.unwrap();
        let mut king = *board.get_piece(Position::new('f', 4)).unwrap();
        let result = king.move_to(Position::new('e', 3), board.clone());
        assert_eq!(result.is_ok(), true, "f4 King can move to e3");
    }

    #[test]
    fn test_king_invalid_capture() {
        init();
        let mut board = Board::new();
        let mut king = PieceType::King(Color::White, Position::new('e', 1), u8::MAX);
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
        let mut board = Board::new();
        let mut king = PieceType::King(Color::White, Position::new('e', 6), u8::MAX);
        board.squares[Position::new('e', 6).to_index() as usize].piece = Some(king);
        board.squares[Position::new('e', 1).to_index() as usize].piece = None;

        let result = king.move_to(Position::new('d', 7), board.clone());
        assert_eq!(
            result.is_ok(),
            true,
            "e6 White King can capture Black d7 Pawn"
        );
        let new_board = result.unwrap();
        let piece = new_board.get_piece(Position::new('d', 7)).unwrap();
    }
}