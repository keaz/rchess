use std::ops::Range;

use crate::{
    pieces::{self, king, ChessError, Color, Piece, PieceType},
    Position, Square,
};

pub const BOARD_SIZE: i32 = 8;
pub const BOARD_SQUARES: i32 = BOARD_SIZE * BOARD_SIZE;

#[derive(Debug, Clone)]
pub struct Board {
    pub squares: Vec<Square>,
}

impl Board {
    pub fn new() -> Self {
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

        let squares = Board::fill_white(squares);
        let squares = Board::fill_black(squares);
        Board { squares }
    }

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

    pub fn empty() -> Self {
        let mut squares = Vec::new();
        for y in 0..8 {
            let range: Range<u8> = 97..105;
            for x in range {
                squares.push(Square {
                    piece: None,
                    x: x as char,
                    y,
                });
            }
        }

        Board { squares }
    }

    pub fn move_piece(
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

    pub fn get_piece(&self, position: Position) -> Option<&PieceType> {
        let index = position.to_index();
        self.squares[index as usize].piece.as_ref()
    }

    pub fn get_all_white_pieces(&self) -> Vec<&PieceType> {
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

    pub fn get_all_black_pieces(&self) -> Vec<&PieceType> {
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

    pub fn is_king_check(&self, color: &Color) -> bool {
        let pieces = match color {
            Color::White => self.get_all_white_pieces(),
            Color::Black => self.get_all_black_pieces(),
        };

        pieces.iter().any(|piece| {
            if let PieceType::King(_, _) = piece {
                return king::is_check(**piece, &self);
            }
            false
        })
    }

    pub fn can_king_move_safe_position(&self, color: &Color) -> bool {
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

    pub fn evaluate(&self, color: &Color) -> u8 {
        let mut score = 0;
        for square in &self.squares {
            if let Some(piece) = &square.piece {
                let value = piece.value();
                if piece.color() == *color {
                    score += value;
                } else {
                    score -= value;
                }
            }
        }

        score
    }
}
