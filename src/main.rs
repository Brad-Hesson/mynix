use winnow::{
    LocatingSlice, Parser,
    error::{EmptyError, ErrMode},
};

use crate::lexer::file_p;

mod lexer;

fn main() {
    let text = std::fs::read_to_string("./nix_lexer_edge_cases.nix").unwrap();
    // let text = std::fs::read_to_string("./flake.nix").unwrap();
    let src = text.as_str();
    // while let Ok(tok) = token_p::<_, InputError<_>>(&mut src) {
    let mut p = file_p::<_, ErrMode<EmptyError>, Vec<_>>;
    let mut num = 0;
    for _ in 0..2u64.pow(12) {
        for tok in p.parse(LocatingSlice::new(src)).unwrap() {
            num += 1;
            let s = tok.as_colored_string(src);
            println!("{:<16}[{s}]", format!("{:?}", tok.kind));
        }
        break;
    }
    println!("{num}");
}
