use drawille::{Canvas, Turtle};

fn main() {
    let multi = 200;
    let side_padding = 10;

    // let board_width =  multi;

    // let mut canvas = Canvas::new(multi+(side_padding), multi+(side_padding));
    // canvas.line(side_padding, 0, side_padding, board_width);
    // canvas.line(side_padding, board_width, board_width, board_width);
    // canvas.line(board_width, board_width, board_width, 0);
    // canvas.line(board_width, 0, side_padding, 0);

    // let line_size = board_width / 8;
    // let mut next_line = line_size;
    // while next_line < multi {
    //     canvas.line(0, next_line, board_width, next_line);
    //     canvas.line(next_line, 0, next_line, board_width);
    //     next_line += line_size;
    // }

    // println!("{}", canvas.frame());
    let index: i32 = -7;
    let x = index.abs() as usize;
    println!("{}", x);
}
