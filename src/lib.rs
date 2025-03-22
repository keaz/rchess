use std::fmt::Display;

use board::BoardTrait;
use pieces::{Color, PieceType};

pub mod ai;
pub mod board;
pub mod pieces;

#[derive(Debug)]
pub struct Game {
    pub board: Box<dyn BoardTrait>,
    pub white: Player,
    pub black: Player,
}

impl Game {
    pub fn new() -> Self {
        let board = board::new_board();
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
            board: Box::new(board),
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
            let result = game.board.clone_as_a().move_piece(from, to);
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
        if x < 'a' || x > 'h' || y < 1 || y > 8 {
            panic!("Invalid position: {},{}", x, y);
        }
        Position { x, y }
    }

    pub fn to_index(&self) -> i32 {
        let x = self.x as i32 - 97;
        let y = self.y - 1;
        x + (y * 8) as i32
    }

    pub fn from_index(index: i32) -> Self {
        if index < 0 || index > 64 {
            panic!("Invalid index: {}", index);
        }

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
        BoardTrait, Position, board,
        pieces::{Color, PieceType},
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
        let mut board = board::empty_board();
        let black_king = PieceType::King(Color::Black, Position::new('e', 8));
        let white_queen = PieceType::Queen(Color::White, Position::new('f', 7));
        board.square_mut(&Position::new('e', 8)).piece = Some(black_king);
        board.square_mut(&Position::new('f', 7)).piece = Some(white_queen);

        let is_check = board.is_king_check(&Color::Black);
        assert_eq!(is_check, true);
    }

    #[test]
    fn test_king_not_check() {
        let mut board = board::empty_board();
        let king = PieceType::King(Color::White, Position::new('e', 6));
        let black_queen = PieceType::Queen(Color::Black, Position::new('f', 8));
        board.square_mut(&Position::new('e', 6)).piece = Some(king);
        board.square_mut(&Position::new('f', 8)).piece = Some(black_queen);

        let is_check = board.is_king_check(&Color::White);
        assert_eq!(is_check, false);
    }

    #[test]
    #[should_panic]
    fn test_from_index_invalid_lower() {
        let _ = Position::from_index(-1);
    }

    #[test]
    #[should_panic]
    fn test_from_index_invalid_upper() {
        let _ = Position::from_index(64);
    }

    #[test]
    fn test_from_index() {
        let position = Position::from_index(0);
        assert_eq!(position.x, 'a');
        assert_eq!(position.y, 1);

        let position = Position::from_index(63);
        assert_eq!(position.x, 'h');
        assert_eq!(position.y, 8);

        let position = Position::from_index(28);
        assert_eq!(position.x, 'e');
        assert_eq!(position.y, 4);

        let position = Position::from_index(42);
        assert_eq!(position.x, 'c');
        assert_eq!(position.y, 6);

        let position = Position::from_index(3);
        assert_eq!(position.x, 'd');
        assert_eq!(position.y, 1);
    }
}
