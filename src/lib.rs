use pieces::{ChessError, Piece};

pub mod pieces;

#[derive(Debug, Clone)]
pub struct Board {
    pub squares: Vec<Square>,
}

impl Board {

    pub fn new() -> Self {
        let mut squares = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                squares.push(Square {
                    piece: None,
                    x,
                    y,
                });
            }
        }

        let squares = Board::fill_white(squares);
        let squares = Board::fill_black(squares);
        Board {
            squares,
        }
    }

    fn fill_white(mut squares: Vec<Square>) -> Vec<Square> {
        squares[0].piece = Some(Box::new(pieces::rook::Rook::new(pieces::Color::White, Position::new('a', 1))));
        squares[1].piece = Some(Box::new(pieces::knight::Knight::new(pieces::Color::White, Position::new('b', 1))));
        squares[2].piece = Some(Box::new(pieces::bishop::Bishop::new(pieces::Color::White, Position::new('c', 1))));
        squares[3].piece = Some(Box::new(pieces::Queen::new(pieces::Color::White, Position::new('d', 1))));
        squares[4].piece = Some(Box::new(pieces::King::new(pieces::Color::White, Position::new('e', 1))));
        squares[5].piece = Some(Box::new(pieces::bishop::Bishop::new(pieces::Color::White, Position::new('f', 1))));
        squares[6].piece = Some(Box::new(pieces::knight::Knight::new(pieces::Color::White, Position::new('g', 1))));
        squares[7].piece = Some(Box::new(pieces::rook::Rook::new(pieces::Color::White, Position::new('h', 1))));
        for i in 8..16 {
            squares[i].piece = Some(Box::new(pieces::pawn::Pawn::new(pieces::Color::White, Position::new((i as i8 - 8 + 97) as u8 as char, 2))));
        }

        squares
    }

    fn fill_black(mut squares: Vec<Square>) -> Vec<Square> {
        squares[56].piece = Some(Box::new(pieces::rook::Rook::new(pieces::Color::Black, Position::new('a', 8))));
        squares[57].piece = Some(Box::new(pieces::knight::Knight::new(pieces::Color::Black, Position::new('b', 8))));
        squares[58].piece = Some(Box::new(pieces::bishop::Bishop::new(pieces::Color::Black, Position::new('c', 8))));
        squares[59].piece = Some(Box::new(pieces::Queen::new(pieces::Color::Black, Position::new('d', 8))));
        squares[60].piece = Some(Box::new(pieces::King::new(pieces::Color::Black, Position::new('e', 8))));
        squares[61].piece = Some(Box::new(pieces::bishop::Bishop::new(pieces::Color::Black, Position::new('f', 8))));
        squares[62].piece = Some(Box::new(pieces::knight::Knight::new(pieces::Color::Black, Position::new('g', 8))));
        squares[63].piece = Some(Box::new(pieces::rook::Rook::new(pieces::Color::Black, Position::new('h', 8))));
        for i in 48..56 {
            squares[i].piece = Some(Box::new(pieces::pawn::Pawn::new(pieces::Color::Black, Position::new((i as i8 - 48 + 97) as u8 as char, 7))));
        }

        squares
    }

    pub fn move_piece(mut self, from: Position, to: Position) -> Result<Board, ChessError> {
        let from_index = from.to_index();
        let to_index = to.to_index();
        let piece = self.squares[from_index as usize].piece.take();
        if piece.is_none() {
            return Err(ChessError::InvalidMove);
        }
        let board = piece.unwrap().move_to(to, self)?; // Todo
        Ok(board)
    }

    pub fn get_piece(&self, position: Position) -> Option<&Box<dyn Piece>> {
        let index = position.to_index();
        self.squares[index as usize].piece.as_ref()
    }

}

#[derive(Debug)]
pub struct Square {
    pub piece: Option<Box<dyn Piece>>,
    pub x: i32,
    pub y: i32,
}

impl Clone for Square {
    fn clone(&self) -> Self {
        let piece = match &self.piece {
            Some(piece) => Some(piece.clone_as_a()),
            None => None,
        };

        Square {
            piece,
            x: self.x,
            y: self.y,
        }
    }
    
}

#[derive(Debug, Clone, Copy)]
pub struct Position {
    pub x: char,
    pub y: i8,
}

impl Position {
    
    pub fn new(x: char, y: i8) -> Self {
        Position {
            x,
            y,
        }
    }

    pub fn to_index(&self) -> i32 {
        let x = self.x as i32 - 97;
        let y = self.y - 1;
        x + (y * 8) as i32
    }
    
}


#[cfg(test)]
mod test {

    use crate::Position;

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

}