use std::io::Write;

use winnow::error::InputError;

use crate::lexer::{Token, token_p};

mod lexer;

fn main() {
    let text = std::fs::read_to_string("./flake.nix").unwrap();
    let mut src = text.as_str();
    while let Ok(tok) = token_p::<_, InputError<_>>(&mut src) {
        print!("{tok} ");
        std::io::stdout().flush().unwrap();
    }
    println!("---------  Rest  ---------");
    println!("{src}");
}
