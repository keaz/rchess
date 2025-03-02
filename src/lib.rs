use std::{fmt::Display, ops::Range};

use board::Board;
use pieces::{king, ChessError, Color, Piece, PieceType};

pub mod ai;
pub mod board;
pub mod pieces;

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

            let captured = result.unwrap();
            match turn {
                Color::Black => {
                    if game.board.is_king_check(&Color::Black) {
                        println!("Invalid move, Black king is in check");
                        continue;
                    }
                }
                Color::White => {
                    if game.board.is_king_check(&Color::White) {
                        println!("Invalid move, White king is in check");
                        continue;
                    }
                }
            }

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
