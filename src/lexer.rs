use std::fmt::{self, Display};

use colour::grey;
use strum::{EnumIter, EnumProperty, IntoEnumIterator};
use winnow::Result;
use winnow::ascii::{Caseless, dec_int, float, multispace1, till_line_ending};
use winnow::combinator::{delimited, eof, not, peek, preceded, repeat, terminated, trace};
use winnow::error::ParserError;
use winnow::stream::{AsBStr, AsChar, Compare, FindSlice, ParseSlice, Stream, StreamIsPartial};
use winnow::token::{any, none_of, take_until, take_while};
use winnow::{
    combinator::{alt, opt},
    prelude::*,
    token::{literal, one_of},
};

fn alt_iter<I, O, E>(it: impl IntoIterator<Item = impl Parser<I, O, E>>) -> impl Parser<I, O, E>
where
    I: Stream,
    E: ParserError<I>,
{
    let mut it = it.into_iter();
    trace("alt_iter", move |input: &mut I| {
        let mut error: Option<E> = None;
        let start = input.checkpoint();
        for mut branch in &mut it {
            input.reset(&start);
            match branch.parse_next(input) {
                Err(e) if e.is_backtrack() => {
                    error = match error {
                        Some(error) => Some(error.or(e)),
                        None => Some(e),
                    };
                }
                res => return res,
            }
        }
        match error {
            Some(e) => Err(e.append(input, &start)),
            None => Err(ParserError::from_input(input)),
        }
    })
}

fn punct_p<I, E>(input: &mut I) -> Result<Punct, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str>,
    E: ParserError<I>,
{
    let punct_to_p = |punct: Punct| literal(punct.as_str()).value(punct);
    alt_iter(Punct::iter().map(punct_to_p)).parse_next(input)
}

fn singleline_comment_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<(char, char)>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
{
    preceded("#", till_line_ending).parse_next(input)
}

fn doc_comment_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<&'static str>,
    E: ParserError<I>,
{
    delimited("/**", take_until(0.., "*/"), "*/").parse_next(input)
}

fn multiline_comment_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str> + FindSlice<&'static str>,
    E: ParserError<I>,
{
    delimited(("/*", not("*")), take_until(0.., "*/"), "*/").parse_next(input)
}

fn ident_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<&'static str>,
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
        .take()
        .parse_next(input)
}

fn strlit_p<I, E>(input: &mut I) -> Result<Vec<InterpPart<I>>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    '"'.parse_next(input)?;
    let ignore_p = alt((('\\', any).void(), "$$".void()));
    let end_p = alt(("\"", "${"));
    let mut str_part_p = escaped_take_until(ignore_p, end_p);
    let mut parts = Vec::new();
    loop {
        let str_part = str_part_p.parse_next(input)?;
        if !str_part.as_bstr().is_empty() {
            parts.push(InterpPart::Str(str_part));
        }
        if let Some(interp) = opt(interp_p).parse_next(input)? {
            parts.push(InterpPart::Interp(interp));
        } else {
            break;
        }
    }
    '"'.parse_next(input)?;
    Ok(parts)
}

fn indented_strlit_p<I, E>(input: &mut I) -> Result<Vec<InterpPart<I>>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    "''".parse_next(input)?;
    let ignore_p = alt((
        ("''\\", any).void(),
        "''$".void(),
        "'''".void(),
        "$$".void(),
    ));
    let end_p = alt(("''", "${"));
    let mut str_part_p = escaped_take_until(ignore_p, end_p);
    let mut parts = Vec::new();
    loop {
        let str_part = str_part_p.parse_next(input)?;
        if !str_part.as_bstr().is_empty() {
            parts.push(InterpPart::Str(str_part));
        }
        if let Some(interp) = opt(interp_p).parse_next(input)? {
            parts.push(InterpPart::Interp(interp));
        } else {
            break;
        }
    }
    "''".parse_next(input)?;
    Ok(parts)
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

fn number_p<I, E>(input: &mut I) -> Result<Token<I>, E>
where
    I: Stream + StreamIsPartial,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: ParseSlice<f64> + ParseSlice<i64>,
{
    let num_p = || take_while(1.., (AsChar::is_dec_digit, '.', 'E', 'e', '+', '-'));
    alt((
        num_p().parse_to::<i64>().map(Token::Int),
        num_p().parse_to::<f64>().map(Token::Float),
    ))
    .parse_next(input)
}

fn path_char<C: AsChar>(c: C) -> bool {
    let c = c.as_char();
    c.is_alphanum() || ['.', '\\', '-', '_', '+'].contains(&c)
}
fn path_p<I, E>(input: &mut I) -> Result<Vec<InterpPart<I>>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    preceded(
        peek((take_while(0.., path_char), '/', take_while(0.., path_char))),
        repeat::<_, _, Vec<_>, _, _>(
            1..,
            alt((
                take_while(1.., (path_char, '/')).map(InterpPart::Str),
                interp_p.map(InterpPart::Interp),
            )),
        ),
    )
    .verify(|parts: &Vec<_>| {
        parts.iter().any(|part| match part {
            InterpPart::Str(s) => {
                AsBStr::as_bstr(s).contains(&b'/') && AsBStr::as_bstr(s).iter().any(|b| *b != b'/')
            }
            InterpPart::Interp(_) => false,
        })
    })
    .parse_next(input)
}

fn lookup_p<I, E>(input: &mut I) -> Result<I::Slice, E>
where
    I: Stream + StreamIsPartial + Compare<char>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr,
{
    delimited('<', take_while(1.., (path_char, '/')), '>')
        .verify(|s| {
            AsBStr::as_bstr(s).contains(&b'/') && AsBStr::as_bstr(s).iter().any(|b| *b != b'/')
        })
        .parse_next(input)
}

fn blank_p<I, E>(input: &mut I) -> Result<(), E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>,
    E: ParserError<I>,
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

pub fn token_p<I, E>(input: &mut I) -> Result<Token<I>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    let tok_p = || {
        alt((
            trace("path_p", path_p).map(Token::Path),
            trace("lookup_p", lookup_p).map(Token::Lookup),
            trace("ident_p", ident_p).map(Token::Ident),
            trace("doc_comment_p", doc_comment_p).map(Token::DocComment),
            trace("punct_p", punct_p).map(Token::Punct),
            trace("strlit_p", strlit_p).map(Token::StrLit),
            trace("indented_strlit_p", indented_strlit_p).map(Token::IndentedStrLit),
            trace("interp_p", interp_p).map(Token::Interp),
            trace("number_p", number_p),
        ))
    };
    alt((terminated(tok_p(), blank_p), preceded(blank_p, tok_p()))).parse_next(input)
}

fn interp_p<I, E>(input: &mut I) -> Result<Vec<Token<I>>, E>
where
    I: Stream
        + StreamIsPartial
        + Compare<&'static str>
        + Compare<char>
        + FindSlice<(char, char)>
        + FindSlice<&'static str>
        + Compare<Caseless<&'static str>>,
    E: ParserError<I>,
    I::Token: AsChar + Clone,
    I::Slice: AsBStr + ParseSlice<f64> + ParseSlice<i64>,
    I::IterOffsets: Clone,
{
    "${".parse_next(input)?;
    let mut depth = 0;
    let mut tokens = Vec::new();
    loop {
        let tok = token_p.parse_next(input)?;
        match &tok {
            Token::Punct(Punct::OpenCurl) => depth += 1,
            Token::Punct(Punct::CloseCurl) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
        tokens.push(tok);
    }
    Ok(tokens)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, EnumProperty)]
pub enum Punct {
    #[strum(props(lit = ";"))]
    Semicolon,
    #[strum(props(lit = "{"))]
    OpenCurl,
    #[strum(props(lit = "}"))]
    CloseCurl,
    #[strum(props(lit = "("))]
    OpenParen,
    #[strum(props(lit = ")"))]
    CloseParen,
    #[strum(props(lit = "..."))]
    Ellipses,
    #[strum(props(lit = "."))]
    Dot,
    #[strum(props(lit = "=="))]
    EqEq,
    #[strum(props(lit = "="))]
    Eq,
    #[strum(props(lit = ","))]
    Comma,
    #[strum(props(lit = "["))]
    OpenSquare,
    #[strum(props(lit = "]"))]
    CloseSquare,
    #[strum(props(lit = ":"))]
    Colon,
    #[strum(props(lit = "//"))]
    DoubleSlash,
    #[strum(props(lit = "/"))]
    Slash,
    #[strum(props(lit = "++"))]
    PlusPlus,
    #[strum(props(lit = "+"))]
    Plus,
    #[strum(props(lit = "->"))]
    RightArrow,
    #[strum(props(lit = "-"))]
    Minus,
    #[strum(props(lit = "*"))]
    Mul,
    #[strum(props(lit = "@"))]
    At,
    #[strum(props(lit = "?"))]
    Question,
    #[strum(props(lit = "<="))]
    LessEq,
    #[strum(props(lit = "<"))]
    Less,
    #[strum(props(lit = ">="))]
    GreaterEq,
    #[strum(props(lit = ">"))]
    Greater,
    #[strum(props(lit = "!="))]
    BangEq,
    #[strum(props(lit = "!"))]
    Bang,
    #[strum(props(lit = "&&"))]
    LogicalAnd,
    #[strum(props(lit = "||"))]
    LogicalOr,
}
impl Punct {
    pub fn as_str(&self) -> &'static str {
        self.get_str("lit")
            .expect("every punct should have a `lit` property")
    }
}

#[derive(Debug, PartialEq)]
pub enum Token<I: Stream> {
    DocComment(I::Slice),
    Interp(Vec<Token<I>>),
    IndentedStrLit(Vec<InterpPart<I>>),
    StrLit(Vec<InterpPart<I>>),
    Path(Vec<InterpPart<I>>),
    Lookup(I::Slice),
    Ident(I::Slice),
    Punct(Punct),
    Int(i64),
    Float(f64),
}

impl Display for Token<&str> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::DocComment(s) => {
                colour::write_grey!(f, "{s}")
            }

            Token::Interp(tokens) => {
                colour::write_magenta!(f, "${{")?;

                for tok in tokens {
                    write!(f, "{tok}")?;
                }

                colour::write_magenta!(f, "}}")
            }

            Token::IndentedStrLit(items) => {
                colour::write_green!(f, "''")?;
                fmt_interp_parts(f, items, PartColour::String)?;
                colour::write_green!(f, "''")
            }

            Token::StrLit(items) => {
                colour::write_green!(f, "\"")?;
                fmt_interp_parts(f, items, PartColour::String)?;
                colour::write_green!(f, "\"")
            }

            Token::Path(items) => fmt_interp_parts(f, items, PartColour::Path),

            Token::Lookup(s) => {
                colour::write_cyan!(f, "<{s}>")
            }

            Token::Ident(s) => {
                colour::write_white!(f, "{s}")
            }

            Token::Punct(punct) => {
                colour::write_yellow!(f, "{punct}")
            }

            Token::Int(n) => {
                colour::write_blue!(f, "{n}")
            }

            Token::Float(x) => {
                colour::write_blue!(f, "{x:.2}")
            }
        }
    }
}

#[derive(Clone, Copy)]
enum PartColour {
    String,
    Path,
}

fn fmt_interp_parts(
    f: &mut fmt::Formatter<'_>,
    items: &[InterpPart<&str>],
    colour: PartColour,
) -> fmt::Result {
    for item in items {
        match item {
            InterpPart::Str(s) => match colour {
                PartColour::String => {
                    colour::write_green!(f, "{s}")?;
                }
                PartColour::Path => {
                    colour::write_cyan!(f, "{s}")?;
                }
            },

            InterpPart::Interp(tokens) => {
                colour::write_magenta!(f, "${{")?;

                for tok in tokens {
                    write!(f, "{tok}")?;
                }

                colour::write_magenta!(f, "}}")?;
            }
        }
    }

    Ok(())
}

impl Display for Punct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, PartialEq)]
pub enum InterpPart<I: Stream> {
    Str(I::Slice),
    Interp(Vec<Token<I>>),
}
