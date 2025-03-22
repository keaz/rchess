use std::{fmt::Debug, ops::Range};

use crate::{
    Position, Square,
    pieces::{self, ChessError, Color, Piece, PieceType, king},
};

pub const BOARD_SIZE: i32 = 8;
pub const BOARD_SQUARES: i32 = BOARD_SIZE * BOARD_SIZE;

pub trait BoardTrait: Debug + CloneAsBoard + 'static {
    fn move_piece(&mut self, from: Position, to: Position)
    -> Result<Option<PieceType>, ChessError>;
    fn get_piece(&self, position: Position) -> Option<&PieceType>;
    fn get_all_white_pieces(&self) -> Vec<&PieceType>;
    fn get_all_black_pieces(&self) -> Vec<&PieceType>;
    fn is_king_check(&self, color: &Color) -> bool;
    fn can_king_move_safe_position(&self, color: &Color) -> bool;
    fn evaluate(&self, color: &Color) -> i16;
    fn square(&self, position: &Position) -> &Square;
    fn square_mut(&mut self, position: &Position) -> &mut Square;
}

pub trait CloneAsBoard {
    fn clone_as_a(&self) -> Box<dyn BoardTrait>;
}

impl<T: 'static + BoardTrait + Clone> CloneAsBoard for T {
    fn clone_as_a(&self) -> Box<dyn BoardTrait> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
struct Board {
    pub squares: Vec<Square>,
}

impl Board {
    fn fill_white(mut squares: Vec<Square>) -> Vec<Square> {
        squares[0].piece = Some(pieces::PieceType::Rook(
            pieces::Color::White,
            Position::new('a', 1),
        ));
        squares[1].piece = Some(pieces::PieceType::Knight(
            pieces::Color::White,
            Position::new('b', 1),
        ));
        squares[2].piece = Some(pieces::PieceType::Bishop(
            pieces::Color::White,
            Position::new('c', 1),
        ));
        squares[3].piece = Some(pieces::PieceType::Queen(
            pieces::Color::White,
            Position::new('d', 1),
        ));
        squares[4].piece = Some(pieces::PieceType::King(
            pieces::Color::White,
            Position::new('e', 1),
        ));
        squares[5].piece = Some(pieces::PieceType::Bishop(
            pieces::Color::White,
            Position::new('f', 1),
        ));
        squares[6].piece = Some(pieces::PieceType::Knight(
            pieces::Color::White,
            Position::new('g', 1),
        ));
        squares[7].piece = Some(pieces::PieceType::Rook(
            pieces::Color::White,
            Position::new('h', 1),
        ));
        for i in 8..16 {
            squares[i].piece = Some(pieces::PieceType::Pawn(
                pieces::Color::White,
                Position::new((i as i8 - 8 + 97) as u8 as char, 2),
                true,
            ));
        }

        squares
    }

    fn fill_black(mut squares: Vec<Square>) -> Vec<Square> {
        squares[56].piece = Some(pieces::PieceType::Rook(
            pieces::Color::Black,
            Position::new('a', 8),
        ));
        squares[57].piece = Some(pieces::PieceType::Knight(
            pieces::Color::Black,
            Position::new('b', 8),
        ));
        squares[58].piece = Some(pieces::PieceType::Bishop(
            pieces::Color::Black,
            Position::new('c', 8),
        ));
        squares[59].piece = Some(pieces::PieceType::Queen(
            pieces::Color::Black,
            Position::new('d', 8),
        ));
        squares[60].piece = Some(pieces::PieceType::King(
            pieces::Color::Black,
            Position::new('e', 8),
        ));
        squares[61].piece = Some(pieces::PieceType::Bishop(
            pieces::Color::Black,
            Position::new('f', 8),
        ));
        squares[62].piece = Some(pieces::PieceType::Knight(
            pieces::Color::Black,
            Position::new('g', 8),
        ));
        squares[63].piece = Some(pieces::PieceType::Rook(
            pieces::Color::Black,
            Position::new('h', 8),
        ));
        for i in 48..56 {
            squares[i].piece = Some(pieces::PieceType::Pawn(
                pieces::Color::Black,
                Position::new((i as i8 - 48 + 97) as u8 as char, 7),
                true,
            ));
        }

        squares
    }

    fn new_inner() -> Board {
        let mut squares = Board::get_squares();

        let squares = Board::fill_white(squares);
        let squares = Board::fill_black(squares);
        Board { squares }
    }

    fn empty_inner() -> Board {
        let squares = Board::get_squares();

        Board { squares }
    }

    fn get_squares() -> Vec<Square> {
        let mut squares = Vec::new();
        for y in 1..9 {
            let range: Range<u8> = 97..105;
            for x in range {
                squares.push(Square {
                    piece: None,
                    x: x as char,
                    y,
                });
            }
        }

        squares
    }
}

impl BoardTrait for Board {
    fn move_piece(
        &mut self,
        from: Position,
        to: Position,
    ) -> Result<Option<PieceType>, ChessError> {
        let from_index = from.to_index();
        let piece = self.squares[from_index as usize].piece.take();
        if piece.is_none() {
            return Err(ChessError::InvalidMove);
        }
        let board = piece.unwrap().move_to(to, self)?;
        Ok(board)
    }

    fn get_piece(&self, position: Position) -> Option<&PieceType> {
        let index = position.to_index();
        self.squares[index as usize].piece.as_ref()
    }

    fn get_all_white_pieces(&self) -> Vec<&PieceType> {
        let mut pieces = Vec::new();
        for square in &self.squares {
            if let Some(piece) = &square.piece {
                if piece.color() == Color::White {
                    pieces.push(piece);
                }
            }
        }

        pieces
    }

    fn get_all_black_pieces(&self) -> Vec<&PieceType> {
        let mut pieces = Vec::new();
        for square in &self.squares {
            if let Some(piece) = &square.piece {
                if piece.color() == Color::Black {
                    pieces.push(piece);
                }
            }
        }

        pieces
    }

    fn is_king_check(&self, color: &Color) -> bool {
        let pieces = match color {
            Color::White => self.get_all_white_pieces(),
            Color::Black => self.get_all_black_pieces(),
        };

        pieces.iter().any(|piece| {
            if let PieceType::King(_, _) = piece {
                return king::is_check(**piece, self);
            }
            false
        })
    }

    fn can_king_move_safe_position(&self, color: &Color) -> bool {
        let pieces = match color {
            Color::White => self.get_all_white_pieces(),
            Color::Black => self.get_all_black_pieces(),
        };

        pieces.iter().any(|piece| {
            if let PieceType::King(_, _) = piece {
                return king::can_king_move_safe_position(**piece, self);
            }
            false
        })
    }

    fn evaluate(&self, color: &Color) -> i16 {
        let mut score = 0;
        for square in &self.squares {
            if let Some(piece) = &square.piece {
                let value = piece.value() as i16;
                if piece.color() == *color {
                    score += value;
                } else {
                    score -= value;
                }
            }
        }

        score
    }

    fn square_mut(&mut self, position: &Position) -> &mut Square {
        let index = position.to_index();
        &mut self.squares[index as usize]
    }

    fn square(&self, position: &Position) -> &Square {
        let index = position.to_index();
        &self.squares[index as usize]
    }
}

pub fn new_board() -> impl BoardTrait {
    Board::new_inner()
}

pub fn empty_board() -> impl BoardTrait {
    Board::empty_inner()
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_new_board_evealuate() {
        let board = Board::new_inner();
        let white_score = board.evaluate(&Color::White);

        assert_eq!(white_score, 0);
    }

    #[test]
    fn test_white_both_knight_missing() {
        let mut board = Board::new_inner();
        board.squares[1].piece = None;
        board.squares[6].piece = None;

        let white_score = board.evaluate(&Color::White);

        assert_eq!(white_score, -6);
    }

    #[test]
    fn test_only_king_remains() {
        let mut board = Board::new_inner();
        for i in 0..16 {
            if i == 4 {
                continue;
            }
            board.squares[i].piece = None;
        }

        assert_eq!(board.evaluate(&Color::White), -39);
    }

    #[test]
    fn test_board_to_square() {
        let board = Board::new_inner();
        let square = board.square(&Position::from_index(3));
        assert_eq!(square.x, 'd');
        assert_eq!(square.y, 1);
    }
}
