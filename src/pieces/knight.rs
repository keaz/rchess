use std::ops::ControlFlow;

use crate::{Board, Position, BOARD_SQUARES};

use super::{ChessError, Color, Piece, PieceType};

pub fn move_to(
    knight: &PieceType,
    position: Position,
    mut board: Board,
) -> Result<(Board, Option<PieceType>), ChessError> {
    match knight {
        PieceType::Knight(color, current_position) => {
            let new_index = position.to_index();
            let old_index = current_position.to_index();

            can_move_to(&current_position, &color, position, &board)?;

            let captured_piece = board.squares[new_index as usize].piece;
            board.squares[old_index as usize].piece = None;
            board.squares[new_index as usize].piece = Some(PieceType::Knight(*color, position));

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
    if new_index < 0 || new_index >= BOARD_SQUARES {
        return Err(ChessError::InvalidMove);
    }
    let old_index = current_position.to_index();

    let jump = new_index - old_index;
    let jump = jump.abs();
    if jump != 6 && jump != 10 && jump != 15 && jump != 17 {
        return Err(ChessError::InvalidMove);
    }

    let square = &board.squares[new_index as usize];
    if let Some(piece) = &square.piece {
        if piece.color() == color {
            return Err(ChessError::InvalidCapture);
        }
    }

    Ok(())
}

pub fn possible_moves(current_position: &Position, color: &Color, board: &Board) -> Vec<Position> {
    let mut positions = vec![];
    let moves = [6, 10, 15, 17, -6, -10, -15, -17];
    for m in moves.iter() {
        let next_position = Position::from_index(current_position.to_index() + *m);
        if can_move_to(current_position, color, next_position, board) == Ok(()) {
            positions.push(next_position);
        }
    }
    positions
}

fn valide_move(
    color: &Color,
    board: &Board,
    next_inndex: i32,
    positions: &mut Vec<Position>,
) -> ControlFlow<()> {
    let square = &board.squares[next_inndex as usize];
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
        pieces::{ChessError, Color, Piece, PieceType},
        Board, Position,
    };

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn test_knight_invalid_move() {
        init();
        let mut board = Board::empty();
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('c', 1), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 Knight should not be able to move to c1"
        );

        let result = knight.move_to(Position::new('h', 6), board.clone());
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidMove,
            "d4 Knight should not be able to move to h6"
        );
    }

    #[test]
    fn test_knight_valid_move() {
        init();
        let mut board = Board::empty();
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('e', 6), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to e6"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('e', 6).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('f', 5), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to f5"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('f', 5).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('f', 3), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to f3"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('f', 3).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('e', 2), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to e2"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('e', 2).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('c', 2), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to c2"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('c', 2).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('b', 3), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to b3"
        );

        let (mut board, _capture) = result.unwrap();
        board.squares[Position::new('b', 3).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('b', 5), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to move to b5"
        );
    }

    #[test]
    fn test_kingh_invalid_capture() {
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 4));
        board.squares[Position::new('d', 4).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('e', 2), board);
        assert_eq!(
            result.err().unwrap(),
            ChessError::InvalidCapture,
            "d4 Knight should not be able to capture white pawn at e2"
        );
    }

    #[test]
    fn test_knight_valid_capture() {
        init();
        let mut board = Board::new();
        board.squares[1].piece = None;
        board.squares[Position::new('e', 2).to_index() as usize].piece = None;
        let mut knight = PieceType::Knight(Color::White, Position::new('d', 5));
        board.squares[Position::new('d', 5).to_index() as usize].piece = Some(knight);
        let result = knight.move_to(Position::new('e', 7), board);
        assert_eq!(
            result.is_ok(),
            true,
            "d4 Knight should be able to capture black pawn at e7"
        );
    }
}
