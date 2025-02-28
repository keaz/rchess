use crate::{pieces::Color, Board};

// 1. loop through all pieces on the board
// 2. for each piece, generate all possible moves
// 3. for each move, evaluate the board
// 4. return the best move
pub fn generate_move(color: Color, board: &mut Board) {
    let pices = match color {
        Color::Black => {
            board.get_all_black_pieces()
        },
        Color::White => {
            board.get_all_white_pieces()
        },
    };

    for pice in pices{
    
    }
}
