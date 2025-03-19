use crate::{
    Position,
    board::BoardTrait,
    pieces::{Color, Piece, PieceType},
};

// 1. loop through all pieces on the board
// 2. for each piece, generate all possible moves
// 3. for each move, evaluate the board
// 4. return the best move
pub fn generate_move(color: Color, board: &dyn BoardTrait) -> Option<(&PieceType, Position)> {
    let pieces = match color {
        Color::Black => board.get_all_black_pieces(),
        Color::White => board.get_all_white_pieces(),
    };

    let mut best_score = 0;
    let mut best_move = Option::None;
    for piece in pieces {
        let possible_moves = piece.possible_moves(board);
        for new_position in possible_moves {
            let mut cloned_board = board.clone_as_a();
            let future_board = cloned_board.as_mut();

            let mut future_piece = piece.clone();
            if let Ok(_) = future_piece.move_to(new_position, future_board) {
                let score = future_board.evaluate(&color);
                if score > best_score {
                    best_score = score;
                    best_move = Option::Some((piece, new_position));
                }
            }
        }
    }

    best_move
}

#[cfg(test)]
mod test {

    use std::ops::Range;

    use crate::{
        Position, Square,
        ai::generate_move,
        board::{self, BoardTrait},
        pieces::{self, Color, PieceType},
    };

    #[test]
    fn test_generate_move() {
        let mut board = board::new_board();
        let best_move = generate_move(Color::White, &mut board);
        assert_eq!(best_move.is_some(), true);
    }

    #[derive(Debug, Clone)]
    struct MockBoard {
        pub white: Vec<Square>,
        pub black: Vec<Square>,
        pub is_king_check: bool,
    }

    impl MockBoard {
        pub fn new() -> Self {
            let white = MockBoard::fill_white();
            let black = MockBoard::fill_black();
            MockBoard {
                white,
                black,
                is_king_check: false,
            }
        }
        fn fill_white() -> Vec<Square> {
            let mut squares = Vec::new();
            for y in 1..3 {
                let range: Range<u8> = 97..105;
                for x in range {
                    squares.push(Square {
                        piece: None,
                        x: x as char,
                        y,
                    });
                }
            }
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

        fn fill_black() -> Vec<Square> {
            let mut squares = Vec::new();
            for y in 7..9 {
                let range: Range<u8> = 97..105;
                for x in range {
                    squares.push(Square {
                        piece: None,
                        x: x as char,
                        y,
                    });
                }
            }
            squares[8].piece = Some(pieces::PieceType::Rook(
                pieces::Color::Black,
                Position::new('a', 8),
            ));
            squares[9].piece = Some(pieces::PieceType::Knight(
                pieces::Color::Black,
                Position::new('b', 8),
            ));
            squares[10].piece = Some(pieces::PieceType::Bishop(
                pieces::Color::Black,
                Position::new('c', 8),
            ));
            squares[11].piece = Some(pieces::PieceType::Queen(
                pieces::Color::Black,
                Position::new('d', 8),
            ));
            squares[12].piece = Some(pieces::PieceType::King(
                pieces::Color::Black,
                Position::new('e', 8),
            ));
            squares[13].piece = Some(pieces::PieceType::Bishop(
                pieces::Color::Black,
                Position::new('f', 8),
            ));
            squares[14].piece = Some(pieces::PieceType::Knight(
                pieces::Color::Black,
                Position::new('g', 8),
            ));
            squares[15].piece = Some(pieces::PieceType::Rook(
                pieces::Color::Black,
                Position::new('h', 8),
            ));
            for i in 0..8 {
                squares[i].piece = Some(pieces::PieceType::Pawn(
                    pieces::Color::Black,
                    Position::new((i as i8 + 97) as u8 as char, 7),
                    true,
                ));
            }

            squares
        }
    }

    impl BoardTrait for MockBoard {
        fn move_piece(
            &mut self,
            from: Position,
            to: Position,
        ) -> Result<Option<PieceType>, crate::pieces::ChessError> {
            todo!()
        }

        fn get_piece(&self, position: Position) -> Option<&PieceType> {
            todo!()
        }

        fn get_all_white_pieces(&self) -> Vec<&PieceType> {
            self.white
                .iter()
                .filter_map(|square| square.piece.as_ref())
                .collect()
        }

        fn get_all_black_pieces(&self) -> Vec<&PieceType> {
            self.black
                .iter()
                .filter_map(|square| square.piece.as_ref())
                .collect()
        }

        fn is_king_check(&self, color: &Color) -> bool {
            self.is_king_check
        }

        fn can_king_move_safe_position(&self, color: &Color) -> bool {
            todo!()
        }

        fn evaluate(&self, color: &Color) -> i16 {
            todo!()
        }

        fn square(&self, position: &Position) -> &crate::Square {
            todo!()
        }

        fn square_mut(&mut self, position: &Position) -> &mut crate::Square {
            todo!()
        }
    }
}
