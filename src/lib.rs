use std::{fmt::Display, ops::Range};

use pieces::{king, ChessError, Color, Piece, PieceType};

pub mod ai;
pub mod pieces;

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
        mut self,
        from: Position,
        to: Position,
    ) -> Result<(Board, Option<PieceType>), ChessError> {
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
}

#[derive(Debug)]
pub struct Game {
    pub board: Board,
    pub white: Player,
    pub black: Player,
}

impl Game {
    pub fn new() -> Self {
        let board = Board::new();
        let white = Player {
            color: Color::White,
            moves: Vec::new(),
            captured_pieces: Vec::new(),
        };
        let black = Player {
            color: Color::Black,
            moves: Vec::new(),
            captured_pieces: Vec::new(),
        };

        Game {
            board,
            white,
            black,
        }
    }

    pub fn play(&mut self) {
        let game = self;
        let mut turn = Color::White;
        loop {
            let player = match turn {
                Color::White => &mut game.white,
                Color::Black => &mut game.black,
            };

            let captured = &player.captured_pieces;
            if !captured.is_empty() {
                println!("{} captured pieces: ", turn);
                captured.iter().for_each(|p| {
                    print!("{}, ", p);
                });
                println!("");
            }

            player.moves.iter().for_each(|m| {
                print!("{}, ", m);
            });
            println!("");
            if game.board.is_king_check(&turn) {
                if game.board.can_king_move_safe_position(&turn) {
                    println!("{} king is in checkmate", turn);
                    break;
                }
                println!("{:?} king is in check", turn);
            }

            let mut input = String::new();
            println!("{} turn", player.color);
            println!("Enter move: ");
            std::io::stdin().read_line(&mut input).unwrap();
            let input = input.trim();
            let from = Position::new(
                input.chars().nth(0).unwrap(),
                input.chars().nth(1).unwrap().to_digit(10).unwrap() as i8,
            );
            let to = Position::new(
                input.chars().nth(2).unwrap(),
                input.chars().nth(3).unwrap().to_digit(10).unwrap() as i8,
            );
            let result = game.board.clone().move_piece(from, to);
            if result.is_err() {
                println!("Invalid move");
                continue;
            }

            let (tmp_board, captured) = result.unwrap();
            match turn {
                Color::Black => {
                    if tmp_board.is_king_check(&Color::Black) {
                        println!("Invalid move, Black king is in check");
                        continue;
                    }
                }
                Color::White => {
                    if tmp_board.is_king_check(&Color::White) {
                        println!("Invalid move, White king is in check");
                        continue;
                    }
                }
            }

            game.board = tmp_board;
            player.moves.push(Move { from, to });
            if let Some(captured) = captured {
                player.captured_pieces.push(captured);
            }

            turn = match turn {
                Color::White => Color::Black,
                Color::Black => Color::White,
            };
        }
    }
}

#[derive(Debug, Clone)]
pub struct Player {
    pub color: Color,
    pub moves: Vec<Move>,
    pub captured_pieces: Vec<PieceType>,
}

impl Player {
    pub fn get_total_value(&self) -> u8 {
        self.captured_pieces
            .iter()
            .fold(0, |acc, piece| acc + piece.value())
    }
}

#[derive(Debug, Clone)]
pub struct Move {
    pub from: Position,
    pub to: Position,
}

impl Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.from.x, self.from.y)?;
        write!(f, "{}{}", self.to.x, self.to.y)
    }
}

#[derive(Debug)]
pub struct Square {
    pub piece: Option<PieceType>,
    pub x: char,
    pub y: i8,
}

impl Clone for Square {
    fn clone(&self) -> Self {
        let piece = match &self.piece {
            Some(piece) => Some(piece.clone()),
            None => None,
        };

        Square {
            piece,
            x: self.x,
            y: self.y,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: char,
    pub y: i8,
}

impl Position {
    pub fn new(x: char, y: i8) -> Self {
        Position { x, y }
    }

    pub fn to_index(&self) -> i32 {
        let x = self.x as i32 - 97;
        let y = self.y - 1;
        x + (y * 8) as i32
    }

    pub fn from_index(index: i32) -> Self {
        let x = (index % 8) + 97;
        let y = (index / 8) + 1;
        Position {
            x: x as u8 as char,
            y: y as i8,
        }
    }
}

#[cfg(test)]
mod test {

    use crate::{
        pieces::{Color, PieceType},
        Board, Position,
    };

    #[test]
    fn test_position_to_index() {
        let position = Position::new('a', 1);
        assert_eq!(position.to_index(), 0);
        let position = Position::new('h', 8);
        assert_eq!(position.to_index(), 63);
        let position = Position::new('e', 4);
        assert_eq!(position.to_index(), 28);
        let position = Position::new('c', 6);
        assert_eq!(position.to_index(), 42);
    }

    #[test]
    fn test_black_king_check() {
        let mut board = Board::empty();
        let black_king = PieceType::King(Color::Black, Position::new('e', 8));
        let white_queen = PieceType::Queen(Color::White, Position::new('f', 7));
        board.squares[Position::new('e', 8).to_index() as usize].piece = Some(black_king);
        board.squares[Position::new('f', 7).to_index() as usize].piece = Some(white_queen);

        let is_check = board.is_king_check(&Color::Black);
        assert_eq!(is_check, true);
    }

    #[test]
    fn test_king_not_check() {
        let mut board = Board::empty();
        let king = PieceType::King(Color::White, Position::new('e', 6));
        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 8));
        board.squares[Position::new('e', 6).to_index() as usize].piece = Some(king);
        board.squares[Position::new('f', 8).to_index() as usize].piece = Some(black_queen);

        let is_check = board.is_king_check(&Color::White);
        assert_eq!(is_check, false);
    }
}
