use crate::{
    BoardTrait, Position,
    pieces::{bishop::bishop_move, rook::rook_move},
};

use super::{ChessError, Color, Piece, PieceType, bishop, rook};

pub fn move_to(
    queen: &PieceType,
    position: Position,
    board: &mut dyn BoardTrait,
) -> Result<Option<PieceType>, ChessError> {
    match queen {
        PieceType::Queen(color, current_position) => {
            can_move_to(&current_position, &color, position, board)?;

            let captured_piece = board.square_mut(&position).piece;
            board.square_mut(&current_position).piece = None;
            board.square_mut(&position).piece = Some(PieceType::Queen(*color, position));

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
    if jump % 7 != 0 && jump % 9 != 0 && jump % 8 != 0 && jump / 8 != 0 {
        return Err(ChessError::InvalidMove);
    }

    if jump.abs() < 7 && position.y != current_position.y {
        return Err(ChessError::InvalidMove);
    }

    if jump % 8 == 0 || (jump / 8 == 0 && position.y == current_position.y) {
        rook_move(board, old_index, new_index, jump)?;
    } else if jump % 7 == 0 || jump % 9 == 0 {
        bishop_move(board, old_index, new_index, jump)?;
    }

    let square = &board.square(&position);
    if let Some(piece) = &square.piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn possible_moves(
    current_position: &Position,
    color: &Color,
    board: &dyn BoardTrait,
) -> Vec<Position> {
    let mut bishop_positions = bishop::possible_moves(current_position, color, board);
    let rook_positions = rook::possible_moves(current_position, color, board);
    bishop_positions.extend(rook_positions);
    bishop_positions
}

#[cfg(test)]
mod test {
    use crate::{
        BoardTrait, Position, board,
        pieces::{
            ChessError, Color, Piece, PieceType,
            queen::{can_move_to, possible_moves},
        },
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_queen_invalid_move() {
        init();

        let mut board = board::empty_board();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);

        let result = queen.move_to(Position::new('c', 2), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to c2"
        );
        let result = queen.move_to(Position::new('e', 7), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to e7"
        );
        let result = queen.move_to(Position::new('a', 8), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to a8"
        );
        let result = queen.move_to(Position::new('h', 1), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 White Queen should not be able to move to h1"
        );
    }

    #[test]
    fn test_queen_valid_move() {
        init();

        let mut board = board::empty_board();
        let queen = PieceType::Queen(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);

        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('d', 2),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to d2"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('d', 3),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to d7"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('d', 5),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to d5"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('d', 6),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to d6"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('e', 5),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to e5"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('e', 4),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to e4"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('e', 3),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to e3"
        );

        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('c', 3),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to c3"
        );
        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('c', 4),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to c4"
        );

        let result = can_move_to(
            &Position::new('d', 4),
            &Color::White,
            Position::new('c', 5),
            &board,
        );
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to c5"
        );
    }

    #[test]
    fn test_queen_blocked_move() {
        init();

        let mut board = board::new_board();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);
        board.square_mut(&Position::new('d', 1)).piece = None;

        let result = queen.move_to(Position::new('d', 8), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to d8"
        );
        let result = queen.move_to(Position::new('d', 1), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to d1"
        );
        let result = queen.move_to(Position::new('a', 1), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to a1"
        );
        let result = queen.move_to(Position::new('h', 8), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::BlockedMove,
            "d4 White Queen should not be able to move to h8"
        );
    }

    #[test]
    fn test_white_queen_invalid_capture() {
        init();

        let mut board = board::new_board();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);
        board.square_mut(&Position::new('d', 1)).piece = None;

        let result = queen.move_to(Position::new('d', 2), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "d4 White Queen should not be able to capture d2 white pawn"
        );
    }

    #[test]
    fn test_black_queen_invalid_capture() {
        init();

        let mut board = board::new_board();
        let mut queen = PieceType::Queen(Color::Black, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);
        board.square_mut(&Position::new('d', 8)).piece = None;

        let result = queen.move_to(Position::new('d', 7), &mut board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "d4 Black Queen should not be able to capture d7 black pawn"
        );
    }

    #[test]
    fn test_white_queen_valid_capture() {
        init();

        let mut board = board::new_board();
        let mut queen = PieceType::Queen(Color::White, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);
        board.square_mut(&Position::new('d', 1)).piece = None;

        let result = queen.move_to(Position::new('d', 7), &mut board);
        assert!(
            result.is_ok(),
            "d4 White Queen should be able to move to capture d7 black pawn"
        );
    }

    #[test]
    fn test_black_queen_valid_capture() {
        init();

        let mut board = board::new_board();
        let mut queen = PieceType::Queen(Color::Black, Position::new('d', 4));
        board.square_mut(&Position::new('d', 4)).piece = Some(queen);
        board.square_mut(&Position::new('d', 8)).piece = None;

        let result = queen.move_to(Position::new('d', 2), &mut board);
        assert!(
            result.is_ok(),
            "d4 Black Queen should not be able to capture d2 white pawn"
        );
    }

    #[test]
    fn test_possible_moves() {
        init();

        let board = board::new_board();
        let positions = possible_moves(&Position::new('d', 4), &Color::White, &board);
        assert_eq!(positions.len(), 16);
    }
}
