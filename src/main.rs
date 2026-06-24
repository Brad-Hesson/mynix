use winnow::{
    Parser,
    error::{EmptyError, ErrMode},
};

use crate::lexer::file_p;

mod lexer;

fn main() {
    let text = std::fs::read_to_string("./nix_lexer_edge_cases.nix").unwrap();
    // let text = std::fs::read_to_string("./flake.nix").unwrap();
    let src = text.as_str();
    // while let Ok(tok) = token_p::<_, InputError<_>>(&mut src) {
    let mut p = file_p::<_, ErrMode<EmptyError>>;
    let mut num = 0;
    for _ in 0..2u64.pow(10) {
        for tok in p.parse(src).unwrap() {
            num += 1;
            print!("{} ", tok);
        }
        break;
    }
    println!("{num}");
}
