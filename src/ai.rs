use crate::{
    pieces::{Color, Piece},
    Board,
};

// 1. loop through all pieces on the board
// 2. for each piece, generate all possible moves
// 3. for each move, evaluate the board
// 4. return the best move
pub fn generate_move(color: Color, board: &mut Board) {
    let pieces = match color {
        Color::Black => board.get_all_black_pieces(),
        Color::White => board.get_all_white_pieces(),
    };

    let mut best_score = 0;
    for piece in pieces {
        let possible_moves = piece.possible_moves(board);
        for new_position in possible_moves {
            let mut future_board = board.clone();
            let mut future_piece = piece.clone();
            future_piece.move_to(new_position, &mut future_board);
            let score = future_board.evaluate(&color);
            if score > best_score {
                best_score = score;
            }
        }
    }
}
