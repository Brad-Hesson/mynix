use colour::{blue, gray, green, red, white, yellow};
use winnow::{
    LocatingSlice, Parser, error::{EmptyError, ErrMode},
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
    for _ in 0..2u64.pow(12) {
        for tok in p.parse(LocatingSlice::new(src)).unwrap() {
            num += 1;
            match tok.kind {
                lexer::Kind::DocComment => gray!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Interp => red!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::IndentedStrLit => green!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::StrLit => green!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Path => colour::cyan!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Lookup => colour::cyan!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Ident => white!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Punct => yellow!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Int => blue!("{} ", &src[tok.span.start..tok.span.end]),
                lexer::Kind::Float => blue!("{} ", &src[tok.span.start..tok.span.end]),
            }
        }
        break;
    }
    println!("{num}");
}
