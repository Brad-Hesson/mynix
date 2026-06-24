use std::fmt::{self, Display};

use strum::{EnumCount, EnumIter, EnumProperty, IntoEnumIterator, VariantArray};
use winnow::{
    Result,
    ascii::{Caseless, multispace1, till_line_ending},
    combinator::{
        alt, cut_err, delimited, eof, not, opt, peek, preceded, repeat, terminated, trace,
    },
    error::{ModalError, ParserError},
    prelude::*,
    stream::{AsBStr, AsChar, Compare, FindSlice, Location, ParseSlice, Stream, StreamIsPartial},
    token::{any, literal, one_of, take_until, take_while},
};

// fn alt_iter<I, O, E>(it: impl IntoIterator<Item = impl Parser<I, O, E>>) -> impl Parser<I, O, E>
// where
//     I: Stream,
//     E: ParserError<I>,
// {
//     let mut it = it.into_iter();
//     trace("alt_iter", move |input: &mut I| {
//         let mut error: Option<E> = None;
//         let start = input.checkpoint();
//         for mut branch in &mut it {
//             input.reset(&start);
//             match branch.parse_next(input) {
//                 Err(e) if e.is_backtrack() => {
//                     error = match error {
//                         Some(error) => Some(error.or(e)),
//                         None => Some(e),
//                     };
//                 }
//                 res => return res,
//             }
//         }
//         match error {
//             Some(e) => Err(e.append(input, &start)),
//             None => Err(ParserError::from_input(input)),
//         }
//     })
// }

fn punct_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + Location,
    E: ParserError<I>,
    I::Slice: AsBStr,
{
    let span = (|input: &mut I| {
        let max_len = input.eof_offset();
        let slice = input.peek_slice(3.min(max_len));
        let bytes = slice.as_bstr();
        for i in 0..Punct::COUNT {
            let len = Punct::STRS[i].len();
            if len > max_len {
                continue;
            }
            if bytes[..len] == *Punct::STRS[i].as_bytes() {
                input.next_slice(len);
                return Ok(Punct::VARIANTS[i]);
            }
        }
        Err(ParserError::from_input(input))
    })
    .span()
    .parse_next(input)?;
    Ok(Token {
        kind: Kind::Punct,
        span: span.into(),
    })
}

fn singleline_comment_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<(char, char)>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
{
    preceded("#", till_line_ending).parse_next(input)
}

fn doc_comment_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<&'static str> + Location,
    E: ParserError<I> + ModalError,
{
    preceded("/**", cut_err(terminated(take_until(0.., "*/"), "*/")))
        .span()
        .map(|span| Token {
            kind: Kind::DocComment,
            span: span.into(),
        })
        .parse_next(input)
}

fn multiline_comment_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<&'static str>,
    E: ParserError<I> + ModalError,
{
    preceded(
        ("/*", not("*")),
        cut_err(terminated(take_until(0.., "*/"), "*/")),
    )
    .parse_next(input)
}

fn ident_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + Location,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
{
    fn first<C: AsChar>(c: C) -> bool {
        let c = c.as_char();
        c.is_alpha() || c == '_'
    }
    fn rest<C: AsChar>(c: C) -> bool {
        let c = c.as_char();
        c.is_alphanum() || ['_', '-', '\''].contains(&c)
    }
    (one_of(first), take_while(0.., rest))
        .span()
        .map(|span| Token {
            kind: Kind::Ident,
            span: span.into(),
        })
        .parse_next(input)
}

fn strlit_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    (|input: &mut I| {
        '"'.parse_next(input)?;
        let ignore_p = alt((('\\', any).void(), "$$".void()));
        let end_p = alt(("\"", "${"));
        let mut str_part_p = escaped_take_until(ignore_p, end_p);
        loop {
            str_part_p.parse_next(input)?;
            if opt(interp_p).parse_next(input)?.is_none() {
                break;
            }
        }
        '"'.parse_next(input)?;
        Ok(())
    })
    .span()
    .map(|span| Token {
        kind: Kind::StrLit,
        span: span.into(),
    })
    .parse_next(input)
}

fn indented_strlit_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    (|input: &mut I| {
        "''".parse_next(input)?;
        let ignore_p = alt((
            ("''\\", any).void(),
            "''$".void(),
            "'''".void(),
            "$$".void(),
        ));
        let end_p = alt(("''", "${"));
        let mut str_part_p = escaped_take_until(ignore_p, end_p);
        loop {
            str_part_p.parse_next(input)?;
            if opt(interp_p).parse_next(input)?.is_none() {
                break;
            }
        }
        "''".parse_next(input)?;
        Ok(())
    })
    .span()
    .map(|span| Token {
        kind: Kind::IndentedStrLit,
        span: span.into(),
    })
    .parse_next(input)
}

fn escaped_take_until<I, E, O1, O2>(
    ignore: impl Parser<I, O1, E>,
    stop: impl Parser<I, O2, E>,
) -> impl Parser<I, I::Slice, E>
where
    I: Stream + StreamIsPartial,
    E: ParserError<I>,
{
    let mut skip = trace("ignore", repeat::<_, _, (), _, _>(.., ignore).void());
    let mut check = trace("check", peek(opt(stop)));
    let inner = move |input: &mut I| {
        loop {
            skip.parse_next(input)?;
            if check.parse_next(input)?.is_some() {
                break;
            }
            any(input)?;
        }
        Ok(())
    };
    inner.take()
}

fn int_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<char> + Location,
    E: ParserError<I>,
    I::Token: AsChar,
    I::Slice: ParseSlice<i64>,
{
    tokenize(Kind::Int, take_while(1.., AsChar::is_dec_digit)).parse_next(input)
}

fn tokenize<I, O, E>(kind: Kind, parser: impl Parser<I, O, E>) -> impl Parser<I, Token, E>
where
    I: Stream + Location,
{
    parser.span().map(move |span| Token {
        kind,
        span: span.into(),
    })
}

fn float_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<char> + Location,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: ParseSlice<f64>,
{
    let digits_p = |x: usize| take_while(x.., AsChar::is_dec_digit);
    fn nonzero<C: AsChar>(c: C) -> bool {
        matches!(c.as_char(), '1'..='9')
    }
    let exp_p = || (one_of(('e', 'E')), opt(one_of(('+', '-'))), digits_p(1));
    tokenize(
        Kind::Float,
        alt((
            (one_of(nonzero), digits_p(0), '.', digits_p(0), opt(exp_p())).void(),
            (opt('0'), '.', digits_p(1), opt(exp_p())).void(),
            (digits_p(1), exp_p()).void(),
        )),
    )
    .parse_next(input)
}

fn number_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<char> + Location,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: ParseSlice<f64> + ParseSlice<i64>,
{
    alt((float_p, int_p)).parse_next(input)
}

fn path_char<C: AsChar>(c: C) -> bool {
    let c = c.as_char();
    c.is_alphanum() || ['.', '-', '_', '+'].contains(&c)
}

fn path_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    tokenize(Kind::Path, |input: &mut I| {
        let pchars_p = || take_while(1.., path_char);
        (
            opt(alt((pchars_p().void(), '~'.void()))),
            '/',
            repeat::<_, _, (), _, _>(0.., (pchars_p(), '/')),
            alt((pchars_p().void(), peek('$').void())),
        )
            .take()
            .parse_next(input)?;
        while opt(interp_p).parse_next(input)?.is_some() {
            let mut rest = opt(take_while(1.., (path_char, '/')));
            rest.parse_next(input)?;
        }
        Ok(())
    })
    .parse_next(input)
}

fn lookup_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream + StreamIsPartial + Compare<char> + Location,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr,
{
    tokenize(
        Kind::Lookup,
        delimited('<', take_while(1.., (path_char, '/')), '>'),
    )
    .parse_next(input)
}

fn blank_p<I, E>(input: &mut I) -> Result<(), E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
{
    repeat::<_, _, (), _, _>(
        0..,
        alt((
            multispace1,
            trace("singleline_comment_p", singleline_comment_p),
            trace("multiline_comment_p", multiline_comment_p),
        )),
    )
    .parse_next(input)
}

fn token_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    alt((
        trace("path_p", path_p),
        trace("ident_p", ident_p),
        trace("strlit_p", strlit_p),
        trace("indented_strlit_p", indented_strlit_p),
        trace("interp_p", interp_p),
        trace("lookup_p", lookup_p),
        trace("doc_comment_p", doc_comment_p),
        trace("number_p", number_p),
        trace("punct_p", punct_p),
    ))
    .parse_next(input)
}

pub fn file_p<I, E>(input: &mut I) -> Result<Vec<Token>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    delimited(blank_p, repeat(0.., terminated(token_p, blank_p)), eof).parse_next(input)
}

fn interp_p<I, E>(input: &mut I) -> Result<Token, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>
        + Location,
    E: ParserError<I> + ModalError,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    tokenize(Kind::Interp, |input: &mut I| {
        "${".parse_next(input)?;
        let mut depth = 0;
        let mut tokens = Vec::new();
        loop {
            blank_p.parse_next(input)?;
            let (tok, slice) = token_p.with_taken().parse_next(input)?;
            match &tok.kind {
                Kind::Punct => {
                    let punct = Punct::STRS
                        .iter()
                        .zip(Punct::VARIANTS)
                        .find_map(|(s, var)| (s.as_bytes() == slice.as_bstr()).then_some(*var))
                        .unwrap();
                    match punct {
                        Punct::OpenCurl => depth += 1,
                        Punct::CloseCurl => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            tokens.push(tok);
        }
        Ok(tokens)
    })
    .parse_next(input)
}

const fn lit(lit: &str) -> u32 {
    let bytes = lit.as_bytes();
    assert!(bytes.len() < 4);
    let mut buf = [0u8; 4];
    let mut i = 0;
    while i < bytes.len() {
        buf[i] = bytes[i];
        i += 1;
    }
    buf[3] = lit.len() as u8;
    u32::from_ne_bytes(buf)
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantArray, EnumCount)]
pub enum Punct {
    Semicolon = lit(";"),
    OpenCurl = lit("{"),
    CloseCurl = lit("}"),
    OpenParen = lit("("),
    CloseParen = lit(")"),
    Ellipses = lit("..."),
    Dot = lit("."),
    EqEq = lit("=="),
    Eq = lit("="),
    Comma = lit(","),
    OpenSquare = lit("["),
    CloseSquare = lit("]"),
    Colon = lit(":"),
    DoubleSlash = lit("//"),
    Slash = lit("/"),
    PlusPlus = lit("++"),
    Plus = lit("+"),
    RightArrow = lit("->"),
    Minus = lit("-"),
    Mul = lit("*"),
    At = lit("@"),
    Question = lit("?"),
    LessEq = lit("<="),
    Less = lit("<"),
    GreaterEq = lit(">="),
    Greater = lit(">"),
    BangEq = lit("!="),
    Bang = lit("!"),
    LogicalAnd = lit("&&"),
    LogicalOr = lit("||"),
}
impl Punct {
    const STRS: [&'static str; Self::COUNT] = const {
        let mut slice = [std::mem::MaybeUninit::<&str>::uninit(); Self::COUNT];
        let mut i = 0;
        while i < Self::COUNT {
            slice[i].write(Self::VARIANTS[i].as_str());
            i += 1;
        }
        unsafe { std::mem::transmute(slice) }
    };
    pub const fn as_str(&self) -> &str {
        let len = self.len();
        let ptr = std::ptr::from_ref(self).cast::<u8>();
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        unsafe { std::str::from_utf8_unchecked(slice) }
    }
    pub const fn len(self) -> usize {
        let bytes = u32::to_ne_bytes(self as u32);
        bytes[3] as usize
    }
}

#[test]
fn feature() {
    for i in 0..Punct::COUNT {
        println!("{:?} => `{}`", Punct::VARIANTS[i], Punct::STRS[i]);
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Kind {
    DocComment,
    Interp,
    IndentedStrLit,
    StrLit,
    Path,
    Lookup,
    Ident,
    Punct,
    Int,
    Float,
}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl From<std::ops::Range<usize>> for Span {
    fn from(value: std::ops::Range<usize>) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}
// impl Token{
//     fn new
// }

// #[derive(Debug, PartialEq)]
// pub enum InterpPart<I: Stream> {
//     Str(I::Slice),
//     Interp(Vec<Token<I>>),
// }

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use winnow::error::{EmptyError, ErrMode};

//     type Tok = Token<&'static str>;
//     type Part = InterpPart<&'static str>;

//     fn lex(src: &'static str) -> std::result::Result<Vec<Tok>, String> {
//         let input = src;

//         file_p::<_, ErrMode<EmptyError>>
//             .parse(input)
//             .map_err(|_| String::new())
//     }

//     #[track_caller]
//     fn assert_tokens(src: &'static str, expected: Vec<Tok>) {
//         let actual = match lex(src) {
//             Ok(actual) => actual,
//             Err(e) => panic!("expected lexer success\nsource:\n{src}\nerror:\n{e}"),
//         };

//         assert_eq!(actual, expected, "source:\n{src}");
//     }

//     #[track_caller]
//     fn assert_rejects(src: &'static str) {
//         match lex(src) {
//             Ok(tokens) => {
//                 panic!("expected lexer failure\nsource:\n{src}\ntokens:\n{tokens:#?}");
//             }
//             Err(_) => {}
//         }
//     }

//     fn id(s: &'static str) -> Tok {
//         Token::Ident(s)
//     }

//     fn int(n: i64) -> Tok {
//         Token::Int(n)
//     }

//     fn float(x: f64) -> Tok {
//         Token::Float(x)
//     }

//     fn p(punct: Punct) -> Tok {
//         Token::Punct(punct)
//     }

//     fn str_lit(parts: Vec<Part>) -> Tok {
//         Token::StrLit(parts)
//     }

//     fn ind_str(parts: Vec<Part>) -> Tok {
//         Token::IndentedStrLit(parts)
//     }

//     fn path(parts: Vec<Part>) -> Tok {
//         Token::Path(parts)
//     }

//     fn interp(tokens: Vec<Tok>) -> Part {
//         InterpPart::Interp(tokens)
//     }

//     fn part(s: &'static str) -> Part {
//         InterpPart::Str(s)
//     }

//     // ---------------------------------------------------------------------
//     // Trivia and comments
//     // ---------------------------------------------------------------------

//     #[test]
//     fn empty_input_tokens() {
//         assert_tokens("", vec![]);
//     }

//     #[test]
//     fn whitespace_only_tokens() {
//         assert_tokens(" \t\n\r\n  ", vec![]);
//     }

//     #[test]
//     fn singleline_comment_only_tokens() {
//         assert_tokens("# hello world", vec![]);
//     }

//     #[test]
//     fn singleline_comment_before_ident_tokens() {
//         assert_tokens("# hello\nx", vec![id("x")]);
//     }

//     #[test]
//     fn singleline_comment_after_ident_tokens() {
//         assert_tokens("x # hello\n y", vec![id("x"), id("y")]);
//     }

//     #[test]
//     fn multiline_comment_only_tokens() {
//         assert_tokens("/* hello world */", vec![]);
//     }

//     #[test]
//     fn multiline_comment_between_tokens() {
//         assert_tokens("x /* hello */ y", vec![id("x"), id("y")]);
//     }

//     #[test]
//     fn doc_comment_token() {
//         assert_tokens(
//             "/** hello world */",
//             vec![Token::DocComment(" hello world ")],
//         );
//     }

//     #[test]
//     fn doc_comment_before_ident_token() {
//         assert_tokens("/** docs */ x", vec![Token::DocComment(" docs "), id("x")]);
//     }

//     // ---------------------------------------------------------------------
//     // Identifiers and keyword-like tokens
//     // ---------------------------------------------------------------------

//     #[test]
//     fn simple_ident_token() {
//         assert_tokens("abc", vec![id("abc")]);
//     }

//     #[test]
//     fn underscore_ident_token() {
//         assert_tokens("_abc", vec![id("_abc")]);
//     }

//     #[test]
//     fn ident_with_digits_token() {
//         assert_tokens("abc123", vec![id("abc123")]);
//     }

//     #[test]
//     fn ident_with_dash_token() {
//         assert_tokens("foo-bar", vec![id("foo-bar")]);
//     }

//     #[test]
//     fn ident_with_apostrophe_token() {
//         assert_tokens("foo'", vec![id("foo'")]);
//     }

//     #[test]
//     fn ident_with_multiple_apostrophes_token() {
//         assert_tokens("foo''", vec![id("foo''")]);
//     }

//     #[test]
//     fn keyword_words_are_ident_tokens_for_lexer() {
//         assert_tokens(
//             "if then else assert with let in rec inherit or",
//             vec![
//                 id("if"),
//                 id("then"),
//                 id("else"),
//                 id("assert"),
//                 id("with"),
//                 id("let"),
//                 id("in"),
//                 id("rec"),
//                 id("inherit"),
//                 id("or"),
//             ],
//         );
//     }

//     #[test]
//     fn attr_selection_tokens() {
//         assert_tokens(
//             "pkgs.lib.strings",
//             vec![
//                 id("pkgs"),
//                 p(Punct::Dot),
//                 id("lib"),
//                 p(Punct::Dot),
//                 id("strings"),
//             ],
//         );
//     }

//     #[test]
//     fn builder_dot_sh_is_not_path_token() {
//         assert_tokens("builder.sh", vec![id("builder"), p(Punct::Dot), id("sh")]);
//     }

//     // ---------------------------------------------------------------------
//     // Integers and floats
//     // ---------------------------------------------------------------------

//     #[test]
//     fn int_zero_token() {
//         assert_tokens("0", vec![int(0)]);
//     }

//     #[test]
//     fn int_decimal_token() {
//         assert_tokens("12345", vec![int(12345)]);
//     }

//     #[test]
//     fn int_leading_zeroes_token() {
//         assert_tokens("000123", vec![int(123)]);
//     }

//     #[test]
//     fn int_i64_max_token() {
//         assert_tokens("9223372036854775807", vec![int(9223372036854775807)]);
//     }

//     #[test]
//     fn float_fraction_token() {
//         assert_tokens("123.5", vec![float(123.5)]);
//     }

//     #[test]
//     fn float_with_exponent_token() {
//         assert_tokens("1.0e2", vec![float(100.0)]);
//     }

//     #[test]
//     fn float_with_positive_exponent_token() {
//         assert_tokens("1.0e+2", vec![float(100.0)]);
//     }

//     #[test]
//     fn float_with_negative_exponent_token() {
//         assert_tokens("1.0e-2", vec![float(0.01)]);
//     }

//     #[test]
//     fn int_plus_int_not_greedy_number_token() {
//         assert_tokens("1+2", vec![int(1), p(Punct::Plus), int(2)]);
//     }

//     #[test]
//     fn int_minus_int_not_greedy_number_token() {
//         assert_tokens("1-2", vec![int(1), p(Punct::Minus), int(2)]);
//     }

//     #[test]
//     fn float_plus_float_not_greedy_number_token() {
//         assert_tokens("1.0+2.0", vec![float(1.0), p(Punct::Plus), float(2.0)]);
//     }

//     #[test]
//     fn float_minus_float_not_greedy_number_token() {
//         assert_tokens("1.0-2.0", vec![float(1.0), p(Punct::Minus), float(2.0)]);
//     }

//     #[test]
//     fn exponent_float_plus_int_not_greedy_number_token() {
//         assert_tokens("1e+2+3", vec![float(100.0), p(Punct::Plus), int(3)]);
//     }

//     #[test]
//     fn arithmetic_expr_number_tokens() {
//         assert_tokens(
//             "x = 1+2;",
//             vec![
//                 id("x"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Plus),
//                 int(2),
//                 p(Punct::Semicolon),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Punctuation and operators
//     // ---------------------------------------------------------------------

//     #[test]
//     fn semicolon_token() {
//         assert_tokens(";", vec![p(Punct::Semicolon)]);
//     }

//     #[test]
//     fn curl_tokens() {
//         assert_tokens("{ }", vec![p(Punct::OpenCurl), p(Punct::CloseCurl)]);
//     }

//     #[test]
//     fn paren_tokens() {
//         assert_tokens("( )", vec![p(Punct::OpenParen), p(Punct::CloseParen)]);
//     }

//     #[test]
//     fn square_tokens() {
//         assert_tokens("[ ]", vec![p(Punct::OpenSquare), p(Punct::CloseSquare)]);
//     }

//     #[test]
//     fn ellipses_token_not_three_dots() {
//         assert_tokens("...", vec![p(Punct::Ellipses)]);
//     }

//     #[test]
//     fn dot_token() {
//         assert_tokens(".", vec![p(Punct::Dot)]);
//     }

//     #[test]
//     fn eqeq_token_not_two_eqs() {
//         assert_tokens("==", vec![p(Punct::EqEq)]);
//     }

//     #[test]
//     fn eq_token() {
//         assert_tokens("=", vec![p(Punct::Eq)]);
//     }

//     #[test]
//     fn comma_colon_tokens() {
//         assert_tokens(",", vec![p(Punct::Comma)]);
//         assert_tokens(":", vec![p(Punct::Colon)]);
//     }

//     #[test]
//     fn slash_tokens() {
//         assert_tokens("/", vec![p(Punct::Slash)]);
//         assert_tokens("//", vec![p(Punct::DoubleSlash)]);
//     }

//     #[test]
//     fn plus_tokens() {
//         assert_tokens("+", vec![p(Punct::Plus)]);
//         assert_tokens("++", vec![p(Punct::PlusPlus)]);
//     }

//     #[test]
//     fn arrow_and_minus_tokens() {
//         assert_tokens("-", vec![p(Punct::Minus)]);
//         assert_tokens("->", vec![p(Punct::RightArrow)]);
//     }

//     #[test]
//     fn mul_token() {
//         assert_tokens("*", vec![p(Punct::Mul)]);
//     }

//     #[test]
//     fn at_and_question_tokens() {
//         assert_tokens("@", vec![p(Punct::At)]);
//         assert_tokens("?", vec![p(Punct::Question)]);
//     }

//     #[test]
//     fn comparison_operator_tokens() {
//         assert_tokens(
//             "< <= > >= == !=",
//             vec![
//                 p(Punct::Less),
//                 p(Punct::LessEq),
//                 p(Punct::Greater),
//                 p(Punct::GreaterEq),
//                 p(Punct::EqEq),
//                 p(Punct::BangEq),
//             ],
//         );
//     }

//     #[test]
//     fn logical_operator_tokens() {
//         assert_tokens(
//             "! && ||",
//             vec![p(Punct::Bang), p(Punct::LogicalAnd), p(Punct::LogicalOr)],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Double-quoted strings
//     // ---------------------------------------------------------------------

//     #[test]
//     fn double_quoted_empty_string_token() {
//         assert_tokens("\"\"", vec![str_lit(vec![])]);
//     }

//     #[test]
//     fn double_quoted_simple_string_token() {
//         assert_tokens("\"hello\"", vec![str_lit(vec![part("hello")])]);
//     }

//     #[test]
//     fn double_quoted_string_with_spaces_token() {
//         assert_tokens("\"hello world\"", vec![str_lit(vec![part("hello world")])]);
//     }

//     #[test]
//     fn double_quoted_multiline_string_token() {
//         assert_tokens(
//             "\"hello\nworld\"",
//             vec![str_lit(vec![part("hello\nworld")])],
//         );
//     }

//     #[test]
//     fn double_quoted_escaped_quote_token() {
//         assert_tokens(r#""quote: \"""#, vec![str_lit(vec![part(r#"quote: \""#)])]);
//     }

//     #[test]
//     fn double_quoted_escaped_backslash_token() {
//         assert_tokens(r#""slash: \\""#, vec![str_lit(vec![part(r#"slash: \\"#)])]);
//     }

//     #[test]
//     fn double_quoted_escape_n_token() {
//         assert_tokens(
//             r#""hello\nworld""#,
//             vec![str_lit(vec![part(r#"hello\nworld"#)])],
//         );
//     }

//     #[test]
//     fn double_quoted_escape_t_token() {
//         assert_tokens(
//             r#""hello\tworld""#,
//             vec![str_lit(vec![part(r#"hello\tworld"#)])],
//         );
//     }

//     #[test]
//     fn double_quoted_unknown_escape_token() {
//         assert_tokens(r#""\x \q \.""#, vec![str_lit(vec![part(r#"\x \q \."#)])]);
//     }

//     #[test]
//     fn double_quoted_literal_dollar_token() {
//         assert_tokens(r#""$out/bin""#, vec![str_lit(vec![part("$out/bin")])]);
//     }

//     #[test]
//     fn double_quoted_escaped_interpolation_start_token() {
//         assert_tokens(
//             r#""\${not_interp}""#,
//             vec![str_lit(vec![part(r#"\${not_interp}"#)])],
//         );
//     }

//     #[test]
//     fn double_quoted_double_dollar_before_brace_token() {
//         assert_tokens(
//             r#""$${not_interp}""#,
//             vec![str_lit(vec![part("$${not_interp}")])],
//         );
//     }

//     #[test]
//     fn double_quoted_simple_interpolation_token() {
//         assert_tokens(
//             r#""hello ${name}""#,
//             vec![str_lit(vec![part("hello "), interp(vec![id("name")])])],
//         );
//     }

//     #[test]
//     fn double_quoted_sequential_interp() {
//         assert_tokens(
//             r#""hello ${fname}${lname}""#,
//             vec![str_lit(vec![
//                 part("hello "),
//                 interp(vec![id("fname")]),
//                 interp(vec![id("lname")]),
//             ])],
//         );
//     }

//     #[test]
//     fn double_quoted_expr_interpolation_token() {
//         assert_tokens(
//             r#""value = ${x + y}""#,
//             vec![str_lit(vec![
//                 part("value = "),
//                 interp(vec![id("x"), p(Punct::Plus), id("y")]),
//             ])],
//         );
//     }

//     #[test]
//     fn double_quoted_attr_interpolation_token() {
//         assert_tokens(
//             r#""${pkgs.hello.name}""#,
//             vec![str_lit(vec![interp(vec![
//                 id("pkgs"),
//                 p(Punct::Dot),
//                 id("hello"),
//                 p(Punct::Dot),
//                 id("name"),
//             ])])],
//         );
//     }

//     #[test]
//     fn double_quoted_multiple_interpolations_token() {
//         assert_tokens(
//             r#""${a}-${b}-${c}""#,
//             vec![str_lit(vec![
//                 interp(vec![id("a")]),
//                 part("-"),
//                 interp(vec![id("b")]),
//                 part("-"),
//                 interp(vec![id("c")]),
//             ])],
//         );
//     }

//     #[test]
//     fn double_quoted_path_inside_interpolation_token() {
//         assert_tokens(
//             r#""${./foo/bar.nix}""#,
//             vec![str_lit(vec![interp(vec![path(vec![part(
//                 "./foo/bar.nix",
//             )])])])],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Indented strings
//     // ---------------------------------------------------------------------

//     #[test]
//     fn indented_empty_string_token() {
//         assert_tokens("''''", vec![ind_str(vec![])]);
//     }

//     #[test]
//     fn indented_simple_string_token() {
//         assert_tokens("''hello''", vec![ind_str(vec![part("hello")])]);
//     }

//     #[test]
//     fn indented_multiline_string_token() {
//         assert_tokens(
//             "''\n  hello\n  world\n''",
//             vec![ind_str(vec![part("\n  hello\n  world\n")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_double_quotes_token() {
//         assert_tokens(
//             "''echo \"hello\"''",
//             vec![ind_str(vec![part("echo \"hello\"")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_backslash_token() {
//         assert_tokens(
//             "''C:\\path\\file''",
//             vec![ind_str(vec![part("C:\\path\\file")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_literal_dollar_token() {
//         assert_tokens("''$out/bin''", vec![ind_str(vec![part("$out/bin")])]);
//     }

//     #[test]
//     fn indented_string_with_escaped_dollar_token() {
//         assert_tokens(
//             "''echo ''$PATH''",
//             vec![ind_str(vec![part("echo ''$PATH")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_escaped_interpolation_start_token() {
//         assert_tokens(
//             "''echo ''${not_interp}''",
//             vec![ind_str(vec![part("echo ''${not_interp}")])],
//         );
//     }

//     #[test]
//     fn indented_string_sequential_interp() {
//         assert_tokens(
//             r#"''hello ${fname}${lname}''"#,
//             vec![ind_str(vec![
//                 part("hello "),
//                 interp(vec![id("fname")]),
//                 interp(vec![id("lname")]),
//             ])],
//         );
//     }

//     #[test]
//     fn indented_string_with_double_dollar_before_brace_token() {
//         assert_tokens(
//             "''echo $${not_interp}''",
//             vec![ind_str(vec![part("echo $${not_interp}")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_escaped_two_single_quotes_token() {
//         assert_tokens(
//             "''can write ''' inside''",
//             vec![ind_str(vec![part("can write ''' inside")])],
//         );
//     }

//     #[test]
//     fn indented_string_with_simple_interpolation_token() {
//         assert_tokens(
//             "''hello ${name}''",
//             vec![ind_str(vec![part("hello "), interp(vec![id("name")])])],
//         );
//     }

//     #[test]
//     fn indented_string_with_expr_interpolation_token() {
//         assert_tokens(
//             "''${if ok then \"yes\" else \"no\"}''",
//             vec![ind_str(vec![interp(vec![
//                 id("if"),
//                 id("ok"),
//                 id("then"),
//                 str_lit(vec![part("yes")]),
//                 id("else"),
//                 str_lit(vec![part("no")]),
//             ])])],
//         );
//     }

//     #[test]
//     fn indented_shell_script_shape_token() {
//         assert_tokens(
//             "''\n  mkdir -p $out/bin\n  ${script}\n''",
//             vec![ind_str(vec![
//                 part("\n  mkdir -p $out/bin\n  "),
//                 interp(vec![id("script")]),
//                 part("\n"),
//             ])],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Paths
//     // ---------------------------------------------------------------------

//     #[test]
//     fn dot_slash_path_token() {
//         assert_tokens("./foo.nix", vec![path(vec![part("./foo.nix")])]);
//     }

//     #[test]
//     fn path_sequential_interp() {
//         assert_tokens(
//             "~/a${}${}b/c.foo",
//             vec![path(vec![
//                 part("~/a"),
//                 interp(vec![]),
//                 interp(vec![]),
//                 part("b/c.foo"),
//             ])],
//         );
//     }

//     #[test]
//     fn dot_slash_nested_path_token() {
//         assert_tokens(
//             "./foo/bar/baz.nix",
//             vec![path(vec![part("./foo/bar/baz.nix")])],
//         );
//     }

//     #[test]
//     fn bare_relative_path_token() {
//         assert_tokens("foo/bar", vec![path(vec![part("foo/bar")])]);
//     }

//     #[test]
//     fn bare_nested_relative_path_token() {
//         assert_tokens("foo/bar/baz", vec![path(vec![part("foo/bar/baz")])]);
//     }

//     #[test]
//     fn path_with_dash_underscore_plus_and_dot_token() {
//         assert_tokens(
//             "foo-bar_1.2+3/baz-qux_4.5+6",
//             vec![path(vec![part("foo-bar_1.2+3/baz-qux_4.5+6")])],
//         );
//     }

//     #[test]
//     fn parent_relative_path_token() {
//         assert_tokens("../foo/bar.nix", vec![path(vec![part("../foo/bar.nix")])]);
//     }

//     #[test]
//     fn absolute_path_token() {
//         assert_tokens(
//             "/etc/nixos/configuration.nix",
//             vec![path(vec![part("/etc/nixos/configuration.nix")])],
//         );
//     }

//     #[test]
//     fn home_relative_path_token() {
//         assert_tokens("~/foo/bar.nix", vec![path(vec![part("~/foo/bar.nix")])]);
//     }

//     #[test]
//     fn path_with_interpolation_after_slash_token() {
//         assert_tokens(
//             "./${name}.nix",
//             vec![path(vec![
//                 part("./"),
//                 interp(vec![id("name")]),
//                 part(".nix"),
//             ])],
//         );
//     }

//     #[test]
//     fn path_with_interpolation_between_segments_token() {
//         assert_tokens(
//             "foo/${bar}/baz",
//             vec![path(vec![
//                 part("foo/"),
//                 interp(vec![id("bar")]),
//                 part("/baz"),
//             ])],
//         );
//     }

//     #[test]
//     fn path_with_interpolation_suffix_token() {
//         assert_tokens(
//             "./foo-${bar}.nix",
//             vec![path(vec![
//                 part("./foo-"),
//                 interp(vec![id("bar")]),
//                 part(".nix"),
//             ])],
//         );
//     }

//     #[test]
//     fn path_with_multiple_interpolations_token() {
//         assert_tokens(
//             "./${dir}/${name}-${version}.nix",
//             vec![path(vec![
//                 part("./"),
//                 interp(vec![id("dir")]),
//                 part("/"),
//                 interp(vec![id("name")]),
//                 part("-"),
//                 interp(vec![id("version")]),
//                 part(".nix"),
//             ])],
//         );
//     }

//     #[test]
//     fn paths_inside_list_tokens() {
//         assert_tokens(
//             "[ ./a ./b/c ../d/e ]",
//             vec![
//                 p(Punct::OpenSquare),
//                 path(vec![part("./a")]),
//                 path(vec![part("./b/c")]),
//                 path(vec![part("../d/e")]),
//                 p(Punct::CloseSquare),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Lookup paths
//     // ---------------------------------------------------------------------

//     #[test]
//     fn lookup_without_slash_token() {
//         assert_tokens("<nixpkgs>", vec![Token::Lookup("nixpkgs")]);
//     }

//     #[test]
//     fn lookup_without_slash_with_dash_token() {
//         assert_tokens("<nixos-unstable>", vec![Token::Lookup("nixos-unstable")]);
//     }

//     #[test]
//     fn lookup_with_slash_token() {
//         assert_tokens("<nixpkgs/lib>", vec![Token::Lookup("nixpkgs/lib")]);
//     }

//     #[test]
//     fn lookup_with_nested_slashes_token() {
//         assert_tokens(
//             "<nixpkgs/pkgs/top-level>",
//             vec![Token::Lookup("nixpkgs/pkgs/top-level")],
//         );
//     }

//     #[test]
//     fn lookup_with_path_chars_token() {
//         assert_tokens(
//             "<foo-bar_1.2+3/baz-qux>",
//             vec![Token::Lookup("foo-bar_1.2+3/baz-qux")],
//         );
//     }

//     #[test]
//     fn import_lookup_tokens() {
//         assert_tokens(
//             "import <nixpkgs> {}",
//             vec![
//                 id("import"),
//                 Token::Lookup("nixpkgs"),
//                 p(Punct::OpenCurl),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // More path boundary tests
//     // ---------------------------------------------------------------------

//     #[test]
//     fn bare_ident_is_not_path_token() {
//         assert_tokens("foo", vec![id("foo")]);
//     }

//     #[test]
//     fn dotted_ident_is_not_path_token() {
//         assert_tokens("foo.bar", vec![id("foo"), p(Punct::Dot), id("bar")]);
//     }

//     #[test]
//     fn dashed_ident_is_not_path_token() {
//         assert_tokens("foo-bar", vec![id("foo-bar")]);
//     }

//     #[test]
//     fn slash_alone_is_not_path_token() {
//         assert_tokens("/", vec![p(Punct::Slash)]);
//     }

//     #[test]
//     fn double_slash_alone_is_not_path_token() {
//         assert_tokens("//", vec![p(Punct::DoubleSlash)]);
//     }

//     #[test]
//     fn dot_slash_path_stops_before_semicolon_token() {
//         assert_tokens(
//             "./foo;",
//             vec![path(vec![part("./foo")]), p(Punct::Semicolon)],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_comma_token() {
//         assert_tokens(
//             "./foo, bar",
//             vec![path(vec![part("./foo")]), p(Punct::Comma), id("bar")],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_colon_token() {
//         assert_tokens(
//             "./foo: bar",
//             vec![path(vec![part("./foo")]), p(Punct::Colon), id("bar")],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_question_token() {
//         assert_tokens(
//             "./foo? bar",
//             vec![path(vec![part("./foo")]), p(Punct::Question), id("bar")],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_at_token() {
//         assert_tokens(
//             "./foo@bar",
//             vec![path(vec![part("./foo")]), p(Punct::At), id("bar")],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_eq_token() {
//         assert_tokens(
//             "./foo=bar",
//             vec![path(vec![part("./foo")]), p(Punct::Eq), id("bar")],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_close_paren_token() {
//         assert_tokens(
//             "(./foo)",
//             vec![
//                 p(Punct::OpenParen),
//                 path(vec![part("./foo")]),
//                 p(Punct::CloseParen),
//             ],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_close_square_token() {
//         assert_tokens(
//             "[ ./foo ]",
//             vec![
//                 p(Punct::OpenSquare),
//                 path(vec![part("./foo")]),
//                 p(Punct::CloseSquare),
//             ],
//         );
//     }

//     #[test]
//     fn dot_slash_path_stops_before_close_curl_token() {
//         assert_tokens(
//             "{ src = ./foo; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("src"),
//                 p(Punct::Eq),
//                 path(vec![part("./foo")]),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn spaced_plus_after_path_is_operator_token() {
//         assert_tokens(
//             "./foo + bar",
//             vec![path(vec![part("./foo")]), p(Punct::Plus), id("bar")],
//         );
//     }

//     #[test]
//     fn plus_inside_path_segment_is_path_char_token() {
//         assert_tokens("./foo+bar", vec![path(vec![part("./foo+bar")])]);
//     }

//     #[test]
//     fn spaced_concat_after_path_is_operator_token() {
//         assert_tokens(
//             "./foo ++ bar",
//             vec![path(vec![part("./foo")]), p(Punct::PlusPlus), id("bar")],
//         );
//     }

//     #[test]
//     fn plus_plus_inside_path_segment_is_path_chars_token() {
//         assert_tokens("./foo++bar", vec![path(vec![part("./foo++bar")])]);
//     }

//     #[test]
//     fn path_with_interpolation_after_slash_token_precise() {
//         assert_tokens(
//             "./${name}.nix",
//             vec![path(vec![
//                 part("./"),
//                 interp(vec![id("name")]),
//                 part(".nix"),
//             ])],
//         );
//     }

//     #[test]
//     fn path_with_interpolation_after_existing_path_slash_token_precise() {
//         assert_tokens(
//             "foo/${bar}/baz",
//             vec![path(vec![
//                 part("foo/"),
//                 interp(vec![id("bar")]),
//                 part("/baz"),
//             ])],
//         );
//     }

//     #[test]
//     fn path_with_two_interpolations_token_precise() {
//         assert_tokens(
//             "./${dir}/${name}.nix",
//             vec![path(vec![
//                 part("./"),
//                 interp(vec![id("dir")]),
//                 part("/"),
//                 interp(vec![id("name")]),
//                 part(".nix"),
//             ])],
//         );
//     }

//     #[test]
//     fn interpolation_before_any_slash_is_not_folded_into_relative_path_token() {
//         assert_tokens(
//             "foo${bar}/baz",
//             vec![
//                 id("foo"),
//                 Token::Interp(vec![id("bar")]),
//                 path(vec![part("/baz")]),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // More lookup path boundary tests
//     // ---------------------------------------------------------------------

//     fn lookup(s: &'static str) -> Tok {
//         Token::Lookup(s)
//     }

//     #[test]
//     fn lookup_nixpkgs_without_slash_token() {
//         assert_tokens("<nixpkgs>", vec![lookup("nixpkgs")]);
//     }

//     #[test]
//     fn lookup_simple_name_then_ident_token() {
//         assert_tokens("<nixpkgs>foo", vec![lookup("nixpkgs"), id("foo")]);
//     }

//     #[test]
//     fn lookup_simple_name_then_dot_attr_token() {
//         assert_tokens(
//             "<nixpkgs>.lib",
//             vec![lookup("nixpkgs"), p(Punct::Dot), id("lib")],
//         );
//     }

//     #[test]
//     fn lookup_with_dash_without_slash_token() {
//         assert_tokens("<nixos-unstable>", vec![lookup("nixos-unstable")]);
//     }

//     #[test]
//     fn lookup_with_dot_without_slash_token() {
//         assert_tokens("<foo.bar>", vec![lookup("foo.bar")]);
//     }

//     #[test]
//     fn lookup_with_plus_without_slash_token() {
//         assert_tokens("<foo+bar>", vec![lookup("foo+bar")]);
//     }

//     #[test]
//     fn lookup_with_underscore_without_slash_token() {
//         assert_tokens("<foo_bar>", vec![lookup("foo_bar")]);
//     }

//     #[test]
//     fn lookup_with_digits_without_slash_token() {
//         assert_tokens("<foo123>", vec![lookup("foo123")]);
//     }

//     #[test]
//     fn lookup_nested_path_token() {
//         assert_tokens("<nixpkgs/lib/systems>", vec![lookup("nixpkgs/lib/systems")]);
//     }

//     #[test]
//     fn lookup_does_not_need_to_start_with_alpha_token() {
//         assert_tokens("<1foo>", vec![lookup("1foo")]);
//     }

//     #[test]
//     fn angle_comparison_is_not_lookup_token() {
//         assert_tokens(
//             "a < b > c",
//             vec![id("a"), p(Punct::Less), id("b"), p(Punct::Greater), id("c")],
//         );
//     }

//     #[test]
//     fn spaced_angle_text_is_not_lookup_token() {
//         assert_tokens(
//             "<nix pkgs>",
//             vec![p(Punct::Less), id("nix"), id("pkgs"), p(Punct::Greater)],
//         );
//     }

//     #[test]
//     fn empty_angle_pair_is_not_lookup_token() {
//         assert_tokens("<>", vec![p(Punct::Less), p(Punct::Greater)]);
//     }

//     #[test]
//     fn lookup_does_not_allow_interpolation_token() {
//         assert_tokens(
//             "<${nixpkgs}>",
//             vec![
//                 p(Punct::Less),
//                 Token::Interp(vec![id("nixpkgs")]),
//                 p(Punct::Greater),
//             ],
//         );
//     }

//     #[test]
//     fn lookup_stops_at_first_greater_token() {
//         assert_tokens(
//             "<foo> > bar",
//             vec![lookup("foo"), p(Punct::Greater), id("bar")],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Interpolation as a top-level token
//     // ---------------------------------------------------------------------

//     #[test]
//     fn top_level_interpolation_ident_token() {
//         assert_tokens("${name}", vec![Token::Interp(vec![id("name")])]);
//     }

//     #[test]
//     fn top_level_interpolation_expr_token() {
//         assert_tokens(
//             "${x + y}",
//             vec![Token::Interp(vec![id("x"), p(Punct::Plus), id("y")])],
//         );
//     }

//     #[test]
//     fn top_level_interpolation_nested_attrset_token() {
//         assert_tokens(
//             "${{ x = 1; }}",
//             vec![Token::Interp(vec![
//                 p(Punct::OpenCurl),
//                 id("x"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ])],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Common Nix expression shapes
//     // ---------------------------------------------------------------------

//     #[test]
//     fn simple_let_expr_tokens() {
//         assert_tokens(
//             "let x = 1; y = 2; in x + y",
//             vec![
//                 id("let"),
//                 id("x"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 id("y"),
//                 p(Punct::Eq),
//                 int(2),
//                 p(Punct::Semicolon),
//                 id("in"),
//                 id("x"),
//                 p(Punct::Plus),
//                 id("y"),
//             ],
//         );
//     }

//     #[test]
//     fn simple_if_expr_tokens() {
//         assert_tokens(
//             "if x then y else z",
//             vec![id("if"), id("x"), id("then"), id("y"), id("else"), id("z")],
//         );
//     }

//     #[test]
//     fn simple_attrset_tokens() {
//         assert_tokens(
//             "{ a = 1; b = 2; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("a"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 id("b"),
//                 p(Punct::Eq),
//                 int(2),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn rec_attrset_tokens() {
//         assert_tokens(
//             "rec { x = y; y = 1; }",
//             vec![
//                 id("rec"),
//                 p(Punct::OpenCurl),
//                 id("x"),
//                 p(Punct::Eq),
//                 id("y"),
//                 p(Punct::Semicolon),
//                 id("y"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn list_tokens() {
//         assert_tokens(
//             "[ 1 2 3 ]",
//             vec![
//                 p(Punct::OpenSquare),
//                 int(1),
//                 int(2),
//                 int(3),
//                 p(Punct::CloseSquare),
//             ],
//         );
//     }

//     #[test]
//     fn lambda_tokens() {
//         assert_tokens("x: x", vec![id("x"), p(Punct::Colon), id("x")]);
//     }

//     #[test]
//     fn set_pattern_tokens() {
//         assert_tokens(
//             "{ x, y ? 1, z, ... }: x",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("x"),
//                 p(Punct::Comma),
//                 id("y"),
//                 p(Punct::Question),
//                 int(1),
//                 p(Punct::Comma),
//                 id("z"),
//                 p(Punct::Comma),
//                 p(Punct::Ellipses),
//                 p(Punct::CloseCurl),
//                 p(Punct::Colon),
//                 id("x"),
//             ],
//         );
//     }

//     #[test]
//     fn at_pattern_tokens() {
//         assert_tokens(
//             "args@{ x, y, ... }: x",
//             vec![
//                 id("args"),
//                 p(Punct::At),
//                 p(Punct::OpenCurl),
//                 id("x"),
//                 p(Punct::Comma),
//                 id("y"),
//                 p(Punct::Comma),
//                 p(Punct::Ellipses),
//                 p(Punct::CloseCurl),
//                 p(Punct::Colon),
//                 id("x"),
//             ],
//         );
//     }

//     #[test]
//     fn inherit_plain_tokens() {
//         assert_tokens(
//             "{ inherit x y z; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("inherit"),
//                 id("x"),
//                 id("y"),
//                 id("z"),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn inherit_from_tokens() {
//         assert_tokens(
//             "{ inherit (builtins) map filter; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("inherit"),
//                 p(Punct::OpenParen),
//                 id("builtins"),
//                 p(Punct::CloseParen),
//                 id("map"),
//                 id("filter"),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn update_operator_tokens() {
//         assert_tokens(
//             "{ a = 1; } // { b = 2; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("a"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//                 p(Punct::DoubleSlash),
//                 p(Punct::OpenCurl),
//                 id("b"),
//                 p(Punct::Eq),
//                 int(2),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn concat_operator_tokens() {
//         assert_tokens(
//             "[ 1 ] ++ [ 2 ]",
//             vec![
//                 p(Punct::OpenSquare),
//                 int(1),
//                 p(Punct::CloseSquare),
//                 p(Punct::PlusPlus),
//                 p(Punct::OpenSquare),
//                 int(2),
//                 p(Punct::CloseSquare),
//             ],
//         );
//     }

//     #[test]
//     fn attr_selection_with_or_tokens() {
//         assert_tokens(
//             "a.b.c or 123",
//             vec![
//                 id("a"),
//                 p(Punct::Dot),
//                 id("b"),
//                 p(Punct::Dot),
//                 id("c"),
//                 id("or"),
//                 int(123),
//             ],
//         );
//     }

//     #[test]
//     fn has_attr_tokens() {
//         assert_tokens("x ? y", vec![id("x"), p(Punct::Question), id("y")]);
//     }

//     #[test]
//     fn string_attr_name_tokens() {
//         assert_tokens(
//             r#"{ "foo bar" = 1; }"#,
//             vec![
//                 p(Punct::OpenCurl),
//                 str_lit(vec![part("foo bar")]),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn interpolated_attr_name_tokens() {
//         assert_tokens(
//             r#"{ ${name} = 1; }"#,
//             vec![
//                 p(Punct::OpenCurl),
//                 Token::Interp(vec![id("name")]),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn mixed_attr_path_tokens() {
//         assert_tokens(
//             r#"{ a."b c".${d}.e = 1; }"#,
//             vec![
//                 p(Punct::OpenCurl),
//                 id("a"),
//                 p(Punct::Dot),
//                 str_lit(vec![part("b c")]),
//                 p(Punct::Dot),
//                 Token::Interp(vec![id("d")]),
//                 p(Punct::Dot),
//                 id("e"),
//                 p(Punct::Eq),
//                 int(1),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn call_package_fragment_tokens() {
//         assert_tokens(
//             "prev.callPackage ./pkgs/mypkg { inherit lib; }",
//             vec![
//                 id("prev"),
//                 p(Punct::Dot),
//                 id("callPackage"),
//                 path(vec![part("./pkgs/mypkg")]),
//                 p(Punct::OpenCurl),
//                 id("inherit"),
//                 id("lib"),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn flake_input_url_attr_tokens() {
//         assert_tokens(
//             r#"inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";"#,
//             vec![
//                 id("inputs"),
//                 p(Punct::Dot),
//                 id("nixpkgs"),
//                 p(Punct::Dot),
//                 id("url"),
//                 p(Punct::Eq),
//                 str_lit(vec![part("github:NixOS/nixpkgs/nixos-unstable")]),
//                 p(Punct::Semicolon),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // Lexically valid token streams that the parser may later reject
//     // ---------------------------------------------------------------------

//     #[test]
//     fn bare_else_still_lexes_to_ident_token() {
//         assert_tokens("else", vec![id("else")]);
//     }

//     #[test]
//     fn nonsense_keyword_sequence_still_lexes_to_idents() {
//         assert_tokens(
//             "if if then else else",
//             vec![id("if"), id("if"), id("then"), id("else"), id("else")],
//         );
//     }

//     #[test]
//     fn attr_missing_value_still_lexes_to_tokens() {
//         assert_tokens(
//             "{ x = ; }",
//             vec![
//                 p(Punct::OpenCurl),
//                 id("x"),
//                 p(Punct::Eq),
//                 p(Punct::Semicolon),
//                 p(Punct::CloseCurl),
//             ],
//         );
//     }

//     #[test]
//     fn unmatched_close_delimiters_still_lex_to_tokens() {
//         assert_tokens(
//             "} ] )",
//             vec![
//                 p(Punct::CloseCurl),
//                 p(Punct::CloseSquare),
//                 p(Punct::CloseParen),
//             ],
//         );
//     }

//     // ---------------------------------------------------------------------
//     // True lexical failures
//     // ---------------------------------------------------------------------

//     #[test]
//     fn rejects_unterminated_double_quote_empty() {
//         assert_rejects("\"");
//     }

//     #[test]
//     fn rejects_unterminated_double_quote_text() {
//         assert_rejects("\"hello");
//     }

//     #[test]
//     fn rejects_unterminated_double_quote_after_escape() {
//         assert_rejects(r#""hello \"#);
//     }

//     #[test]
//     fn rejects_unterminated_double_quote_interpolation() {
//         assert_rejects(r#""hello ${name""#);
//     }

//     #[test]
//     fn rejects_unterminated_nested_string_inside_interpolation() {
//         assert_rejects(r#""${"abc}""#);
//     }

//     #[test]
//     fn rejects_unterminated_indented_string_open_only() {
//         assert_rejects("''");
//     }

//     #[test]
//     fn rejects_unterminated_indented_string_text() {
//         assert_rejects("''hello");
//     }

//     #[test]
//     fn rejects_unterminated_indented_string_multiline() {
//         assert_rejects("''\n  hello\n  world\n");
//     }

//     #[test]
//     fn rejects_unterminated_indented_string_interpolation() {
//         assert_rejects("''hello ${name''");
//     }

//     #[test]
//     fn rejects_top_level_unterminated_interpolation() {
//         assert_rejects("${x");
//     }

//     #[test]
//     fn rejects_top_level_unterminated_interpolation_with_expr() {
//         assert_rejects("${x + y");
//     }

//     #[test]
//     fn rejects_string_interpolation_with_unclosed_brace() {
//         assert_rejects(r#""${x + y""#);
//     }

//     #[test]
//     fn rejects_indented_string_interpolation_with_unclosed_brace() {
//         assert_rejects("''${x + y''");
//     }

//     #[test]
//     fn rejects_unterminated_multiline_comment() {
//         assert_rejects("/* hello");
//     }

//     #[test]
//     fn rejects_unterminated_doc_comment() {
//         assert_rejects("/** hello");
//     }

//     #[test]
//     fn rejects_backtick() {
//         assert_rejects("`");
//     }

//     #[test]
//     fn rejects_backtick_between_tokens() {
//         assert_rejects("x ` y");
//     }

//     #[test]
//     fn rejects_single_quote_alone() {
//         assert_rejects("'");
//     }

//     #[test]
//     fn rejects_single_quote_prefixed_ident() {
//         assert_rejects("'foo");
//     }

//     #[test]
//     fn rejects_int_over_i64_max() {
//         assert_rejects("9223372036854775808");
//     }

//     #[test]
//     fn leading_dot_float() {
//         assert_tokens(".25", vec![Token::Float(0.25)]);
//     }

//     #[test]
//     fn negative_int_is_unary_minus() {
//         assert_tokens("-1", vec![Token::Punct(Punct::Minus), Token::Int(1)]);
//     }

//     #[test]
//     fn negative_leading_dot_float_is_unary_minus() {
//         assert_tokens("-.25", vec![Token::Punct(Punct::Minus), Token::Float(0.25)]);
//     }

//     #[test]
//     fn infix_minus_not_greedy_number() {
//         assert_tokens(
//             "1-2",
//             vec![Token::Int(1), Token::Punct(Punct::Minus), Token::Int(2)],
//         );
//     }

//     #[test]
//     fn infix_plus_not_greedy_number() {
//         assert_tokens(
//             "1+2",
//             vec![Token::Int(1), Token::Punct(Punct::Plus), Token::Int(2)],
//         );
//     }

//     #[test]
//     fn one_dot_is_float_for_nix_compat() {
//         assert_tokens("1.", vec![Token::Float(1.0)]);
//     }

//     #[test]
//     fn zero_dot_is_not_float_for_nix_compat() {
//         assert_tokens("0.", vec![Token::Int(0), Token::Punct(Punct::Dot)]);
//     }
// }
