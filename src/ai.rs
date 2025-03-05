use crate::{
    pieces::{Color, Piece, PieceType},
    Board, Position,
};

// 1. loop through all pieces on the board
// 2. for each piece, generate all possible moves
// 3. for each move, evaluate the board
// 4. return the best move
pub fn generate_move(color: Color, board: &mut Board) -> Option<(&PieceType, Position)> {
    let pieces = match color {
        Color::Black => board.get_all_black_pieces(),
        Color::White => board.get_all_white_pieces(),
    };

    let mut best_score = 0;
    let mut best_move = Option::None;
    for piece in pieces {
        let possible_moves = piece.possible_moves(board);
        for new_position in possible_moves {
            let mut future_board = board.clone();
            let mut future_piece = piece.clone();
            if let Ok(_) = future_piece.move_to(new_position, &mut future_board) {
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
        ai::generate_move,
        pieces::{Color, PieceType},
        Board, Position,
    };

    #[test]
    fn test_generate_move() {
        let mut board = Board::new();
        let best_move = generate_move(Color::White, &mut board);
        assert_eq!(best_move.is_some(), true);
    }
}
