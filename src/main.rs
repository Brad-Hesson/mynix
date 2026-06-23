use std::io::Write;

use winnow::{LocatingSlice, Parser, error::InputError};

use crate::lexer::{Token, token_p};

mod lexer;

fn main() {
    let text = std::fs::read_to_string("./nix_lexer_edge_cases.nix").unwrap();
    // let text = std::fs::read_to_string("./flake.nix").unwrap();
    let src = text.as_str();
    // while let Ok(tok) = token_p::<_, InputError<_>>(&mut src) {
    let mut p = token_p::<_, InputError<_>>.with_span();
    let mut num = 0;
    for _ in 0..2u64.pow(13) {
        for tok in p.parse_iter(LocatingSlice::new(src)) {
            let tok = tok.unwrap();
            num += 1;
            print!("{tok:?} ");
            // std::io::stdout().flush().unwrap();
        }
        break;
    }
    println!("{num}");
}