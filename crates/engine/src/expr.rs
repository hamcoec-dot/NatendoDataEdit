//! Expressions for computed columns and row checks: a small, Excel-like
//! language over a row's cells. An expression is parsed once (columns and
//! function names are resolved, literal patterns compiled) and evaluated per
//! row without allocating for plain column values. Results are written as
//! text: a delimited file cannot keep formulas.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Progress};
use crate::fill::{civil_from_days, days_from_civil, format_number};
use crate::join::LookupTable;
use crate::ops::{check_cols, ColPlan, Cols};
use crate::value::{parse_date, parse_number, ReadOptions};
use crate::Error;

/// Longest expression, in characters.
pub const MAX_EXPR_CHARS: usize = 10_000;
/// Deepest nesting of brackets and calls.
pub const MAX_DEPTH: usize = 64;
/// Most parts (values, operators, calls) in one expression.
pub const MAX_NODES: usize = 2_000;
/// Longest text a result may be, in bytes.
pub const MAX_TEXT: usize = 1 << 20;
/// Most group references (`$1`, `$0`) in one REGEXREPLACE replacement.
pub const MAX_REFS: usize = 1_000;
/// Most rows a preview shows.
pub const MAX_PREVIEW: u64 = 200;
/// Most LOOKUPs in one expression.
pub const MAX_LOOKUPS: usize = 8;

/// An error value, written as its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Wrong kind of value (text where a number is needed).
    Value,
    Div0,
    /// A number out of range (square root of a negative, overflow).
    Num,
    /// Nothing found.
    Na,
    /// A LOOKUP whose tab or column is not open.
    Ref,
}

impl Fault {
    pub fn code(self) -> &'static str {
        match self {
            Fault::Value => "#VALUE!",
            Fault::Div0 => "#DIV/0!",
            Fault::Num => "#NUM!",
            Fault::Na => "#N/A",
            Fault::Ref => "#REF!",
        }
    }
}

/// A value while evaluating.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    Blank,
    Num(f64),
    Text(Cow<'a, str>),
    Bool(bool),
    /// Days since 1970-01-01; a fraction is the time of day.
    Date(f64),
    Err(Fault),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Join,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

macro_rules! funcs {
    ($($f:ident $name:literal $min:literal $max:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[allow(clippy::upper_case_acronyms)]
        enum Func { $($f),* }
        /// Function names with their argument counts (max 255 = any).
        const FUNCS: &[(&str, Func, u8, u8)] = &[$(($name, Func::$f, $min, $max)),*];
    };
}

funcs! {
    If "IF" 2 3, Ifs "IFS" 2 254, IfError "IFERROR" 2 2, And "AND" 1 255,
    Or "OR" 1 255, Not "NOT" 1 1, IsBlank "ISBLANK" 1 1, IsNumber "ISNUMBER" 1 1,
    IsText "ISTEXT" 1 1, IsDate "ISDATE" 1 1, IsError "ISERROR" 1 1,
    Len "LEN" 1 1, Left "LEFT" 1 2, Right "RIGHT" 1 2, Mid "MID" 3 3,
    Upper "UPPER" 1 1, Lower "LOWER" 1 1, Proper "PROPER" 1 1, Trim "TRIM" 1 1,
    Concat "CONCAT" 1 255, Substitute "SUBSTITUTE" 3 4, Replace "REPLACE" 4 4,
    Find "FIND" 2 3, Search "SEARCH" 2 3, Exact "EXACT" 2 2, Rept "REPT" 2 2,
    TextJoin "TEXTJOIN" 3 255, ValueF "VALUE" 1 1, Fixed "FIXED" 1 3,
    Round "ROUND" 1 2, RoundUp "ROUNDUP" 1 2, RoundDown "ROUNDDOWN" 1 2,
    Int "INT" 1 1, Abs "ABS" 1 1, Sign "SIGN" 1 1, Mod "MOD" 2 2,
    Power "POWER" 2 2, Sqrt "SQRT" 1 1, Exp "EXP" 1 1, Ln "LN" 1 1,
    Log10 "LOG10" 1 1, Min "MIN" 1 255, Max "MAX" 1 255, Sum "SUM" 1 255,
    Average "AVERAGE" 1 255, Date "DATE" 3 3, Year "YEAR" 1 1, Month "MONTH" 1 1,
    Day "DAY" 1 1, Hour "HOUR" 1 1, Minute "MINUTE" 1 1, Weekday "WEEKDAY" 1 1,
    Days "DAYS" 2 2, Today "TODAY" 0 0, Now "NOW" 0 0, Row "ROW" 0 0,
    Coalesce "COALESCE" 1 255, RegexMatch "REGEXMATCH" 2 2,
    RegexExtract "REGEXEXTRACT" 2 3, RegexReplace "REGEXREPLACE" 3 3,
    Lookup "LOOKUP" 4 5,
}

/// Every function name, for the help list and the editor.
pub fn function_names() -> Vec<&'static str> {
    FUNCS.iter().map(|f| f.0).collect()
}

#[derive(Debug)]
enum Node {
    Num(f64),
    Text(String),
    Bool(bool),
    /// Index into `Expr::cols`.
    Col(usize),
    Neg(Box<Node>),
    Percent(Box<Node>),
    Bin(Op, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>, Option<Box<Regex>>),
    /// LOOKUP(value, "tab", "key column", "result column"[, not found]):
    /// `lookups[idx]` of the expression, bound before it runs.
    Lookup {
        idx: usize,
        table: Option<Arc<LookupTable>>,
        key: Box<Node>,
        fallback: Option<Box<Node>>,
    },
}

/// What a LOOKUP reads: another tab's name, its key column and its result
/// column (column titles from its first row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupRef {
    pub tab: String,
    pub key: String,
    pub column: String,
}

/// Lookup tables for an expression's LOOKUPs, in [`Expr::lookups`] order.
/// The app copies them out of the other tabs first; they are not saved.
#[derive(Debug, Clone, Default)]
pub struct Lookups(pub Vec<Option<Arc<LookupTable>>>);

impl PartialEq for Lookups {
    fn eq(&self, o: &Self) -> bool {
        self.0.len() == o.0.len()
            && self.0.iter().zip(&o.0).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            })
    }
}

/// A parsed expression.
#[derive(Debug)]
pub struct Expr {
    root: Node,
    /// Logical columns read, in the order `Node::Col` numbers them.
    cols: Vec<u32>,
    lookups: Vec<LookupRef>,
}

/// Where an expression runs: the row's cells, where each of the
/// expression's columns (`Expr::columns` order) sits among them (`None`:
/// blank), the row number (from 1), the local time and how numbers are read.
pub struct Ctx<'b> {
    pub cells: &'b [Cow<'b, str>],
    pub slots: &'b [Option<usize>],
    pub row: u64,
    pub now: f64,
    pub read: ReadOptions,
}

// ---- tokens ----

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Text(String),
    Col(String),
    Name(String),
    Op(&'static str),
    Open,
    Close,
    Comma,
}

fn bad(msg: &str, at: usize) -> Error {
    Error::Invalid(format!("{msg} (at character {})", at + 1))
}

fn tokens(src: &str) -> Result<Vec<(Tok, usize)>, Error> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let at = i;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let tok = match c {
            '0'..='9' | '.' => {
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let t: String = chars[s..i].iter().collect();
                let v: f64 = t.parse().map_err(|_| bad("not a number", at))?;
                out.push((Tok::Num(v), at));
                continue;
            }
            '"' => {
                let mut t = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(bad("text is missing its closing quote", at)),
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            t.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            t.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push((Tok::Text(t), at));
                continue;
            }
            '[' => {
                let mut t = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(bad("a column name is missing its ]", at)),
                        Some(']') if chars.get(i + 1) == Some(&']') => {
                            t.push(']');
                            i += 2;
                        }
                        Some(']') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            t.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push((Tok::Col(t), at));
                continue;
            }
            c if c.is_alphabetic() || c == '_' => {
                let s = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push((Tok::Name(chars[s..i].iter().collect()), at));
                continue;
            }
            '(' => Tok::Open,
            ')' => Tok::Close,
            ',' | ';' => Tok::Comma,
            '<' if chars.get(i + 1) == Some(&'>') => {
                i += 1;
                Tok::Op("<>")
            }
            '<' if chars.get(i + 1) == Some(&'=') => {
                i += 1;
                Tok::Op("<=")
            }
            '>' if chars.get(i + 1) == Some(&'=') => {
                i += 1;
                Tok::Op(">=")
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                i += 1;
                Tok::Op("<>")
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                i += 1;
                Tok::Op("=")
            }
            '+' => Tok::Op("+"),
            '-' => Tok::Op("-"),
            '*' => Tok::Op("*"),
            '/' => Tok::Op("/"),
            '^' => Tok::Op("^"),
            '&' => Tok::Op("&"),
            '%' => Tok::Op("%"),
            '=' => Tok::Op("="),
            '<' => Tok::Op("<"),
            '>' => Tok::Op(">"),
            _ => return Err(bad(&format!("unexpected character '{c}'"), at)),
        };
        out.push((tok, at));
        i += 1;
    }
    Ok(out)
}

// ---- parser ----

struct Parser<'n> {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    end: usize,
    names: &'n [String],
    cols: Vec<u32>,
    nodes: usize,
    depth: usize,
    lookups: Vec<LookupRef>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn at(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end, |t| t.1)
    }

    fn node(&mut self, n: Node) -> Result<Node, Error> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(Error::Invalid("the expression is too long".into()));
        }
        Ok(n)
    }

    fn op(&self) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(o)) => Some(o),
            _ => None,
        }
    }

    fn compare(&mut self) -> Result<Node, Error> {
        let mut l = self.join()?;
        while let Some(o) = self.op() {
            let op = match o {
                "=" => Op::Eq,
                "<>" => Op::Ne,
                "<" => Op::Lt,
                "<=" => Op::Le,
                ">" => Op::Gt,
                ">=" => Op::Ge,
                _ => break,
            };
            self.pos += 1;
            let r = self.join()?;
            l = self.node(Node::Bin(op, Box::new(l), Box::new(r)))?;
        }
        Ok(l)
    }

    fn join(&mut self) -> Result<Node, Error> {
        let mut l = self.sum()?;
        while self.op() == Some("&") {
            self.pos += 1;
            let r = self.sum()?;
            l = self.node(Node::Bin(Op::Join, Box::new(l), Box::new(r)))?;
        }
        Ok(l)
    }

    fn sum(&mut self) -> Result<Node, Error> {
        let mut l = self.product()?;
        while let Some(o @ ("+" | "-")) = self.op() {
            self.pos += 1;
            let r = self.product()?;
            let op = if o == "+" { Op::Add } else { Op::Sub };
            l = self.node(Node::Bin(op, Box::new(l), Box::new(r)))?;
        }
        Ok(l)
    }

    fn product(&mut self) -> Result<Node, Error> {
        let mut l = self.unary()?;
        while let Some(o @ ("*" | "/")) = self.op() {
            self.pos += 1;
            let r = self.unary()?;
            let op = if o == "*" { Op::Mul } else { Op::Div };
            l = self.node(Node::Bin(op, Box::new(l), Box::new(r)))?;
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Node, Error> {
        match self.op() {
            Some("-") => {
                self.pos += 1;
                self.deeper()?;
                let n = self.unary()?;
                self.depth -= 1;
                self.node(Node::Neg(Box::new(n)))
            }
            Some("+") => {
                self.pos += 1;
                self.deeper()?;
                let n = self.unary();
                self.depth -= 1;
                n
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<Node, Error> {
        let mut l = self.postfix()?;
        while self.op() == Some("^") {
            self.pos += 1;
            let r = self.postfix()?;
            l = self.node(Node::Bin(Op::Pow, Box::new(l), Box::new(r)))?;
        }
        Ok(l)
    }

    fn postfix(&mut self) -> Result<Node, Error> {
        let mut n = self.atom()?;
        while self.op() == Some("%") {
            self.pos += 1;
            n = self.node(Node::Percent(Box::new(n)))?;
        }
        Ok(n)
    }

    fn deeper(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::Invalid("the expression is nested too deeply".into()));
        }
        Ok(())
    }

    fn column(&mut self, name: &str, at: usize) -> Result<Node, Error> {
        let col = if let Some(n) = name.strip_prefix('#') {
            match n.trim().parse::<u32>() {
                Ok(k) if k >= 1 && (k as usize) <= self.names.len() => k - 1,
                _ => return Err(bad(&format!("there is no column {name}"), at)),
            }
        } else if let Some(i) = self.names.iter().position(|t| t == name) {
            i as u32
        } else if let Some(i) = self
            .names
            .iter()
            .position(|t| t.to_lowercase() == name.to_lowercase())
        {
            i as u32
        } else {
            return Err(bad(&format!("there is no column named [{name}]"), at));
        };
        let k = match self.cols.iter().position(|&c| c == col) {
            Some(k) => k,
            None => {
                self.cols.push(col);
                self.cols.len() - 1
            }
        };
        self.node(Node::Col(k))
    }

    fn atom(&mut self) -> Result<Node, Error> {
        let at = self.at();
        let Some((tok, _)) = self.toks.get(self.pos).cloned() else {
            return Err(bad("the expression ends too soon", at));
        };
        self.pos += 1;
        match tok {
            Tok::Num(v) => self.node(Node::Num(v)),
            Tok::Text(t) => self.node(Node::Text(t)),
            Tok::Col(name) => self.column(&name, at),
            Tok::Open => {
                self.deeper()?;
                let n = self.compare()?;
                self.depth -= 1;
                if self.peek() != Some(&Tok::Close) {
                    return Err(bad("a ( is missing its )", self.at()));
                }
                self.pos += 1;
                Ok(n)
            }
            Tok::Name(name) => {
                let upper = name.to_uppercase();
                if self.peek() != Some(&Tok::Open) {
                    return match upper.as_str() {
                        "TRUE" => self.node(Node::Bool(true)),
                        "FALSE" => self.node(Node::Bool(false)),
                        _ => Err(bad(
                            &format!("unknown name {name} (columns are written in brackets, like [{name}])"),
                            at,
                        )),
                    };
                }
                let Some(&(_, f, min, max)) = FUNCS.iter().find(|f| f.0 == upper) else {
                    return Err(bad(&format!("unknown function {name}"), at));
                };
                self.pos += 1;
                self.deeper()?;
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::Close) {
                    self.pos += 1;
                } else {
                    loop {
                        args.push(self.compare()?);
                        match self.peek() {
                            Some(Tok::Comma) => self.pos += 1,
                            Some(Tok::Close) => {
                                self.pos += 1;
                                break;
                            }
                            _ => return Err(bad(&format!("{upper}( is missing its )"), self.at())),
                        }
                    }
                }
                self.depth -= 1;
                let n = args.len();
                if n < min as usize || (max != 255 && n > max as usize) {
                    let want = if min == max {
                        format!("{min}")
                    } else if max == 255 {
                        format!("at least {min}")
                    } else {
                        format!("{min} to {max}")
                    };
                    return Err(bad(
                        &format!(
                            "{upper} takes {want} value{}",
                            if want == "1" { "" } else { "s" }
                        ),
                        at,
                    ));
                }
                if f == Func::Ifs && n % 2 == 1 {
                    return Err(bad("IFS takes pairs of test and value", at));
                }
                if f == Func::Lookup {
                    let mut names = Vec::with_capacity(3);
                    for a in &args[1..4] {
                        let Node::Text(t) = a else {
                            return Err(bad(
                                "LOOKUP needs the tab and column names written as text in quotes",
                                at,
                            ));
                        };
                        names.push(t.clone());
                    }
                    if self.lookups.len() >= MAX_LOOKUPS {
                        return Err(bad(
                            &format!("an expression can use at most {MAX_LOOKUPS} LOOKUPs"),
                            at,
                        ));
                    }
                    let mut it = args.into_iter();
                    let key = Box::new(it.next().expect("4 or 5 values"));
                    let fallback = it.nth(3).map(Box::new);
                    let column = names.pop().expect("three names");
                    let key_col = names.pop().expect("three names");
                    let tab = names.pop().expect("three names");
                    self.lookups.push(LookupRef {
                        tab,
                        key: key_col,
                        column,
                    });
                    return self.node(Node::Lookup {
                        idx: self.lookups.len() - 1,
                        table: None,
                        key,
                        fallback,
                    });
                }
                let re = if matches!(
                    f,
                    Func::RegexMatch | Func::RegexExtract | Func::RegexReplace
                ) {
                    let Node::Text(p) = &args[1] else {
                        return Err(bad(
                            &format!("{upper} needs its pattern written as text in quotes"),
                            at,
                        ));
                    };
                    Some(Box::new(crate::find::build(p, true)?))
                } else {
                    None
                };
                self.node(Node::Call(f, args, re))
            }
            Tok::Op(o) => Err(bad(&format!("unexpected {o}"), at)),
            Tok::Close => Err(bad("unexpected )", at)),
            Tok::Comma => Err(bad("unexpected comma", at)),
        }
    }
}

impl Expr {
    /// Parse `src` against the column titles (one per logical column).
    pub fn parse(src: &str, names: &[String]) -> Result<Expr, Error> {
        let n = src.chars().count();
        if n > MAX_EXPR_CHARS {
            return Err(Error::Invalid(format!(
                "an expression can be at most {MAX_EXPR_CHARS} characters"
            )));
        }
        let src = src.trim_start().strip_prefix('=').unwrap_or(src);
        let toks = tokens(src)?;
        if toks.is_empty() {
            return Err(Error::Invalid("write an expression".into()));
        }
        let mut p = Parser {
            toks,
            pos: 0,
            end: src.chars().count(),
            names,
            cols: Vec::new(),
            nodes: 0,
            depth: 0,
            lookups: Vec::new(),
        };
        let root = p.compare()?;
        if p.pos < p.toks.len() {
            return Err(bad("unexpected text after the expression", p.at()));
        }
        Ok(Expr {
            root,
            cols: p.cols,
            lookups: p.lookups,
        })
    }

    /// The other tabs and columns this expression's LOOKUPs read.
    pub fn lookups(&self) -> &[LookupRef] {
        &self.lookups
    }

    /// Give the LOOKUPs their tables ([`Expr::lookups`] order). A LOOKUP
    /// without one gives `#REF!`.
    pub fn bind(&mut self, tables: &Lookups) {
        fn walk(n: &mut Node, t: &Lookups) {
            match n {
                Node::Neg(x) | Node::Percent(x) => walk(x, t),
                Node::Bin(_, a, b) => {
                    walk(a, t);
                    walk(b, t);
                }
                Node::Call(_, args, _) => args.iter_mut().for_each(|a| walk(a, t)),
                Node::Lookup {
                    idx,
                    table,
                    key,
                    fallback,
                } => {
                    *table = t.0.get(*idx).cloned().flatten();
                    walk(key, t);
                    if let Some(f) = fallback {
                        walk(f, t);
                    }
                }
                Node::Num(_) | Node::Text(_) | Node::Bool(_) | Node::Col(_) => {}
            }
        }
        walk(&mut self.root, tables);
    }

    /// The logical columns this expression reads, in `Ctx::cells` order.
    pub fn columns(&self) -> &[u32] {
        &self.cols
    }

    pub fn eval<'a>(&'a self, ctx: &Ctx<'a>) -> Value<'a> {
        eval(&self.root, ctx)
    }

    /// The result as cell text.
    pub fn text(&self, ctx: &Ctx<'_>) -> String {
        to_text(&self.eval(ctx)).into_owned()
    }

    /// True when the expression gives TRUE (errors and other values are
    /// not true).
    pub fn holds(&self, ctx: &Ctx<'_>) -> bool {
        matches!(to_bool(&self.eval(ctx)), Ok(true))
    }
}

// ---- conversions ----

/// A number from a value (dates count as their day number). Text that holds
/// a number is read as one; text that holds a date gives the date.
fn numeric(v: &Value, read: ReadOptions) -> Result<(f64, bool), Fault> {
    match v {
        Value::Blank => Ok((0.0, false)),
        Value::Num(n) => Ok((*n, false)),
        Value::Bool(b) => Ok((f64::from(u8::from(*b)), false)),
        Value::Date(d) => Ok((*d, true)),
        Value::Err(e) => Err(*e),
        Value::Text(t) => {
            if t.trim().is_empty() {
                Ok((0.0, false))
            } else if let Some(n) = parse_number(t, read) {
                Ok((n, false))
            } else if let Some(d) = parse_date(t, read) {
                Ok((d, true))
            } else {
                Err(Fault::Value)
            }
        }
    }
}

fn num(v: &Value, read: ReadOptions) -> Result<f64, Fault> {
    numeric(v, read).map(|x| x.0)
}

fn date(v: &Value, read: ReadOptions) -> Result<f64, Fault> {
    match v {
        Value::Date(d) => Ok(*d),
        Value::Text(t) => match parse_date(t, read) {
            Some(d) => Ok(d),
            None => num(v, read),
        },
        _ => num(v, read),
    }
}

fn date_text(d: f64) -> String {
    let day = d.floor();
    let (y, m, dd) = civil_from_days(day as i64);
    let secs = ((d - day) * 86_400.0).round() as i64;
    if secs <= 0 || secs >= 86_400 {
        format!("{y:04}-{m:02}-{dd:02}")
    } else if secs % 60 == 0 {
        format!(
            "{y:04}-{m:02}-{dd:02} {:02}:{:02}",
            secs / 3600,
            secs / 60 % 60
        )
    } else {
        format!(
            "{y:04}-{m:02}-{dd:02} {:02}:{:02}:{:02}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        )
    }
}

pub fn to_text<'a>(v: &Value<'a>) -> Cow<'a, str> {
    match v {
        Value::Blank => Cow::Borrowed(""),
        Value::Num(n) => Cow::Owned(format_number(*n, false)),
        Value::Text(t) => t.clone(),
        Value::Bool(true) => Cow::Borrowed("TRUE"),
        Value::Bool(false) => Cow::Borrowed("FALSE"),
        Value::Date(d) => Cow::Owned(date_text(*d)),
        Value::Err(e) => Cow::Borrowed(e.code()),
    }
}

fn to_bool(v: &Value) -> Result<bool, Fault> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Blank => Ok(false),
        Value::Num(n) | Value::Date(n) => Ok(*n != 0.0),
        Value::Err(e) => Err(*e),
        Value::Text(t) => {
            let t = t.trim();
            if t.eq_ignore_ascii_case("true") {
                Ok(true)
            } else if t.eq_ignore_ascii_case("false") || t.is_empty() {
                Ok(false)
            } else {
                t.parse::<f64>().map(|n| n != 0.0).map_err(|_| Fault::Value)
            }
        }
    }
}

fn is_blank(v: &Value) -> bool {
    match v {
        Value::Blank => true,
        Value::Text(t) => t.is_empty(),
        _ => false,
    }
}

fn ok_num<'a>(n: f64) -> Value<'a> {
    if n.is_finite() {
        Value::Num(n)
    } else {
        Value::Err(Fault::Num)
    }
}

fn text_val<'a>(s: String) -> Value<'a> {
    if s.len() > MAX_TEXT {
        Value::Err(Fault::Value)
    } else {
        Value::Text(Cow::Owned(s))
    }
}

macro_rules! tryv {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(f) => return Value::Err(f),
        }
    };
}

/// Compare two values the way a spreadsheet does: numbers (and dates) by
/// value, text without regard to case, blank as 0 or "".
fn compare(a: &Value, b: &Value, read: ReadOptions) -> Result<std::cmp::Ordering, Fault> {
    use std::cmp::Ordering;
    if let Value::Err(e) = a {
        return Err(*e);
    }
    if let Value::Err(e) = b {
        return Err(*e);
    }
    let as_num = |v: &Value| -> Option<f64> {
        match v {
            Value::Num(n) | Value::Date(n) => Some(*n),
            Value::Text(t) => parse_number(t, read),
            _ => None,
        }
    };
    let is_date = |v: &Value| matches!(v, Value::Date(_));
    let (na, nb) = (as_num(a), as_num(b));
    let ord = |x: f64, y: f64| x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    if let (Some(x), Some(y)) = (na, nb) {
        return Ok(ord(x, y));
    }
    if is_date(a) || is_date(b) {
        if let (Ok(x), Ok(y)) = (date(a, read), date(b, read)) {
            return Ok(ord(x, y));
        }
    }
    match (a, b) {
        (Value::Bool(x), Value::Bool(y)) => return Ok(x.cmp(y)),
        (Value::Blank, Value::Blank) => return Ok(Ordering::Equal),
        (Value::Blank, _) if nb.is_some() => return Ok(ord(0.0, nb.unwrap_or(0.0))),
        (_, Value::Blank) if na.is_some() => return Ok(ord(na.unwrap_or(0.0), 0.0)),
        (Value::Blank, Value::Bool(y)) => return Ok(false.cmp(y)),
        (Value::Bool(x), Value::Blank) => return Ok(x.cmp(&false)),
        _ => {}
    }
    let (x, y) = (to_text(a).to_lowercase(), to_text(b).to_lowercase());
    Ok(x.cmp(&y))
}

fn round_to(n: f64, d: f64, how: i8) -> f64 {
    let d = d.trunc().clamp(-15.0, 15.0) as i32;
    let f = 10f64.powi(d);
    let x = n * f;
    // Round a hair past the binary value (2.675 is 2.67499999...).
    let nudge = |x: f64| x + x.signum() * x.abs() * 1e-12;
    let r = match how {
        0 => nudge(x).round(),
        1 => {
            let t = nudge(x).trunc();
            if (x - t).abs() > 1e-9 * x.abs().max(1.0) {
                t + x.signum()
            } else {
                t
            }
        }
        _ => nudge(x).trunc(),
    };
    r / f
}

fn chars_len(s: &str) -> usize {
    s.chars().count()
}

fn slice_chars(s: &str, start: usize, len: usize) -> &str {
    let mut it = s
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()));
    let a = it.nth(start).unwrap_or(s.len());
    let b = s[a..]
        .char_indices()
        .map(|(i, _)| a + i)
        .chain(std::iter::once(s.len()))
        .nth(len)
        .unwrap_or(s.len());
    &s[a..b]
}

fn count_arg(v: &Value, read: ReadOptions) -> Result<usize, Fault> {
    let n = num(v, read)?;
    if n < 0.0 {
        return Err(Fault::Value);
    }
    Ok(n.trunc().min(MAX_TEXT as f64) as usize)
}

fn eval<'a>(n: &'a Node, ctx: &Ctx<'a>) -> Value<'a> {
    let read = ctx.read;
    match n {
        Node::Num(v) => Value::Num(*v),
        Node::Text(t) => Value::Text(Cow::Borrowed(t.as_str())),
        Node::Bool(b) => Value::Bool(*b),
        Node::Col(k) => match ctx
            .slots
            .get(*k)
            .copied()
            .flatten()
            .and_then(|s| ctx.cells.get(s))
        {
            Some(c) if !c.is_empty() => Value::Text(Cow::Borrowed(c.as_ref())),
            _ => Value::Blank,
        },
        Node::Neg(x) => {
            let v = eval(x, ctx);
            let (n, d) = tryv!(numeric(&v, read));
            if d {
                Value::Err(Fault::Value)
            } else {
                ok_num(-n)
            }
        }
        Node::Percent(x) => ok_num(tryv!(num(&eval(x, ctx), read)) / 100.0),
        Node::Bin(op, l, r) => {
            let a = eval(l, ctx);
            let b = eval(r, ctx);
            binary(*op, &a, &b, read)
        }
        Node::Call(f, args, re) => call(*f, args, re.as_deref(), ctx),
        Node::Lookup {
            table,
            key,
            fallback,
            ..
        } => {
            let Some(t) = table else {
                return Value::Err(Fault::Ref);
            };
            let k = eval(key, ctx);
            if let Value::Err(e) = k {
                return Value::Err(e);
            }
            match t.get(&to_text(&k)) {
                Some("") => Value::Blank,
                Some(v) => Value::Text(Cow::Borrowed(v)),
                None => fallback
                    .as_ref()
                    .map_or(Value::Err(Fault::Na), |f| eval(f, ctx)),
            }
        }
    }
}

fn binary<'a>(op: Op, a: &Value<'a>, b: &Value<'a>, read: ReadOptions) -> Value<'a> {
    use std::cmp::Ordering::*;
    match op {
        Op::Join => {
            if let Value::Err(e) = a {
                return Value::Err(*e);
            }
            if let Value::Err(e) = b {
                return Value::Err(*e);
            }
            let (x, y) = (to_text(a), to_text(b));
            if x.len().saturating_add(y.len()) > MAX_TEXT {
                return Value::Err(Fault::Value);
            }
            if x.is_empty() {
                return Value::Text(y);
            }
            if y.is_empty() {
                return Value::Text(x);
            }
            text_val(format!("{x}{y}"))
        }
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
            let o = tryv!(compare(a, b, read));
            Value::Bool(match op {
                Op::Eq => o == Equal,
                Op::Ne => o != Equal,
                Op::Lt => o == Less,
                Op::Le => o != Greater,
                Op::Gt => o == Greater,
                _ => o != Less,
            })
        }
        _ => {
            let (x, xd) = tryv!(numeric(a, read));
            let (y, yd) = tryv!(numeric(b, read));
            match op {
                Op::Add => {
                    let v = x + y;
                    if xd != yd && v.is_finite() {
                        Value::Date(v)
                    } else {
                        ok_num(v)
                    }
                }
                Op::Sub => {
                    let v = x - y;
                    if xd && !yd && v.is_finite() {
                        Value::Date(v)
                    } else {
                        ok_num(v)
                    }
                }
                Op::Mul => ok_num(x * y),
                Op::Div => {
                    if y == 0.0 {
                        Value::Err(Fault::Div0)
                    } else {
                        ok_num(x / y)
                    }
                }
                _ => {
                    if x == 0.0 && y < 0.0 {
                        Value::Err(Fault::Div0)
                    } else {
                        ok_num(x.powf(y))
                    }
                }
            }
        }
    }
}

fn call<'a>(f: Func, args: &'a [Node], re: Option<&'a Regex>, ctx: &Ctx<'a>) -> Value<'a> {
    let read = ctx.read;
    let arg = |i: usize| eval(&args[i], ctx);
    let opt = |i: usize| args.get(i).map(|n| eval(n, ctx));
    let text = |i: usize| -> Result<Cow<'a, str>, Fault> {
        match arg(i) {
            Value::Err(e) => Err(e),
            v => Ok(to_text(&v)),
        }
    };
    let number = |i: usize| num(&arg(i), read);
    match f {
        Func::If => {
            if tryv!(to_bool(&arg(0))) {
                arg(1)
            } else {
                opt(2).unwrap_or(Value::Bool(false))
            }
        }
        Func::Ifs => {
            for pair in args.chunks(2) {
                if tryv!(to_bool(&eval(&pair[0], ctx))) {
                    return eval(&pair[1], ctx);
                }
            }
            Value::Err(Fault::Na)
        }
        Func::IfError => match arg(0) {
            Value::Err(_) => arg(1),
            v => v,
        },
        Func::And | Func::Or => {
            let want = f == Func::Or;
            for a in args {
                if tryv!(to_bool(&eval(a, ctx))) == want {
                    return Value::Bool(want);
                }
            }
            Value::Bool(!want)
        }
        Func::Not => Value::Bool(!tryv!(to_bool(&arg(0)))),
        Func::IsBlank => Value::Bool(is_blank(&arg(0))),
        Func::IsNumber => Value::Bool(match arg(0) {
            Value::Num(_) => true,
            Value::Text(t) => parse_number(&t, read).is_some(),
            _ => false,
        }),
        Func::IsText => Value::Bool(match arg(0) {
            Value::Text(t) => !t.is_empty() && parse_number(&t, read).is_none(),
            _ => false,
        }),
        Func::IsDate => Value::Bool(match arg(0) {
            Value::Date(_) => true,
            Value::Text(t) => parse_number(&t, read).is_none() && parse_date(&t, read).is_some(),
            _ => false,
        }),
        Func::IsError => Value::Bool(matches!(arg(0), Value::Err(_))),
        Func::Len => Value::Num(chars_len(&tryv!(text(0))) as f64),
        Func::Left | Func::Right => {
            let s = tryv!(text(0));
            let k = match opt(1) {
                Some(v) => tryv!(count_arg(&v, read)),
                None => 1,
            };
            let len = chars_len(&s);
            let k = k.min(len);
            let part = if f == Func::Left {
                slice_chars(&s, 0, k)
            } else {
                slice_chars(&s, len - k, k)
            };
            Value::Text(Cow::Owned(part.to_string()))
        }
        Func::Mid => {
            let s = tryv!(text(0));
            let start = tryv!(number(1));
            let k = tryv!(count_arg(&arg(2), read));
            if start < 1.0 {
                return Value::Err(Fault::Value);
            }
            let start = (start.trunc() as usize).saturating_sub(1);
            Value::Text(Cow::Owned(slice_chars(&s, start, k).to_string()))
        }
        Func::Upper => Value::Text(Cow::Owned(tryv!(text(0)).to_uppercase())),
        Func::Lower => Value::Text(Cow::Owned(tryv!(text(0)).to_lowercase())),
        Func::Proper => {
            let s = tryv!(text(0));
            let mut out = String::with_capacity(s.len());
            let mut start = true;
            for c in s.chars() {
                if c.is_alphabetic() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = !c.is_alphanumeric() && c != '\'';
                }
            }
            Value::Text(Cow::Owned(out))
        }
        Func::Trim => {
            let s = tryv!(text(0));
            Value::Text(Cow::Owned(
                s.split(' ')
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ))
        }
        Func::Concat => {
            let mut out = String::new();
            for i in 0..args.len() {
                let t = tryv!(text(i));
                if out.len() + t.len() > MAX_TEXT {
                    return Value::Err(Fault::Value);
                }
                out.push_str(&t);
            }
            Value::Text(Cow::Owned(out))
        }
        Func::TextJoin => {
            let sep = tryv!(text(0));
            let skip = tryv!(to_bool(&arg(1)));
            let mut out = String::new();
            let mut first = true;
            for i in 2..args.len() {
                let t = tryv!(text(i));
                if skip && t.is_empty() {
                    continue;
                }
                if out.len() + sep.len() + t.len() > MAX_TEXT {
                    return Value::Err(Fault::Value);
                }
                if !first {
                    out.push_str(&sep);
                }
                first = false;
                out.push_str(&t);
            }
            Value::Text(Cow::Owned(out))
        }
        Func::Substitute => {
            let s = tryv!(text(0));
            let old = tryv!(text(1));
            let new = tryv!(text(2));
            if old.is_empty() {
                return Value::Text(s);
            }
            match opt(3) {
                None => {
                    // Work out the size first: replacing many short matches
                    // with long text could otherwise build a huge string.
                    let hits = s.matches(old.as_ref()).count();
                    let size =
                        (s.len() - hits * old.len()).saturating_add(hits.saturating_mul(new.len()));
                    if size > MAX_TEXT {
                        return Value::Err(Fault::Value);
                    }
                    Value::Text(Cow::Owned(s.replace(old.as_ref(), &new)))
                }
                Some(v) => {
                    let k = tryv!(count_arg(&v, read));
                    if k == 0 {
                        return Value::Err(Fault::Value);
                    }
                    match s.match_indices(old.as_ref()).nth(k - 1) {
                        Some((i, _)) if s.len() - old.len() + new.len() <= MAX_TEXT => Value::Text(
                            Cow::Owned(format!("{}{}{}", &s[..i], new, &s[i + old.len()..])),
                        ),
                        Some(_) => Value::Err(Fault::Value),
                        None => Value::Text(s),
                    }
                }
            }
        }
        Func::Replace => {
            let s = tryv!(text(0));
            let start = tryv!(number(1));
            let k = tryv!(count_arg(&arg(2), read));
            let new = tryv!(text(3));
            if start < 1.0 {
                return Value::Err(Fault::Value);
            }
            let start = (start.trunc() as usize - 1).min(chars_len(&s));
            let head = slice_chars(&s, 0, start);
            let tail = slice_chars(&s, start + k, usize::MAX);
            if head.len() + new.len() + tail.len() > MAX_TEXT {
                return Value::Err(Fault::Value);
            }
            Value::Text(Cow::Owned(format!("{head}{new}{tail}")))
        }
        Func::Find | Func::Search => {
            let needle = tryv!(text(0));
            let hay = tryv!(text(1));
            let start = match opt(2) {
                Some(v) => {
                    let s = tryv!(num(&v, read));
                    if s < 1.0 {
                        return Value::Err(Fault::Value);
                    }
                    s.trunc() as usize - 1
                }
                None => 0,
            };
            let (needle, hay) = if f == Func::Search {
                (
                    Cow::Owned(needle.to_lowercase()),
                    Cow::Owned(hay.to_lowercase()),
                )
            } else {
                (needle, hay)
            };
            if start > chars_len(&hay) {
                return Value::Err(Fault::Value);
            }
            let from = slice_chars(&hay, 0, start).len();
            match hay[from..].find(needle.as_ref()) {
                Some(i) => Value::Num((start + chars_len(&hay[from..from + i]) + 1) as f64),
                None => Value::Err(Fault::Value),
            }
        }
        Func::Exact => Value::Bool(tryv!(text(0)) == tryv!(text(1))),
        Func::Rept => {
            let s = tryv!(text(0));
            let k = tryv!(count_arg(&arg(1), read));
            if s.len().saturating_mul(k) > MAX_TEXT {
                return Value::Err(Fault::Value);
            }
            Value::Text(Cow::Owned(s.repeat(k)))
        }
        Func::ValueF => match arg(0) {
            Value::Text(t) => match parse_number(&t, read) {
                Some(n) => Value::Num(n),
                None => match parse_date(&t, read) {
                    Some(d) => Value::Num(d),
                    None => Value::Err(Fault::Value),
                },
            },
            v => ok_num(tryv!(num(&v, read))),
        },
        Func::Fixed => {
            let n = tryv!(number(0));
            let d = match opt(1) {
                Some(v) => tryv!(num(&v, read)).trunc().clamp(-15.0, 15.0),
                None => 2.0,
            };
            let plain = match opt(2) {
                Some(v) => tryv!(to_bool(&v)),
                None => false,
            };
            let r = round_to(n, d, 0);
            let s = format!("{:.*}", d.max(0.0) as usize, r.abs());
            let (int, frac) = s
                .split_once('.')
                .map_or((s.as_str(), None), |(a, b)| (a, Some(b)));
            let mut out = String::new();
            if r < 0.0 {
                out.push('-');
            }
            if plain {
                out.push_str(int);
            } else {
                for (i, c) in int.chars().enumerate() {
                    if i > 0 && (int.len() - i) % 3 == 0 {
                        out.push(',');
                    }
                    out.push(c);
                }
            }
            if let Some(fr) = frac {
                out.push('.');
                out.push_str(fr);
            }
            Value::Text(Cow::Owned(out))
        }
        Func::Round | Func::RoundUp | Func::RoundDown => {
            let n = tryv!(number(0));
            let d = match opt(1) {
                Some(v) => tryv!(num(&v, read)),
                None => 0.0,
            };
            let how = match f {
                Func::Round => 0,
                Func::RoundUp => 1,
                _ => 2,
            };
            ok_num(round_to(n, d, how))
        }
        Func::Int => ok_num(tryv!(number(0)).floor()),
        Func::Abs => ok_num(tryv!(number(0)).abs()),
        Func::Sign => {
            let n = tryv!(number(0));
            Value::Num(if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            })
        }
        Func::Mod => {
            let a = tryv!(number(0));
            let b = tryv!(number(1));
            if b == 0.0 {
                Value::Err(Fault::Div0)
            } else {
                ok_num(a - b * (a / b).floor())
            }
        }
        Func::Power => {
            let a = tryv!(number(0));
            let b = tryv!(number(1));
            binary(Op::Pow, &Value::Num(a), &Value::Num(b), read)
        }
        Func::Sqrt => {
            let n = tryv!(number(0));
            if n < 0.0 {
                Value::Err(Fault::Num)
            } else {
                ok_num(n.sqrt())
            }
        }
        Func::Exp => ok_num(tryv!(number(0)).exp()),
        Func::Ln | Func::Log10 => {
            let n = tryv!(number(0));
            if n <= 0.0 {
                Value::Err(Fault::Num)
            } else if f == Func::Ln {
                ok_num(n.ln())
            } else {
                ok_num(n.log10())
            }
        }
        Func::Min | Func::Max | Func::Sum | Func::Average => {
            let mut seen = 0usize;
            let mut acc = match f {
                Func::Min => f64::INFINITY,
                Func::Max => f64::NEG_INFINITY,
                _ => 0.0,
            };
            for a in args {
                let v = eval(a, ctx);
                if is_blank(&v) {
                    continue;
                }
                let x = tryv!(num(&v, read));
                seen += 1;
                acc = match f {
                    Func::Min => acc.min(x),
                    Func::Max => acc.max(x),
                    _ => acc + x,
                };
            }
            match f {
                Func::Min | Func::Max if seen == 0 => Value::Num(0.0),
                Func::Average if seen == 0 => Value::Err(Fault::Div0),
                Func::Average => ok_num(acc / seen as f64),
                _ => ok_num(acc),
            }
        }
        Func::Date => {
            let y = tryv!(number(0)).trunc();
            let m = tryv!(number(1)).trunc();
            let d = tryv!(number(2)).trunc();
            if !(-10_000.0..=10_000.0).contains(&y) || m.abs() > 120_000.0 || d.abs() > 4_000_000.0
            {
                return Value::Err(Fault::Num);
            }
            let mi = m as i64 - 1;
            let yy = y as i64 + mi.div_euclid(12);
            let mm = mi.rem_euclid(12) as u32 + 1;
            Value::Date((days_from_civil(yy, mm, 1) + d as i64 - 1) as f64)
        }
        Func::Year | Func::Month | Func::Day | Func::Weekday => {
            let d = tryv!(date(&arg(0), read));
            if !(-4e6..=4e6).contains(&d) {
                return Value::Err(Fault::Num);
            }
            let day = d.floor() as i64;
            let (y, m, dd) = civil_from_days(day);
            Value::Num(match f {
                Func::Year => y as f64,
                Func::Month => m as f64,
                Func::Day => dd as f64,
                _ => ((day + 4).rem_euclid(7) + 1) as f64,
            })
        }
        Func::Hour | Func::Minute => {
            let d = tryv!(date(&arg(0), read));
            let secs = ((d - d.floor()) * 86_400.0).round() as i64;
            Value::Num(if f == Func::Hour {
                (secs / 3600) as f64
            } else {
                (secs / 60 % 60) as f64
            })
        }
        Func::Days => {
            let a = tryv!(date(&arg(0), read));
            let b = tryv!(date(&arg(1), read));
            ok_num(a.floor() - b.floor())
        }
        Func::Today => Value::Date(ctx.now.floor()),
        Func::Now => Value::Date(ctx.now),
        Func::Row => Value::Num(ctx.row as f64),
        // Made into its own node when parsed.
        Func::Lookup => Value::Err(Fault::Ref),
        Func::Coalesce => {
            for a in args {
                let v = eval(a, ctx);
                if !is_blank(&v) {
                    return v;
                }
            }
            Value::Blank
        }
        Func::RegexMatch | Func::RegexExtract | Func::RegexReplace => {
            let re = re.expect("compiled at parse time");
            let s = tryv!(text(0));
            match f {
                Func::RegexMatch => Value::Bool(re.is_match(&s)),
                Func::RegexExtract => {
                    let g = match opt(2) {
                        Some(v) => tryv!(count_arg(&v, read)),
                        None => 0,
                    };
                    match re.captures(&s).and_then(|c| c.get(g)) {
                        Some(m) => Value::Text(Cow::Owned(m.as_str().to_string())),
                        None => Value::Err(Fault::Na),
                    }
                }
                _ => {
                    let rep = tryv!(text(2));
                    let pieces = replacement(&rep, re);
                    // Many group references would make every match slow to
                    // measure and write (work grows with matches times
                    // references).
                    if pieces
                        .iter()
                        .filter(|p| matches!(p, Piece::Group(_)))
                        .count()
                        > MAX_REFS
                    {
                        return Value::Err(Fault::Value);
                    }
                    // Replace match by match. Each match's replacement is
                    // measured before it is written (`$0` repeated could
                    // make one match huge), so nothing past the limit is
                    // ever built.
                    let mut out = String::new();
                    let mut last = 0;
                    for caps in re.captures_iter(&s) {
                        let m = caps.get(0).expect("whole match");
                        let group = |g: usize| caps.get(g).map_or("", |x| x.as_str());
                        let size = pieces.iter().fold(0usize, |n, p| {
                            n.saturating_add(match p {
                                Piece::Text(t) => t.len(),
                                Piece::Group(g) => group(*g).len(),
                            })
                        });
                        if out
                            .len()
                            .saturating_add(m.start() - last)
                            .saturating_add(size)
                            > MAX_TEXT
                        {
                            return Value::Err(Fault::Value);
                        }
                        out.push_str(&s[last..m.start()]);
                        for p in &pieces {
                            match p {
                                Piece::Text(t) => out.push_str(t),
                                Piece::Group(g) => out.push_str(group(*g)),
                            }
                        }
                        last = m.end();
                    }
                    if out.len() + (s.len() - last) > MAX_TEXT {
                        return Value::Err(Fault::Value);
                    }
                    out.push_str(&s[last..]);
                    Value::Text(Cow::Owned(out))
                }
            }
        }
    }
}

/// A piece of REGEXREPLACE's replacement text.
enum Piece {
    Text(String),
    /// A group by number (`$1`, `${2}`; `$0` is the whole match).
    Group(usize),
}

/// Read replacement text the way the regex crate does: `$$` is `$`;
/// `$name` takes the longest run of letters, digits and `_` (all digits: a
/// group number); `${name}` is braced. Unknown groups give empty text.
fn replacement(rep: &str, re: &Regex) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let by_name = |name: &str| -> Option<usize> {
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()) {
            name.parse().ok()
        } else {
            re.capture_names().position(|n| n == Some(name))
        }
    };
    let b = rep.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            let next = rep[i..].find('$').map_or(rep.len(), |k| i + k);
            text.push_str(&rep[i..next]);
            i = next;
            continue;
        }
        if b.get(i + 1) == Some(&b'$') {
            text.push('$');
            i += 2;
            continue;
        }
        let (name, end) = if b.get(i + 1) == Some(&b'{') {
            match rep[i + 2..].find('}') {
                Some(k) => (&rep[i + 2..i + 2 + k], i + 3 + k),
                None => {
                    text.push('$');
                    i += 1;
                    continue;
                }
            }
        } else {
            let k = rep[i + 1..]
                .bytes()
                .position(|c| !(c.is_ascii_alphanumeric() || c == b'_'))
                .unwrap_or(rep.len() - i - 1);
            if k == 0 {
                text.push('$');
                i += 1;
                continue;
            }
            (&rep[i + 1..i + 1 + k], i + 1 + k)
        };
        if !text.is_empty() {
            out.push(Piece::Text(std::mem::take(&mut text)));
        }
        if let Some(g) = by_name(name) {
            out.push(Piece::Group(g));
        }
        i = end;
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    out
}

// ---- computed columns ----

/// Add a computed column, or write over an existing one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ComputeSpec {
    pub expr: String,
    /// Title of a new column (first row, with `keep_first`).
    pub title: String,
    /// Write over this logical column instead of adding one.
    pub target: Option<u32>,
    /// Where a new column goes (logical position); `None`: at the end.
    pub at: Option<u32>,
    /// The first row holds titles: names come from it and it is not computed.
    pub keep_first: bool,
    pub read: ReadOptions,
    /// Local date and time as days since 1970 (TODAY, NOW).
    pub now: f64,
    /// Tables for LOOKUP, filled in by the app (never saved or sent).
    #[serde(skip)]
    pub tables: Lookups,
}

impl Default for ComputeSpec {
    fn default() -> Self {
        ComputeSpec {
            expr: String::new(),
            title: String::new(),
            target: None,
            at: None,
            keep_first: true,
            read: ReadOptions::default(),
            now: 0.0,
            tables: Lookups::default(),
        }
    }
}

/// What a computed column did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeReport {
    pub cells: u64,
    /// Cells that came out as an error code (`#VALUE!` and the like).
    pub errors: u64,
}

/// The first results, for the dialog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComputePreview {
    /// Shown row number (from 1) and result.
    pub rows: Vec<(u64, String)>,
    pub errors: u64,
}

impl Document {
    /// Column titles for expressions: the first row with `keep_first`, else
    /// "Column N".
    pub fn expr_names(&self, keep_first: bool) -> Vec<String> {
        let cols: Vec<u32> = (0..self.col_count()).collect();
        self.titles(&cols, keep_first)
    }

    /// Parse an expression against this document's columns.
    pub fn parse_expr(&self, src: &str, keep_first: bool) -> Result<Expr, Error> {
        Expr::parse(src, &self.expr_names(keep_first))
    }

    /// The spec's expression, parsed, with its LOOKUP tables bound.
    fn compute_expr(&self, spec: &ComputeSpec) -> Result<Expr, Error> {
        let mut e = self.parse_expr(&spec.expr, spec.keep_first)?;
        e.bind(&spec.tables);
        Ok(e)
    }

    /// Results for the first `count` shown data rows.
    pub fn preview_compute(&self, spec: &ComputeSpec, count: u64) -> Result<ComputePreview, Error> {
        let e = self.compute_expr(spec)?;
        check_cols(self, e.columns())?;
        let reader = Cols::new(self, e.columns());
        let first = u64::from(spec.keep_first && self.row_count() > 0);
        let end = self.row_count().min(first + count.min(MAX_PREVIEW));
        let mut out = ComputePreview::default();
        let mut buf = Vec::new();
        for r in first..end {
            let id = self.id_at(r).ok_or(Error::OutOfRange)?;
            reader.read(self, id, &mut buf);
            let ctx = Ctx {
                cells: &buf,
                slots: reader.slots(),
                row: self.row_number(r).unwrap_or(r) + 1,
                now: spec.now,
                read: spec.read,
            };
            let v = e.eval(&ctx);
            if matches!(v, Value::Err(_)) {
                out.errors += 1;
            }
            let mut t = to_text(&v).into_owned();
            if t.len() > 1000 {
                let mut cut = 1000;
                while !t.is_char_boundary(cut) {
                    cut -= 1;
                }
                t.truncate(cut);
                t.push('…');
            }
            out.rows.push((r + 1, t));
        }
        Ok(out)
    }

    /// Plan a computed column over every row (hidden ones too).
    pub fn plan_compute(
        &self,
        spec: &ComputeSpec,
        progress: &Progress,
    ) -> Result<(ColPlan, ComputeReport), Error> {
        self.ready_for_bulk()?;
        let e = self.compute_expr(spec)?;
        check_cols(self, e.columns())?;
        if let Some(t) = spec.target {
            check_cols(self, &[t])?;
        }
        let ncols = self.col_count();
        if spec.target.is_none() && ncols >= crate::document::MAX_MAPPED_COLS {
            return Err(Error::Invalid(
                "the table has as many columns as it can hold".into(),
            ));
        }
        let at = spec.at.unwrap_or(ncols).min(ncols);
        // Read the expression's columns, then the target (to skip unchanged
        // cells).
        let mut read_cols = e.columns().to_vec();
        if let Some(t) = spec.target {
            read_cols.push(t);
        }
        let reader = Cols::new(self, &read_cols);
        let nexpr = e.columns().len();
        let first = u64::from(spec.keep_first && self.total_rows() > 0);
        let spent = AtomicUsize::new(0);
        let budget = crate::find::split_budget(self);
        let errors = AtomicU64::new(0);
        let slot = spec.target.unwrap_or(at);
        let changes: Vec<(u32, Vec<(u32, String)>)> =
            self.par_all_rows(first, progress, |p, id, buf| {
                reader.read(self, id, buf);
                let ctx = Ctx {
                    cells: buf,
                    slots: &reader.slots()[..nexpr],
                    row: p + 1,
                    now: spec.now,
                    read: spec.read,
                };
                let v = e.eval(&ctx);
                if matches!(v, Value::Err(_)) {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
                let t = to_text(&v);
                let unchanged = match spec.target {
                    Some(_) => reader.get(buf, nexpr) == t.as_ref(),
                    None => t.is_empty(),
                };
                if unchanged {
                    return Ok(None);
                }
                if !crate::find::charge_within(&spent, t.len(), budget) {
                    return Err(Error::OutOfMemory(
                        "this computed column would make too many new cells at once",
                    ));
                }
                Ok(Some((id, vec![(slot, t.into_owned())])))
            })?;
        let mut plan = ColPlan::new(self);
        if spec.target.is_none() {
            plan.insert = Some((at, 1));
            plan.cols = 1;
            if first == 1 {
                let id = self.doc_id_at(0).expect("a first row");
                let title = if spec.title.trim().is_empty() {
                    "Computed".to_string()
                } else {
                    spec.title.clone()
                };
                plan.changes.push((id, vec![(at, title)]));
                plan.cells += 1;
            }
        }
        plan.cells += changes.len() as u64;
        plan.changes.extend(changes);
        let report = ComputeReport {
            cells: plan.cells,
            errors: errors.into_inner(),
        };
        Ok((plan, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    fn run(src: &str, cells: &[&str]) -> String {
        let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let e = Expr::parse(src, &names).unwrap();
        let all: Vec<Cow<str>> = cells.iter().map(|c| Cow::Borrowed(*c)).collect();
        let slots: Vec<Option<usize>> = e
            .columns()
            .iter()
            .map(|&c| Some(c as usize).filter(|&c| c < all.len()))
            .collect();
        let ctx = Ctx {
            cells: &all,
            slots: &slots,
            row: 7,
            now: 19_000.5,
            read: ReadOptions::default(),
        };
        e.text(&ctx)
    }

    #[test]
    fn arithmetic_and_text() {
        assert_eq!(run("[a] * [b]", &["3", "4"]), "12");
        assert_eq!(run("=[a] + [b] * 2", &["1", "2.5"]), "6");
        assert_eq!(run("([a] + [b]) * 2", &["1", "2"]), "6");
        assert_eq!(run("-[a]^2", &["3"]), "-9");
        assert_eq!(run("2^3^2", &[]), "64");
        assert_eq!(run("10%", &[]), "0.1");
        assert_eq!(run("0.1 + 0.2", &[]), "0.3");
        assert_eq!(
            run("[a] & \" \" & [b]", &["Ada", "Lovelace"]),
            "Ada Lovelace"
        );
        assert_eq!(run("[a] / [b]", &["1", "0"]), "#DIV/0!");
        assert_eq!(run("[a] + 1", &["x"]), "#VALUE!");
        assert_eq!(run("[a] + 1", &["$1,200.50"]), "1201.5");
        assert_eq!(run("[a]", &["007"]), "007");
        assert_eq!(run("[a] + [c]", &["1", "2", ""]), "1");
        assert_eq!(run("\"say \"\"hi\"\"\"", &[]), "say \"hi\"");
        assert_eq!(run("[#2]", &["x", "y"]), "y");
    }

    #[test]
    fn logic_and_compare() {
        assert_eq!(run("IF([a] >= 18, \"adult\", \"minor\")", &["20"]), "adult");
        assert_eq!(run("if([a] >= 18, \"adult\", \"minor\")", &["9"]), "minor");
        assert_eq!(run("[a] = \"ABC\"", &["abc"]), "TRUE");
        assert_eq!(run("[a] = 10", &["10.0"]), "TRUE");
        assert_eq!(run("[a] < [b]", &["9", "10"]), "TRUE");
        assert_eq!(run("[a] < [b]", &["b", "a"]), "FALSE");
        assert_eq!(run("[a] = 0", &[""]), "TRUE");
        assert_eq!(run("AND([a] > 1, [b] = \"x\")", &["2", "X"]), "TRUE");
        assert_eq!(run("OR(FALSE, NOT(TRUE))", &[]), "FALSE");
        assert_eq!(
            run("IFS([a] > 10, \"big\", [a] > 5, \"mid\")", &["7"]),
            "mid"
        );
        assert_eq!(run("IFS([a] > 10, \"big\")", &["7"]), "#N/A");
        assert_eq!(run("IFERROR(1/0, \"none\")", &[]), "none");
        assert_eq!(run("ISBLANK([a])", &[""]), "TRUE");
        assert_eq!(run("ISNUMBER([a])", &["1e3"]), "TRUE");
        assert_eq!(run("COALESCE([a], [b], \"-\")", &["", ""]), "-");
        assert_eq!(run("[a] <> [b]", &["2024-01-02", "2024-01-03"]), "TRUE");
    }

    #[test]
    fn text_functions() {
        assert_eq!(run("LEFT([a], 2) & RIGHT([a])", &["héllo"]), "héo");
        assert_eq!(run("MID([a], 2, 3)", &["abcdef"]), "bcd");
        assert_eq!(run("LEN([a])", &["日本語"]), "3");
        assert_eq!(run("PROPER([a])", &["o'neil mcDONALD"]), "O'neil Mcdonald");
        assert_eq!(run("TRIM([a])", &["  a   b  "]), "a b");
        assert_eq!(run("SUBSTITUTE([a], \"-\", \"\")", &["1-2-3"]), "123");
        assert_eq!(run("SUBSTITUTE([a], \"-\", \"+\", 2)", &["1-2-3"]), "1-2+3");
        assert_eq!(run("REPLACE([a], 2, 3, \"X\")", &["abcdef"]), "aXef");
        assert_eq!(run("FIND(\"c\", [a])", &["abcabc"]), "3");
        assert_eq!(run("FIND(\"c\", [a], 4)", &["abcabc"]), "6");
        assert_eq!(run("SEARCH(\"C\", [a])", &["abc"]), "3");
        assert_eq!(run("FIND(\"z\", [a])", &["abc"]), "#VALUE!");
        assert_eq!(
            run("TEXTJOIN(\", \", TRUE, [a], [b], [c])", &["x", "", "z"]),
            "x, z"
        );
        assert_eq!(run("REPT(\"ab\", 3)", &[]), "ababab");
        assert_eq!(run("REPT(\"ab\", 10000000)", &[]), "#VALUE!");
        assert_eq!(run("FIXED(1234567.891)", &[]), "1,234,567.89");
        assert_eq!(run("FIXED(-1234.5, 0, TRUE)", &[]), "-1235");
        assert_eq!(run("UPPER([a]) & LOWER([b])", &["a", "B"]), "Ab");
        assert_eq!(
            run("REGEXEXTRACT([a], \"[0-9]+\")", &["order 4521 shipped"]),
            "4521"
        );
        assert_eq!(
            run("REGEXEXTRACT([a], \"(\\w+)@(\\w+)\", 2)", &["me@host"]),
            "host"
        );
        assert_eq!(
            run("REGEXREPLACE([a], \"\\s+\", \" \")", &["a  \t b"]),
            "a b"
        );
        assert_eq!(run("REGEXMATCH([a], \"^[A-Z]{2}$\")", &["CA"]), "TRUE");
    }

    #[test]
    fn numbers_and_dates() {
        assert_eq!(run("ROUND(2.675, 2)", &[]), "2.68");
        assert_eq!(run("ROUND(-2.5, 0)", &[]), "-3");
        assert_eq!(run("ROUNDUP(1.21, 1)", &[]), "1.3");
        assert_eq!(run("ROUNDDOWN(-1.29, 1)", &[]), "-1.2");
        assert_eq!(run("ROUND(1234, -2)", &[]), "1200");
        assert_eq!(run("INT(-1.5)", &[]), "-2");
        assert_eq!(run("MOD(-7, 3)", &[]), "2");
        assert_eq!(run("SQRT(-1)", &[]), "#NUM!");
        assert_eq!(run("SUM([a], [b], [c])", &["1", "", "2"]), "3");
        assert_eq!(run("AVERAGE([a], [b])", &["", ""]), "#DIV/0!");
        assert_eq!(run("MAX([a], [b])", &["-3", "-7"]), "-3");
        assert_eq!(run("DATE(2024, 13, 1)", &[]), "2025-01-01");
        assert_eq!(run("DATE(2024, 3, 0)", &[]), "2024-02-29");
        assert_eq!(
            run("YEAR([a]) & \"/\" & MONTH([a])", &["2023-07-04"]),
            "2023/7"
        );
        assert_eq!(run("WEEKDAY(\"1970-01-01\")", &[]), "5");
        assert_eq!(run("[a] + 30", &["2024-01-15"]), "2024-02-14");
        assert_eq!(run("[a] - [b]", &["2024-03-01", "2024-02-01"]), "29");
        assert_eq!(run("DAYS([a], [b])", &["2024-03-01", "2024-02-01"]), "29");
        assert_eq!(
            run("HOUR([a]) * 60 + MINUTE([a])", &["2024-01-01 13:45"]),
            "825"
        );
        assert_eq!(run("TODAY()", &[]), "2022-01-08");
        assert_eq!(run("NOW()", &[]), "2022-01-08 12:00");
        assert_eq!(run("ROW()", &[]), "7");
        assert_eq!(run("VALUE(\"1,5\") ", &[]), "#VALUE!");
        assert_eq!(run("1e400 * 1", &[]), "#NUM!");
    }

    #[test]
    fn parse_errors_say_where() {
        let names = vec!["Price".to_string(), "Qty".to_string()];
        let msg = |s: &str| match Expr::parse(s, &names) {
            Err(Error::Invalid(m)) => m,
            other => panic!("{s}: {other:?}"),
        };
        assert!(msg("[Prise] * 2").contains("no column named [Prise]"));
        assert!(msg("Price * 2").contains("columns are written in brackets"));
        assert!(msg("FOO(1)").contains("unknown function FOO"));
        assert!(msg("LEFT()").contains("LEFT takes 1 to 2 values"));
        assert!(msg("(1 + 2").contains("missing its )"));
        assert!(msg("1 +").contains("ends too soon"));
        assert!(msg("\"abc").contains("closing quote"));
        assert!(msg("1 2").contains("character 3"));
        assert!(msg("REGEXMATCH([Qty], [Price])").contains("pattern written as text"));
        assert!(msg("IFS(1, 2, 3)").contains("pairs"));
        // Column names ignore case; [#n] picks by number.
        assert!(Expr::parse("[price] * [#2]", &names).is_ok());
        assert!(msg("[#3]").contains("no column #3"));
        // Too deep or too long is refused, never a stack overflow.
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(Expr::parse(&deep, &names).is_err());
        let neg = "-".repeat(5_000) + "1";
        assert!(Expr::parse(&neg, &names).is_err());
        let long = vec!["1"; 3_000].join("+");
        assert!(Expr::parse(&long, &names).is_err());
        assert!(Expr::parse(&"a".repeat(MAX_EXPR_CHARS + 1), &names).is_err());
        assert!(Expr::parse("REGEXMATCH([Qty], \"(\")", &names).is_err());
    }

    #[test]
    fn longest_chains_evaluate_without_running_out_of_stack() {
        // The node cap bounds how deep a left-leaning chain of operators
        // gets; evaluating the deepest allowed chain must fit a worker
        // thread's stack (2 MB; debug builds use bigger frames).
        let names = vec!["a".to_string()];
        let terms = MAX_NODES / 2;
        for op in ["+", "&", "*", "<"] {
            let src = vec!["[a]"; terms].join(op);
            let e = Expr::parse(&src, &names).unwrap();
            let h = std::thread::Builder::new()
                .stack_size(2 << 20)
                .spawn(move || {
                    let cells = vec![Cow::Borrowed("1")];
                    let ctx = Ctx {
                        cells: &cells,
                        slots: &[Some(0)],
                        row: 1,
                        now: 0.0,
                        read: ReadOptions::default(),
                    };
                    e.text(&ctx)
                })
                .unwrap();
            assert!(h.join().is_ok(), "{op}");
        }
    }

    #[test]
    fn big_results_are_refused_before_they_are_built() {
        // 30,000 commas, each replaced by 30,000 letters, would be 900 MB.
        let commas = ",".repeat(30_000);
        let letters = "x".repeat(30_000);
        let cells = [commas.as_str(), letters.as_str()];
        assert_eq!(run("SUBSTITUTE([a], \",\", [b])", &cells), "#VALUE!");
        assert_eq!(run("REGEXREPLACE([a], \",\", [b])", &cells), "#VALUE!");
        assert_eq!(run("LEN([a] & [b])", &cells), "60000");
        // Under the limit they still work.
        assert_eq!(run("SUBSTITUTE([a], \",\", \"\")", &cells), "");
        assert_eq!(
            run("LEN(REGEXREPLACE([a], \",\", \"ab\"))", &cells),
            "60000"
        );
        assert_eq!(
            run("REGEXREPLACE([a], \"(b)\", \"[$1]\")", &["abc"]),
            "a[b]c"
        );
        let half = "y".repeat(MAX_TEXT / 2 + 1);
        assert_eq!(run("LEN([a] & [a])", &[half.as_str()]), "#VALUE!");
    }

    #[test]
    fn group_references_cannot_grow_a_match_past_the_limit() {
        // One match of 50,000 letters pasted back 6,000 times would be 300 MB.
        let a = "a".repeat(50_000);
        let b = "$0".repeat(6_000);
        assert_eq!(
            run("REGEXREPLACE([a], \"a+\", [b])", &[a.as_str(), b.as_str()]),
            "#VALUE!"
        );
        let b = "${0}".repeat(30);
        assert_eq!(
            run("REGEXREPLACE([a], \"a+\", [b])", &[a.as_str(), b.as_str()]),
            "#VALUE!"
        );
        // References read as the regex crate reads them.
        let r = |rep: &str| {
            run(
                &format!("REGEXREPLACE([a], \"(?P<x>b)(c)\", \"{rep}\")"),
                &["abcd"],
            )
        };
        assert_eq!(r("[$1|$2|${x}|$$|$9|$x|$]"), "a[b|c|b|$||b|$]d");
        assert_eq!(r("$1x"), "ad");
        assert_eq!(r("${1}x"), "abxd");
        assert_eq!(r("${"), "a${d");
        // Too many references are refused at once, however short the cell.
        let many = "$0".repeat(MAX_REFS + 1);
        assert_eq!(
            run("REGEXREPLACE([a], \"\", [b])", &["ab", many.as_str()]),
            "#VALUE!"
        );
        let ok = "$0".repeat(MAX_REFS);
        assert_eq!(
            run("LEN(REGEXREPLACE([a], \"b\", [b]))", &["ab", ok.as_str()]),
            "1001"
        );
    }

    #[test]
    fn hostile_input_never_panics() {
        let names = vec!["a".to_string()];
        let srcs = [
            "MID([a], 1e300, 1e300)",
            "LEFT([a], -1)",
            "RIGHT([a], 1e20)",
            "REPLACE([a], 99, 99, \"x\")",
            "FIND(\"\", [a], 999)",
            "DATE(1e9, 1, 1)",
            "YEAR(1e300)",
            "ROUND(1, 1e9)",
            "SUBSTITUTE([a], \"\", \"x\")",
            "REGEXEXTRACT([a], \"a\", 99)",
            "FIXED(1e300, 15)",
            "WEEKDAY(-1e300)",
            "[a] & [a] & [a]",
        ];
        for cell in ["", "é", "日本語", "-0", "1e308", "NaN", "inf", "\u{0}"] {
            for s in srcs {
                let e = Expr::parse(s, &names).unwrap();
                let cells = vec![Cow::Borrowed(cell)];
                let ctx = Ctx {
                    cells: &cells,
                    slots: &[Some(0)],
                    row: 1,
                    now: 0.0,
                    read: ReadOptions::default(),
                };
                let _ = e.text(&ctx);
            }
        }
    }

    #[test]
    fn computed_column_adds_and_writes_over() {
        let mut d = Document::from_bytes(
            b"Price,Qty\r\n2,3\r\n1.5,4\r\nx,1\r\n".to_vec(),
            OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let spec = ComputeSpec {
            expr: "[Price] * [Qty]".into(),
            title: "Total".into(),
            ..Default::default()
        };
        // Columns used in a different order than the file's.
        let rev = ComputeSpec {
            expr: "[Qty] & \"x\" & [Price]".into(),
            ..Default::default()
        };
        assert_eq!(
            d.preview_compute(&rev, 1).unwrap().rows,
            vec![(2, "3x2".into())]
        );
        let pv = d.preview_compute(&spec, 10).unwrap();
        assert_eq!(
            pv.rows,
            vec![(2, "6".into()), (3, "6".into()), (4, "#VALUE!".into())]
        );
        assert_eq!(pv.errors, 1);
        let (plan, rep) = d.plan_compute(&spec, &Progress::new()).unwrap();
        assert_eq!(rep.errors, 1);
        d.apply_cols(plan).unwrap();
        assert_eq!(d.row_values(0).unwrap(), ["Price", "Qty", "Total"]);
        assert_eq!(d.row_values(2).unwrap(), ["1.5", "4", "6"]);
        // Write over the Qty column in place.
        let spec = ComputeSpec {
            expr: "[Qty] * 10".into(),
            target: Some(1),
            ..Default::default()
        };
        let (plan, _) = d.plan_compute(&spec, &Progress::new()).unwrap();
        d.apply_cols(plan).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["2", "30", "6"]);
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(d.row_values(1).unwrap(), ["2", "3"]);
        assert_eq!(d.col_count(), 2);
    }

    #[test]
    fn lookup_reads_another_table() {
        use crate::join::LookupTable;
        let other = Document::from_bytes(
            b"code,name\r\n00123,Ann\r\nb7,Bob\r\n".to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let mut d = Document::from_bytes(
            b"id\r\n123\r\nB7\r\nzz\r\n".to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let src = "LOOKUP([id], \"people\", \"code\", \"name\", \"?\")";
        let e = d.parse_expr(src, true).unwrap();
        assert_eq!(
            e.lookups(),
            &[LookupRef {
                tab: "people".into(),
                key: "code".into(),
                column: "name".into()
            }]
        );
        // Not bound: the app found no such tab.
        let mut spec = ComputeSpec {
            expr: src.into(),
            title: "name".into(),
            ..Default::default()
        };
        let p = d.preview_compute(&spec, 5).unwrap();
        assert_eq!(p.rows[0].1, "#REF!");
        let t = LookupTable::build(&other, 0, 1, true, &Progress::new()).unwrap();
        spec.tables = Lookups(vec![Some(Arc::new(t))]);
        let p = d.preview_compute(&spec, 5).unwrap();
        let got: Vec<&str> = p.rows.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(got, ["Ann", "Bob", "?"]);
        let (plan, rep) = d.plan_compute(&spec, &Progress::new()).unwrap();
        assert_eq!(rep.errors, 0);
        d.apply_cols(plan).unwrap();
        assert_eq!(d.row_values(1).unwrap(), ["123", "Ann"]);
        // Without a fallback, a missing key is #N/A; names must be text.
        spec.expr = "LOOKUP([id], \"people\", \"code\", \"name\")".into();
        spec.target = Some(1);
        let p = d.preview_compute(&spec, 5).unwrap();
        assert_eq!(p.rows[2].1, "#N/A");
        assert!(d
            .parse_expr("LOOKUP([id], [id], \"a\", \"b\")", true)
            .is_err());
        let many = ["LOOKUP([id], \"t\", \"a\", \"b\")"; MAX_LOOKUPS + 1].join(" & ");
        assert!(d.parse_expr(&many, true).is_err());
    }
}
