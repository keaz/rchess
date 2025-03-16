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

    use crate::{
        Position, Square,
        ai::generate_move,
        board::BoardTrait,
        pieces::{self, Color, PieceType},
    };

    #[test]
    fn test_generate_move() {
        let mut board = MockBoard::new();
        let best_move = generate_move(Color::White, &mut board);
        assert_eq!(best_move.is_some(), true);
    }

    #[derive(Debug, Clone)]
    struct MockBoard {
        pub squares: Vec<Square>,
    }

    impl MockBoard {
        pub fn new() -> Self {
            let mut squares = Vec::with_capacity(64);
            let mut squares = MockBoard::fill_white(squares);
            let mut squares = MockBoard::fill_black(squares);
            MockBoard { squares }
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
            todo!()
        }

        fn get_all_black_pieces(&self) -> Vec<&PieceType> {
            todo!()
        }

        fn is_king_check(&self, color: &Color) -> bool {
            todo!()
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
