//! ML-surface spike for Cadenza. Prints the real `cdz_compiler::ast::Node` to an ML-flavored
//! text surface and reads it back, validating a lossless round-trip against the s-expr corpus.
//!
//! Architecture:
//!   * A GENERIC BIJECTION underpins everything: any `(Name a b c)` list round-trips as the call
//!     form `Name(a, b, c)` and back. This covers `def module list tuple match effect op host
//!     handle quote quasiquote unquote record map do type` and every user function for free.
//!   * Ergonomic surfaces are LAYERED on top only where they clearly win and still round-trip:
//!     infix operators (Pratt precedence), member access `a.b`, `let … in …`, `if … then … else`,
//!     and `fn(x) => e`. These are the constructs an agent reads/writes most.
//!
//! The printer emits MINIMAL parentheses using the precedence table; the reader is a Pratt parser
//! that reconstructs the identical Ast. `read_ml(print_ml(read_sexpr(x))) == read_sexpr(x)`.

use cdz_compiler::ast::{self, Node};

mod corpus_test;

// ===================================================================================
// Precedence table (higher binds tighter). Matches the coordinator's spec:
//   or < and < comparisons < (| ^) < & < (<< >>) < (+ -) < (* / %) < member/app
// All left-associative.
// ===================================================================================
fn infix_prec(op: &str) -> Option<u8> {
    Some(match op {
        "or" => 1,
        "and" => 2,
        "=" | "<" | ">" | "<=" | ">=" => 3,
        "|" | "^" => 4,
        "&" => 5,
        "<<" | ">>" => 6,
        "+" | "-" | "+%" | "-%" => 7,
        "*" | "/" | "%" | "*%" => 8,
        _ => return None,
    })
}

const PREC_MEMBER: u8 = 10; // `.` and application bind tightest

// ===================================================================================
// PRINTER: Node -> ML text. `parent_prec` is the precedence of the surrounding context;
// a subexpression parenthesizes itself when its own precedence is lower than the context.
// ===================================================================================

fn print_ml(node: &Node) -> String {
    print_prec(node, 0)
}

fn print_prec(node: &Node, parent_prec: u8) -> String {
    match node {
        Node::Int(n) => n.to_string(),
        Node::Float(f) => fmt_float(*f),
        Node::Str(s) => fmt_string(s),
        Node::Bool(b) => b.to_string(),
        Node::Name(n) => emit_name(n),
        Node::List(items) if items.is_empty() => "#[]".to_string(),
        Node::List(items) => print_list(items, parent_prec),
    }
}

/// A Name prints bare when it re-lexes to exactly that Name; otherwise it is backtick-quoted.
/// This is the lossless escape for operator-symbol names (`|`, `+`, `:`, `->`), leading-dot names
/// (`.of`), reserved keywords used as ordinary names, etc. — the key to representing symbolic heads
/// and atoms in a surface where those glyphs otherwise lex as operators/keywords.
fn emit_name(s: &str) -> String {
    if name_is_bare_safe(s) {
        s.to_string()
    } else {
        let mut out = String::from("`");
        for c in s.chars() {
            if c == '`' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('`');
        out
    }
}

fn name_is_bare_safe(s: &str) -> bool {
    match Lexer::new(s).tokenize() {
        Ok(toks) => toks.len() == 2 && toks[0] == Tok::Name(s.to_string()) && toks[1] == Tok::Eof,
        Err(_) => false,
    }
}

fn fmt_float(f: f64) -> String {
    if f.is_nan() {
        "nan".to_string()
    } else if f == f.trunc() && f.is_finite() {
        // print 1.0 not 1, so the reader re-reads a float
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

fn fmt_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_list(items: &[Node], parent_prec: u8) -> String {
    // Head must be a Name to be a recognized form or an application.
    if let Node::Name(head) = &items[0] {
        let args = &items[1..];

        // ---- Infix binary operators ----
        if let Some(prec) = infix_prec(head) {
            if args.len() == 2 {
                // Left-assoc: left child may share precedence; right child must be higher.
                let l = print_prec(&args[0], prec);
                let r = print_prec(&args[1], prec + 1);
                let s = format!("{} {} {}", l, head, r);
                return maybe_paren(s, prec, parent_prec);
            }
        }

        // ---- Member access `(. a b)` -> `a.b` ----
        if head == "." && args.len() == 2 {
            // Only when the key is an alpha/underscore Name (a field/qualified name); a dotted or
            // numeric key falls through to the generic call form so it round-trips. The key goes
            // through emit_name so a key that re-lexes as a reserved keyword (`.in`, `.then`) is
            // backtick-quoted and read back as a member name, not a keyword.
            if let Node::Name(key) = &args[1] {
                if is_plain_ident(key) {
                    let obj = print_prec(&args[0], PREC_MEMBER);
                    return format!("{}.{}", obj, emit_name(key));
                }
            }
        }

        // ---- quasiquote / unquote / unquote-splicing sigils ----
        // Mirror the s-expr reader's sigils INTO ML: `(quasiquote X)` -> `` `{ X } ``,
        // `(unquote X)` -> `,X` / `,{X}`, `(unquote-splicing X)` -> `,@X` / `,@{X}`. The interior
        // renders in the ordinary ML surface, so a quoted fragment reads like the code it builds.
        // (`quote` stays a call form — s-expr writes plain quote as the word `quote` too, no sigil.)
        if head == "quasiquote" && args.len() == 1 {
            // `` `{…} `` is self-delimited, so it never needs outer parens.
            return format!("`{{ {} }}", print_prec(&args[0], 0));
        }
        if head == "unquote" && args.len() == 1 {
            return paren_if_head(print_unquote(",", &args[0]), parent_prec);
        }
        if head == "unquote-splicing" && args.len() == 1 {
            return paren_if_head(print_unquote(",@", &args[0]), parent_prec);
        }

        // ---- let / if / fn / match keyworded forms ----
        match head.as_str() {
            "let" if is_let_shape(args) => return print_let(args, parent_prec),
            "if" if args.len() == 3 => return print_if(args, parent_prec),
            "fn" if args.len() == 2 => return print_fn(args, parent_prec),
            "match" if is_match_shape(args) => return print_match(args, parent_prec),
            _ => {}
        }

        // ---- Generic application / call form: head(a, b, c) ----
        // This is the lossless bijection for every other Name-headed list. The head goes through
        // emit_name so symbolic/keyword heads (`:`, `->`, `type` is fine, etc.) round-trip.
        let arglist = args.iter().map(|a| print_prec(a, 0)).collect::<Vec<_>>().join(", ");
        return format!("{}({})", emit_name(head), arglist);
    }

    // Head is NOT a name: this is application of a computed function, e.g. `((. List at) xs 0)`.
    // Print as chained application `head(args)`; reading `f(a)(b)` reconstructs the same left-
    // nested list. A match arm `((pat…) body)` is structurally the same as such an application, so
    // this rule round-trips arms with compound patterns too.
    let head = print_prec(&items[0], PREC_MEMBER);
    let arglist = items[1..].iter().map(|a| print_prec(a, 0)).collect::<Vec<_>>().join(", ");
    format!("{}({})", head, arglist)
}

fn maybe_paren(s: String, prec: u8, parent_prec: u8) -> String {
    if prec < parent_prec {
        format!("({})", s)
    } else {
        s
    }
}

/// A `,`/`,@` sigil parses its body greedily to the right; when the sigil node is itself an
/// application/member head (parent_prec == PREC_MEMBER) the bare form would swallow the following
/// args, so parenthesize it there.
fn paren_if_head(s: String, parent_prec: u8) -> String {
    if parent_prec >= PREC_MEMBER {
        format!("({})", s)
    } else {
        s
    }
}

/// A plain identifier: starts alpha/underscore, no dots (so a dotted key stays generic).
fn is_plain_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    !s.contains('.') && !s.is_empty()
}

/// `(let (bindings...) body)` where bindings is a list of `(name init)` pairs.
fn is_let_shape(args: &[Node]) -> bool {
    if args.len() != 2 {
        return false;
    }
    if let Node::List(binds) = &args[0] {
        return binds.iter().all(|b| matches!(b, Node::List(p) if p.len() == 2 && matches!(p[0], Node::Name(_))));
    }
    false
}

fn print_let(args: &[Node], parent_prec: u8) -> String {
    let mut parts = Vec::new();
    if let Node::List(binds) = &args[0] {
        for b in binds {
            if let Node::List(p) = b {
                parts.push(format!("{} = {}", print_ml(&p[0]), print_prec(&p[1], 0)));
            }
        }
    }
    let s = format!("let {} in {}", parts.join(", "), print_prec(&args[1], 0));
    // let has the lowest precedence; parenthesize if inside a tighter context.
    if parent_prec > 0 {
        format!("({})", s)
    } else {
        s
    }
}

fn print_if(args: &[Node], parent_prec: u8) -> String {
    let s = format!(
        "if {} then {} else {}",
        print_prec(&args[0], 0),
        print_prec(&args[1], 0),
        print_prec(&args[2], 0)
    );
    if parent_prec > 0 {
        format!("({})", s)
    } else {
        s
    }
}

fn print_fn(args: &[Node], parent_prec: u8) -> String {
    let params = if let Node::List(ps) = &args[0] {
        ps.iter().map(print_ml).collect::<Vec<_>>().join(", ")
    } else {
        String::new()
    };
    let s = format!("fn({}) => {}", params, print_prec(&args[1], 0));
    if parent_prec > 0 {
        format!("({})", s)
    } else {
        s
    }
}

/// An unquote body: `,x` if the interior is atom-like, `,{expr}` if it needs bracing to keep the
/// sigil attached to exactly the right subtree. Bare form is used only for a single atom or a
/// member-access chain, so re-reading `,x` / `,foo.bar` binds the whole interior.
fn print_unquote(sigil: &str, inner: &Node) -> String {
    if unquote_body_is_atomic(inner) {
        format!("{}{}", sigil, print_prec(inner, PREC_MEMBER))
    } else {
        format!("{}{{ {} }}", sigil, print_prec(inner, 0))
    }
}

/// True for interiors that re-read correctly WITHOUT braces after a `,` sigil: a literal, a name,
/// or a member-access chain (`a.b.c`). Anything with looser structure (arithmetic, application)
/// is braced so the sigil captures exactly it.
fn unquote_body_is_atomic(n: &Node) -> bool {
    match n {
        Node::Int(_) | Node::Float(_) | Node::Str(_) | Node::Bool(_) | Node::Name(_) => true,
        Node::List(items) => {
            // a pure member-access chain `(. a b)` with a plain-ident key
            matches!(items.first(), Some(Node::Name(h)) if h == ".")
                && items.len() == 3
                && matches!(&items[2], Node::Name(k) if is_plain_ident(k))
                && unquote_body_is_atomic(&items[1])
        }
    }
}

/// `(match scrutinee (pat body) (pat body) …)` — every arm is a 2-element list.
fn is_match_shape(args: &[Node]) -> bool {
    if args.is_empty() {
        return false;
    }
    args[1..]
        .iter()
        .all(|a| matches!(a, Node::List(p) if p.len() == 2))
}

/// Print `match` as a keyword block. Each arm is `<pattern-or-guard> => <body>`. The pattern side
/// is an ARBITRARY expression (a guard like `n < 0`, a constructor pattern like `Some(n)`, a
/// wildcard `_`, `else`, or a quote-pattern `` `{…} ``); it prints with the ordinary expression
/// printer, so guard arms and constructor/quote-pattern arms use the identical path and both
/// round-trip. There is no separate "pattern grammar" — that is the point of the spike.
fn print_match(args: &[Node], parent_prec: u8) -> String {
    let scrut = print_prec(&args[0], 0);
    let mut arms = Vec::new();
    for arm in &args[1..] {
        if let Node::List(p) = arm {
            arms.push(format!("  {} => {}", print_prec(&p[0], 0), print_prec(&p[1], 0)));
        }
    }
    let s = format!("match {} {{\n{}\n}}", scrut, arms.join(",\n"));
    if parent_prec > 0 {
        format!("({})", s)
    } else {
        s
    }
}

// ===================================================================================
// LEXER
// ===================================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Name(String),          // identifiers, kebab-case, dotted positional (tuple.0)
    Op(String),            // infix operator symbol
    Let, In, If, Then, Fn, Match,
    LParen, RParen,
    LBrace, RBrace,        // match block delimiters
    LBracket, RBracket,    // #[ ... ] escape uses these after '#'
    Hash,                  // '#'
    Backtick,              // ` quasiquote prefix
    UnquoteSplice,         // ,@ unquote-splicing prefix
    // `Comma` doubles as the unquote prefix `,` — the Pratt parser reads it as unquote in prefix
    // position and as a separator in argument/binding/arm loops; the two positions never overlap.
    Comma, Dot, FatArrow, Arrow, Colon,
    Eof,
}

struct Lexer {
    src: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(s: &str) -> Self {
        Lexer { src: s.chars().collect(), pos: 0 }
    }
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.src.get(self.pos + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else if c == '/' && self.peek2() == Some('/') {
                while let Some(c) = self.bump() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Tok>, String> {
        let mut toks = Vec::new();
        loop {
            self.skip_ws();
            let c = match self.peek() {
                None => {
                    toks.push(Tok::Eof);
                    break;
                }
                Some(c) => c,
            };
            let t = match c {
                '(' => { self.bump(); Tok::LParen }
                ')' => { self.bump(); Tok::RParen }
                '{' => { self.bump(); Tok::LBrace }
                '}' => { self.bump(); Tok::RBrace }
                '[' => { self.bump(); Tok::LBracket }
                ']' => { self.bump(); Tok::RBracket }
                '#' => { self.bump(); Tok::Hash }
                ',' => {
                    self.bump();
                    if self.peek() == Some('@') {
                        self.bump();
                        Tok::UnquoteSplice
                    } else {
                        Tok::Comma
                    }
                }
                // `` `{ … } `` is a quasiquote block; `` `name` `` is a symbolic-name escape.
                '`' if self.peek2() == Some('{') => { self.bump(); Tok::Backtick }
                '`' => Tok::Name(self.read_backtick_name()?),
                '"' => Tok::Str(self.read_string()?),
                '+' | '*' | '%' | '&' | '^' => {
                    self.bump();
                    // check for wrapping suffix `+%`
                    if self.peek() == Some('%') && (c == '+' || c == '-' || c == '*') {
                        self.bump();
                        Tok::Op(format!("{}%", c))
                    } else {
                        Tok::Op(c.to_string())
                    }
                }
                '|' => { self.bump(); Tok::Op("|".into()) }
                '/' => { self.bump(); Tok::Op("/".into()) }
                '=' => {
                    self.bump();
                    if self.peek() == Some('>') {
                        self.bump();
                        Tok::FatArrow
                    } else {
                        Tok::Op("=".into())
                    }
                }
                '<' => {
                    self.bump();
                    match self.peek() {
                        Some('=') => { self.bump(); Tok::Op("<=".into()) }
                        Some('<') => { self.bump(); Tok::Op("<<".into()) }
                        _ => Tok::Op("<".into()),
                    }
                }
                '>' => {
                    self.bump();
                    match self.peek() {
                        Some('=') => { self.bump(); Tok::Op(">=".into()) }
                        Some('>') => { self.bump(); Tok::Op(">>".into()) }
                        _ => Tok::Op(">".into()),
                    }
                }
                '-' => {
                    // KEBAB RULE (stated, not heuristic): a `-` is an IDENTIFIER character iff it
                    // has a word char on BOTH sides (`byte-at`); otherwise it is an operator.
                    // `read_number_or_word` implements the "both sides" half — once a word char has
                    // been read, it absorbs a following `-` only when the char after it is also a
                    // word char. So a hyphen inside a word never reaches this arm. Here (a `-` with
                    // a non-word char to its left) we resolve the token: `->` arrow, `-%` wrapping
                    // sub, `-<digit>` negative literal, bare `-` subtraction. Net: `a - b` subtracts,
                    // `a-b`/`byte-at` is one identifier. COST: a human cannot write `x-1` for
                    // subtraction — it must be `x - 1`, else `x-1` reads as the identifier "x-1".
                    if self.peek2() == Some('>') {
                        self.bump();
                        self.bump();
                        Tok::Arrow
                    } else if self.peek2().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        self.read_number_or_word()?
                    } else {
                        self.bump();
                        if self.peek() == Some('%') {
                            self.bump();
                            Tok::Op("-%".into())
                        } else {
                            Tok::Op("-".into())
                        }
                    }
                }
                '.' => { self.bump(); Tok::Dot }
                ':' => { self.bump(); Tok::Colon }
                c if c.is_ascii_digit() || c.is_alphabetic() || c == '_' => {
                    self.read_number_or_word()?
                }
                other => return Err(format!("unexpected char {:?}", other)),
            };
            toks.push(t);
        }
        Ok(toks)
    }

    /// A backtick-quoted name `\`...\`` — the lossless escape for symbolic/keyword atoms.
    fn read_backtick_name(&mut self) -> Result<String, String> {
        self.bump(); // opening `
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated backtick name".into()),
                Some('`') => break,
                Some('\\') => match self.bump() {
                    Some(c) => s.push(c),
                    None => return Err("unterminated backtick escape".into()),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn read_string(&mut self) -> Result<String, String> {
        self.bump(); // opening "
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".into()),
                Some('"') => break,
                Some('\\') => match self.bump() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(o) => s.push(o),
                    None => return Err("unterminated escape".into()),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    /// Read a number or an identifier word. Kebab hyphens and positional `.N` suffixes are
    /// absorbed into the word so `byte-at` and `tuple.0` come out as single Name tokens, matching
    /// the s-expr reader's token model.
    fn read_number_or_word(&mut self) -> Result<Tok, String> {
        let start = self.pos;
        let first = self.peek().unwrap();
        let negative = first == '-';
        if negative {
            self.bump();
        }
        // Consume the run.
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.bump();
            } else if c == '-' && self.peek2().map(|n| n.is_alphanumeric() || n == '_').unwrap_or(false) {
                // kebab: hyphen glued between word chars
                self.bump();
            } else if c == '.' && self.peek2().map(|n| n.is_ascii_digit()).unwrap_or(false) {
                // positional accessor `.N` or a float fractional part; absorb the dot+digits.
                // classify_token below decides float-vs-name.
                self.bump();
            } else {
                break;
            }
        }
        let text: String = self.src[start..self.pos].iter().collect();
        Ok(classify_word(&text))
    }
}

/// Classify a lexed word into Int / Float / Bool / Name, mirroring the s-expr reader's
/// `classify_token` for the shapes the corpus uses.
fn classify_word(tok: &str) -> Tok {
    match tok {
        "true" => return Tok::Bool(true),
        "false" => return Tok::Bool(false),
        "nan" => return Tok::Float(f64::NAN),
        "let" => return Tok::Let,
        "in" => return Tok::In,
        "if" => return Tok::If,
        "then" => return Tok::Then,
        "fn" => return Tok::Fn,
        "match" => return Tok::Match,
        // `and`/`or` are word-spelled infix operators.
        "and" | "or" => return Tok::Op(tok.to_string()),
        // NOTE: `else` is deliberately NOT reserved — it is a match catch-all pattern head. In the
        // match keyword form it appears bare on the arm's left of `=>`; `parse_match` recognizes it.
        _ => {}
    }
    if looks_like_int(tok) {
        if let Some(i) = parse_int(tok) {
            return Tok::Int(i);
        }
    }
    if looks_like_float(tok) {
        let cleaned: String = tok.chars().filter(|&c| c != '_').collect();
        if let Ok(f) = cleaned.parse::<f64>() {
            return Tok::Float(f);
        }
    }
    Tok::Name(tok.to_string())
}

fn looks_like_int(tok: &str) -> bool {
    let body = tok.strip_prefix('-').unwrap_or(tok);
    if body.is_empty() {
        return false;
    }
    if let Some(rest) = body.strip_prefix("0x") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit() || c == '_');
    }
    if let Some(rest) = body.strip_prefix("0b") {
        return !rest.is_empty() && rest.chars().all(|c| c == '0' || c == '1' || c == '_');
    }
    body.chars().next().map_or(false, |c| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_digit() || c == '_')
}

fn parse_int(tok: &str) -> Option<i64> {
    // Keep the sign in the string so i64::MIN (`-9223372036854775808`, whose magnitude overflows
    // i64) parses correctly — negating a parsed magnitude would overflow.
    let neg = tok.starts_with('-');
    let body = tok.strip_prefix('-').unwrap_or(tok);
    let cleaned_body: String = body.chars().filter(|&c| c != '_').collect();
    let sign = if neg { "-" } else { "" };
    if let Some(h) = cleaned_body.strip_prefix("0x") {
        return i64::from_str_radix(&format!("{}{}", sign, h), 16).ok();
    }
    if let Some(b) = cleaned_body.strip_prefix("0b") {
        return i64::from_str_radix(&format!("{}{}", sign, b), 2).ok();
    }
    format!("{}{}", sign, cleaned_body).parse::<i64>().ok()
}

fn looks_like_float(tok: &str) -> bool {
    let body = tok.strip_prefix('-').unwrap_or(tok);
    if !body.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return false;
    }
    (body.contains('.') || body.contains('e') || body.contains('E'))
        && body.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
}

// ===================================================================================
// PARSER (Pratt)
// ===================================================================================

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(toks: Vec<Tok>) -> Self {
        Parser { toks, pos: 0 }
    }
    fn peek(&self) -> &Tok {
        self.toks.get(self.pos).unwrap_or(&Tok::Eof)
    }
    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn expect(&mut self, t: Tok) -> Result<(), String> {
        let got = self.bump();
        if got == t {
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", t, got))
        }
    }

    /// Pratt loop: parse a prefix, then fold in infix operators of precedence >= min_prec.
    fn parse_expr(&mut self, min_prec: u8) -> Result<Node, String> {
        let mut left = self.parse_prefix()?;
        loop {
            let op = match self.peek() {
                Tok::Op(o) => o.clone(),
                _ => break,
            };
            let prec = match infix_prec(&op) {
                Some(p) => p,
                None => break,
            };
            if prec < min_prec {
                break;
            }
            self.bump(); // operator
            // left-assoc: right side parses at prec+1
            let right = self.parse_expr(prec + 1)?;
            left = Node::List(vec![Node::Name(op), left, right]);
        }
        Ok(left)
    }

    /// Prefix: literals, names (with member access + application), keyword forms, grouping.
    fn parse_prefix(&mut self) -> Result<Node, String> {
        let node = match self.peek().clone() {
            Tok::Int(n) => { self.bump(); Node::Int(n) }
            Tok::Float(f) => { self.bump(); Node::Float(f) }
            Tok::Str(s) => { self.bump(); Node::Str(s) }
            Tok::Bool(b) => { self.bump(); Node::Bool(b) }
            Tok::Name(n) => { self.bump(); Node::Name(n) }
            Tok::Let => return self.parse_let(),
            Tok::If => return self.parse_if(),
            Tok::Fn => return self.parse_fn(),
            Tok::Match => return self.parse_match(),
            Tok::Backtick => {
                // `` `{ X } `` -> (quasiquote X)
                self.bump();
                self.expect(Tok::LBrace)?;
                let inner = self.parse_expr(0)?;
                self.expect(Tok::RBrace)?;
                return Ok(Node::List(vec![Node::Name("quasiquote".into()), inner]));
            }
            Tok::Comma => {
                // `,X` or `,{X}` -> (unquote X)
                self.bump();
                let inner = self.parse_sigil_body()?;
                return Ok(Node::List(vec![Node::Name("unquote".into()), inner]));
            }
            Tok::UnquoteSplice => {
                // `,@X` or `,@{X}` -> (unquote-splicing X)
                self.bump();
                let inner = self.parse_sigil_body()?;
                return Ok(Node::List(vec![Node::Name("unquote-splicing".into()), inner]));
            }
            Tok::LParen => {
                self.bump();
                if self.peek() == &Tok::RParen {
                    self.bump();
                    Node::List(vec![]) // unit-ish empty; printer emits `unit`, rarely hit
                } else {
                    let e = self.parse_expr(0)?;
                    self.expect(Tok::RParen)?;
                    e
                }
            }
            Tok::Hash => {
                // `#[ a b c ]` escape -> plain list
                self.bump();
                self.expect(Tok::LBracket)?;
                let mut items = Vec::new();
                while self.peek() != &Tok::RBracket {
                    items.push(self.parse_expr(0)?);
                }
                self.expect(Tok::RBracket)?;
                return Ok(Node::List(items));
            }
            other => return Err(format!("unexpected token {:?}", other)),
        };
        // postfix: member access `.name` and application `(...)`
        self.parse_postfix(node)
    }

    fn parse_postfix(&mut self, mut node: Node) -> Result<Node, String> {
        loop {
            match self.peek() {
                Tok::Dot => {
                    self.bump();
                    let key = match self.bump() {
                        Tok::Name(n) => n,
                        Tok::Int(i) => i.to_string(), // shouldn't happen (positional glued in lexer)
                        other => return Err(format!("expected member name after '.', got {:?}", other)),
                    };
                    node = Node::List(vec![Node::Name(".".into()), node, Node::Name(key)]);
                }
                Tok::LParen => {
                    // application: node(args...) -> (node args...)
                    self.bump();
                    let mut items = vec![node];
                    if self.peek() != &Tok::RParen {
                        loop {
                            items.push(self.parse_expr(0)?);
                            if self.peek() == &Tok::Comma {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen)?;
                    node = Node::List(items);
                }
                _ => break,
            }
        }
        Ok(node)
    }

    fn parse_let(&mut self) -> Result<Node, String> {
        self.expect(Tok::Let)?;
        let mut binds = Vec::new();
        loop {
            let name = match self.bump() {
                Tok::Name(n) => n,
                other => return Err(format!("expected binding name, got {:?}", other)),
            };
            self.expect(Tok::Op("=".into()))?;
            let init = self.parse_expr(0)?;
            binds.push(Node::List(vec![Node::Name(name), init]));
            if self.peek() == &Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(Tok::In)?;
        let body = self.parse_expr(0)?;
        Ok(Node::List(vec![Node::Name("let".into()), Node::List(binds), body]))
    }

    fn parse_if(&mut self) -> Result<Node, String> {
        self.expect(Tok::If)?;
        let c = self.parse_expr(0)?;
        self.expect(Tok::Then)?;
        let t = self.parse_expr(0)?;
        // `else` is an ordinary identifier token (kept unreserved for match catch-all).
        match self.bump() {
            Tok::Name(ref n) if n == "else" => {}
            other => return Err(format!("expected 'else', got {:?}", other)),
        }
        let e = self.parse_expr(0)?;
        Ok(Node::List(vec![Node::Name("if".into()), c, t, e]))
    }

    fn parse_fn(&mut self) -> Result<Node, String> {
        self.expect(Tok::Fn)?;
        self.expect(Tok::LParen)?;
        let mut params = Vec::new();
        if self.peek() != &Tok::RParen {
            loop {
                match self.bump() {
                    Tok::Name(n) => params.push(Node::Name(n)),
                    other => return Err(format!("expected param, got {:?}", other)),
                }
                if self.peek() == &Tok::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(Tok::RParen)?;
        self.expect(Tok::FatArrow)?;
        let body = self.parse_expr(0)?;
        Ok(Node::List(vec![Node::Name("fn".into()), Node::List(params), body]))
    }

    /// `match SCRUT { PAT => BODY, PAT => BODY, … }`. The PAT side is parsed as an ordinary
    /// expression (guard, constructor pattern, wildcard, `else`, or quote-pattern), so every arm
    /// kind reconstructs the identical `(pat body)` arm the s-expr reader produces.
    fn parse_match(&mut self) -> Result<Node, String> {
        self.expect(Tok::Match)?;
        let scrut = self.parse_expr(0)?;
        self.expect(Tok::LBrace)?;
        let mut items = vec![Node::Name("match".into()), scrut];
        while self.peek() != &Tok::RBrace {
            let pat = self.parse_expr(0)?;
            self.expect(Tok::FatArrow)?;
            let body = self.parse_expr(0)?;
            items.push(Node::List(vec![pat, body]));
            if self.peek() == &Tok::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(Tok::RBrace)?;
        Ok(Node::List(items))
    }

    /// The body of a `,` / `,@` sigil: a braced `{ expr }` or a single tight prefix (a name,
    /// literal, or member-access chain — the shapes `print_unquote` emits bare).
    fn parse_sigil_body(&mut self) -> Result<Node, String> {
        if self.peek() == &Tok::LBrace {
            self.bump();
            let e = self.parse_expr(0)?;
            self.expect(Tok::RBrace)?;
            Ok(e)
        } else {
            // Parse a tight prefix and its postfix member/app chain, but not a following infix op.
            self.parse_prefix()
        }
    }
}

fn read_ml(input: &str) -> Result<Node, String> {
    let toks = Lexer::new(input).tokenize()?;
    let mut p = Parser::new(toks);
    let node = p.parse_expr(0)?;
    if p.peek() != &Tok::Eof {
        return Err(format!("trailing tokens from {:?}", p.peek()));
    }
    Ok(node)
}

// ===================================================================================
// MAIN
// ===================================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let corpus_dir =
        std::path::Path::new("/Users/bythewc/Projects/camshaft/cadenza/spec/semantics");

    if args.contains(&"--print".to_string()) {
        // Read s-exprs from stdin (one per line), print ML + confirm round-trip.
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.unwrap();
            if line.trim().is_empty() {
                continue;
            }
            match ast::read(&line) {
                Ok(node) => {
                    let ml = print_ml(&node);
                    let ok = read_ml(&ml).map(|b| b == node).unwrap_or(false);
                    println!("S-EXPR: {}", line);
                    println!("ML    : {}", ml);
                    println!("round-trips: {}\n", ok);
                }
                Err(e) => println!("parse error on {:?}: {:?}\n", line, e),
            }
        }
        return;
    }

    if args.contains(&"--parse".to_string()) {
        // Read HUMAN-written ML from stdin (one per line), parse it, print the resulting Ast.
        // Proves the lexer/reader honors human intent, not just its own printer's output.
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.unwrap();
            if line.trim().is_empty() {
                continue;
            }
            println!("ML    : {}", line);
            match read_ml(&line) {
                Ok(node) => println!("AST   : {:?}\n", node),
                Err(e) => println!("ERROR : {}\n", e),
            }
        }
        return;
    }

    if args.contains(&"--survey".to_string()) {
        let heads = corpus_test::survey_heads(corpus_dir);
        let mut v: Vec<_> = heads.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        println!("head\tcount");
        for (h, c) in v {
            println!("{}\t{}", h, c);
        }
        return;
    }

    if args.contains(&"--corpus".to_string()) {
        use std::collections::BTreeMap;
        let mut entries: Vec<_> = std::fs::read_dir(corpus_dir)
            .expect("read corpus dir")
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sexp"))
            .collect();
        entries.sort();

        let mut total_passed = 0;
        let mut total_failed = 0;
        let mut merged: BTreeMap<String, (usize, String, String)> = BTreeMap::new();

        println!("=== Per-file ===");
        for path in &entries {
            let name = path.file_name().unwrap().to_string_lossy();
            let r = corpus_test::test_corpus_file(path);
            println!("{:42} {:4} pass  {:4} fail", name, r.passed, r.failed);
            total_passed += r.passed;
            total_failed += r.failed;
            for (k, (c, sample, reason)) in r.fail_buckets {
                let e = merged.entry(k).or_insert((0, sample, reason));
                e.0 += c;
            }
        }

        let total = total_passed + total_failed;
        println!("\n=== Corpus Coverage ===");
        println!("Round-trip identical: {} / {}", total_passed, total);
        if total > 0 {
            println!("Success rate: {:.1}%", 100.0 * total_passed as f64 / total as f64);
        }

        println!("\n=== Per-construct failure buckets ===");
        let mut buckets: Vec<_> = merged.into_iter().collect();
        buckets.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        for (head, (count, sample, reason)) in &buckets {
            println!("{:20} x{:<5} [{}]", head, count, reason);
            println!("      e.g. {}", sample);
        }
        return;
    }

    // Default: precedence + hand-written round-trip demos.
    run_demos();
}

fn rt(sexpr: &str) -> (bool, String, Node, Result<Node, String>) {
    let original = ast::read(sexpr).expect("s-expr parse");
    let ml = print_ml(&original);
    let back = read_ml(&ml);
    let ok = matches!(&back, Ok(n) if *n == original);
    (ok, ml, original, back)
}

fn run_demos() {
    println!("=== Precedence: minimal-paren round-trips ===\n");
    let prec_cases = [
        "(+ 1 (* 2 3))",       // should print 1 + 2 * 3
        "(* (+ 1 2) 3)",       // should print (1 + 2) * 3
        "(- (- 1 2) 3)",       // left-assoc: 1 - 2 - 3
        "(- 1 (- 2 3))",       // right-nested needs parens: 1 - (2 - 3)
        "(and (or a b) c)",    // (a or b) and c
        "(or a (and b c))",    // a or b and c
        "(< (+ 1 2) (* 3 4))", // 1 + 2 < 3 * 4
        "(= (+ a b) (+ c d))", // a + b = c + d
    ];
    for c in prec_cases {
        let (ok, ml, _orig, _back) = rt(c);
        println!("{} s-expr {:28} =>  ML  {:22}", if ok { "OK " } else { "BAD" }, c, ml);
    }

    println!("\n=== Hand-written ML parses to the intended Ast (anti-collusion) ===\n");
    let hand = [
        ("1 + 2 * 3", "(+ 1 (* 2 3))"),
        ("(1 + 2) * 3", "(* (+ 1 2) 3)"),
        ("1 - 2 - 3", "(- (- 1 2) 3)"),
        ("a or b and c", "(or a (and b c))"),
        ("let x = 10 in x + 1", "(let ((x 10)) (+ x 1))"),
        ("if a < b then 1 else 2", "(if (< a b) 1 2)"),
        ("fn(x, y) => x + y", "(fn (x y) (+ x y))"),
        ("Sign.Neg", "(. Sign Neg)"),
        ("List.at(xs, 0)", "((. List at) xs 0)"),
        ("f(1, g(2), 3)", "(f 1 (g 2) 3)"),
        ("tuple(1, 2, 3)", "(tuple 1 2 3)"),
    ];
    for (ml, sexpr) in hand {
        let want = ast::read(sexpr).expect("sexpr");
        match read_ml(ml) {
            Ok(got) if got == want => println!("OK  {:24} => {}", ml, sexpr),
            Ok(got) => println!("BAD {:24} => got {:?} want {:?}", ml, got, want),
            Err(e) => println!("ERR {:24} => {}", ml, e),
        }
    }
}

#[cfg(test)]
mod spike_tests {
    use super::*;

    fn assert_rt(sexpr: &str) {
        let orig = ast::read(sexpr).expect("sexpr");
        let ml = print_ml(&orig);
        let back = read_ml(&ml).unwrap_or_else(|e| panic!("read_ml({:?}) failed: {}", ml, e));
        assert_eq!(orig, back, "round-trip mismatch; ML was {:?}", ml);
    }

    #[test]
    fn precedence_min_paren() {
        for c in [
            "(+ 1 (* 2 3))",
            "(* (+ 1 2) 3)",
            "(- (- 1 2) 3)",
            "(- 1 (- 2 3))",
            "(and (or a b) c)",
            "(or a (and b c))",
            "(< (+ 1 2) (* 3 4))",
            "(| (& 300 127) 128)",
        ] {
            assert_rt(c);
        }
    }

    #[test]
    fn hand_written_precedence() {
        // 1 + 2 * 3 must be (+ 1 (* 2 3)), NOT (* (+ 1 2) 3)
        assert_eq!(read_ml("1 + 2 * 3").unwrap(), ast::read("(+ 1 (* 2 3))").unwrap());
        assert_eq!(read_ml("1 - 2 - 3").unwrap(), ast::read("(- (- 1 2) 3)").unwrap());
        assert_eq!(read_ml("a or b and c").unwrap(), ast::read("(or a (and b c))").unwrap());
    }

    #[test]
    fn core_forms() {
        for c in [
            "42", "true", "\"hi\"",
            "(let ((x 10)) x)",
            "(if (< 1 2) 10 20)",
            "(fn (x y) (+ x y))",
            "(. Sign Neg)",
            "(list 1 2 3)",
            "(tuple 1 2 3)",
            "(record (x 1) (y 2))",
            "(module m (def (main) 42))",
        ] {
            assert_rt(c);
        }
    }

    #[test]
    fn match_keyword_form() {
        // guard arms, constructor patterns, quote patterns, wildcard, else, nested match
        for c in [
            "(match n ((< n 0) A) ((= n 0) B) (else C))",
            "(match e ((Node.NLit v) v) ((Node.NAdd (tuple a b)) (+ a b)))",
            "(match (quote (+ 1 2)) ((Ast.List elems) (List.len elems)) (_ 0))",
            "(match x ((Ok a) a) ((Err _) false))",
            "(match a ((Some inner) (match inner ((Some y) y) ((None _) 0))) (None 0))",
        ] {
            assert_rt(c);
        }
    }

    #[test]
    fn match_hand_written() {
        // Human writes the block form; must parse to the identical (match …) Ast.
        assert_eq!(
            read_ml("match n {\n  n < 0 => Neg,\n  else => Pos\n}").unwrap(),
            ast::read("(match n ((< n 0) Neg) (else Pos))").unwrap()
        );
        assert_eq!(
            read_ml("match x { Some(y) => y, None => 0 }").unwrap(),
            ast::read("(match x ((Some y) y) (None 0))").unwrap()
        );
    }

    #[test]
    fn quasiquote_sigils() {
        for c in [
            "(quasiquote (+ 1 2))",
            "(let ((x 2)) (quasiquote ((unquote x) + 10)))", // structural
            "`(+ ,x 10)",
            "`(+ ,(+ 1 1) 10)",
            "`(a ,(+ b 1))",
            "(let ((x 1)) (= `(f ,x) (quote (f 1))))",
            "`(,@xs 3)", // splice at head position (needs parens on head)
        ] {
            assert_rt(c);
        }
    }

    #[test]
    fn quasiquote_hand_written() {
        // Human writes ML sigils; must parse to identical quasiquote/unquote Ast.
        assert_eq!(
            read_ml("`{ ,x + 10 }").unwrap(),
            ast::read("`(+ ,x 10)").unwrap()
        );
        assert_eq!(
            read_ml("`{ ,{ 1 + 1 } + 10 }").unwrap(),
            ast::read("`(+ ,(+ 1 1) 10)").unwrap()
        );
    }

    #[test]
    fn kebab_rule() {
        // `-` between word chars is part of the identifier; spaced `-` is subtraction.
        assert_eq!(read_ml("byte-at(b, 3)").unwrap(), ast::read("(byte-at b 3)").unwrap());
        assert_eq!(read_ml("a - b").unwrap(), ast::read("(- a b)").unwrap());
        assert_eq!(read_ml("x - 1").unwrap(), ast::read("(- x 1)").unwrap());
        assert_eq!(read_ml("a-b").unwrap(), Node::Name("a-b".into()));
        // The cost, made explicit: `x-1` is the identifier "x-1", NOT subtraction.
        assert_eq!(read_ml("x-1").unwrap(), Node::Name("x-1".into()));
    }
}
