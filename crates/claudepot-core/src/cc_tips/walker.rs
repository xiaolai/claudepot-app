//! Token-aware byte walker for embedded JS in the CC binary.
//!
//! Pure-Rust state machine that recognizes the JS lexical structures
//! we need to skip-over correctly: string literals (single, double,
//! template), regex literals (with character classes), comments
//! (line, block), and bracket nesting. **It is not a parser** — it
//! does not produce an AST. It only knows enough to advance the
//! cursor past one balanced unit at a time, so caller code can find
//! matching `{}` / `[]` / `()` without being fooled by literal text
//! that contains those characters.
//!
//! Why we need this: pure regex extraction breaks on regex literals
//! inside `isRelevant` predicates (e.g.
//! `filePath:/\.(html|css|htm)$/i`), nested `${...}` template
//! interpolations, and brace-balanced arrow-block bodies. All three
//! occur in CC's tip registry as of 2.1.132.
//!
//! Heuristic for `/` regex-vs-division (the single nontrivial bit):
//! when we see `/` in code mode, it starts a regex iff the **previous
//! non-whitespace, non-comment token** ends an expression *context*
//! rather than an expression *value*. We track the previous token's
//! "value-ness" in `prev_was_value` — `true` after identifiers,
//! numbers, string/template literals, `)` `]` — `false` after
//! operators, `(` `[` `{` `,` `;` `:` and value-context keywords
//! (`return`, `typeof`, `in`, `of`, `instanceof`, etc.).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Punct(u8),
    OpenBrace,
    CloseBrace,
    OpenBracket,
    CloseBracket,
    OpenParen,
    CloseParen,
    Comma,
    Semi,
    Arrow,
    StringLit,
    TemplateLit,
    RegexLit,
    Number,
    Ident,
    LineComment,
    BlockComment,
    Whitespace,
    Eof,
    BadByte,
}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub struct Walker<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// True iff the previous emitted token would put `/` in division
    /// position (i.e. it produced a value).
    prev_was_value: bool,
}

const VALUE_CTX_KEYWORDS: &[&[u8]] = &[
    b"return",
    b"typeof",
    b"delete",
    b"void",
    b"in",
    b"of",
    b"instanceof",
    b"new",
    b"throw",
    b"case",
    b"do",
    b"else",
    b"yield",
    b"await",
];

impl<'a> Walker<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            prev_was_value: false,
        }
    }

    pub fn from_offset(bytes: &'a [u8], pos: usize) -> Self {
        Self {
            bytes,
            pos,
            prev_was_value: false,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn set_pos(&mut self, p: usize) {
        self.pos = p;
        self.prev_was_value = false;
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Advance one lexical token. Returns `(token_kind, span)`.
    /// Whitespace and comments are emitted (caller can skip).
    pub fn next_tok(&mut self) -> (Tok, Span) {
        let start = self.pos;
        if self.pos >= self.bytes.len() {
            return (Tok::Eof, Span { start, end: start });
        }
        let b = self.bytes[self.pos];

        // Whitespace
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
            while self.pos < self.bytes.len()
                && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
            {
                self.pos += 1;
            }
            return (
                Tok::Whitespace,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }

        // Comments
        if b == b'/' && self.pos + 1 < self.bytes.len() {
            let n = self.bytes[self.pos + 1];
            if n == b'/' {
                self.pos += 2;
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                return (
                    Tok::LineComment,
                    Span {
                        start,
                        end: self.pos,
                    },
                );
            }
            if n == b'*' {
                self.pos += 2;
                while self.pos + 1 < self.bytes.len()
                    && !(self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/')
                {
                    self.pos += 1;
                }
                if self.pos + 1 < self.bytes.len() {
                    self.pos += 2;
                }
                return (
                    Tok::BlockComment,
                    Span {
                        start,
                        end: self.pos,
                    },
                );
            }
        }

        // String literals
        if b == b'"' || b == b'\'' {
            let quote = b;
            self.pos += 1;
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c == b'\\' && self.pos + 1 < self.bytes.len() {
                    self.pos += 2;
                    continue;
                }
                if c == quote {
                    self.pos += 1;
                    self.prev_was_value = true;
                    return (
                        Tok::StringLit,
                        Span {
                            start,
                            end: self.pos,
                        },
                    );
                }
                if c == b'\n' && quote != b'`' {
                    // unterminated string — bail
                    break;
                }
                self.pos += 1;
            }
            self.prev_was_value = true;
            return (
                Tok::StringLit,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }

        // Template literals
        if b == b'`' {
            self.pos += 1;
            self.skip_template_body();
            self.prev_was_value = true;
            return (
                Tok::TemplateLit,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }

        // Regex literal (heuristic on prev_was_value)
        if b == b'/' && !self.prev_was_value {
            // Disambiguated as regex.
            self.pos += 1;
            let mut in_class = false;
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c == b'\\' && self.pos + 1 < self.bytes.len() {
                    self.pos += 2;
                    continue;
                }
                if c == b'[' {
                    in_class = true;
                } else if c == b']' {
                    in_class = false;
                } else if c == b'/' && !in_class {
                    self.pos += 1;
                    // Skip flags
                    while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_alphabetic()
                    {
                        self.pos += 1;
                    }
                    self.prev_was_value = true;
                    return (
                        Tok::RegexLit,
                        Span {
                            start,
                            end: self.pos,
                        },
                    );
                } else if c == b'\n' {
                    break;
                }
                self.pos += 1;
            }
            self.prev_was_value = true;
            return (
                Tok::RegexLit,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }

        // Numbers
        if b.is_ascii_digit() || (b == b'.' && self.peek_is_digit(self.pos + 1)) {
            self.pos += 1;
            while self.pos < self.bytes.len() {
                let c = self.bytes[self.pos];
                if c.is_ascii_digit() || c == b'.' || c == b'_' {
                    self.pos += 1;
                } else if matches!(c, b'e' | b'E') {
                    self.pos += 1;
                    if self.pos < self.bytes.len() && matches!(self.bytes[self.pos], b'+' | b'-') {
                        self.pos += 1;
                    }
                } else if c.is_ascii_alphabetic() {
                    // BigInt suffix `n` or numeric type tag
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.prev_was_value = true;
            return (
                Tok::Number,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }

        // Identifiers
        if is_ident_start(b) {
            self.pos += 1;
            while self.pos < self.bytes.len() && is_ident_cont(self.bytes[self.pos]) {
                self.pos += 1;
            }
            let span = Span {
                start,
                end: self.pos,
            };
            let ident = &self.bytes[start..self.pos];
            // Value-context keywords leave us in expression-start mode
            // (so a following `/` is regex). Other keywords/identifiers
            // (`async`, `function`, names, etc.) are values.
            self.prev_was_value = !VALUE_CTX_KEYWORDS.contains(&ident);
            return (Tok::Ident, span);
        }

        // Punctuation
        let result = match b {
            b'{' => (Tok::OpenBrace, false),
            b'}' => (Tok::CloseBrace, true),
            b'[' => (Tok::OpenBracket, false),
            b']' => (Tok::CloseBracket, true),
            b'(' => (Tok::OpenParen, false),
            b')' => (Tok::CloseParen, true),
            b',' => (Tok::Comma, false),
            b';' => (Tok::Semi, false),
            _ => (Tok::Punct(b), false),
        };
        // Arrow `=>` consumes two bytes
        if b == b'=' && self.peek_at(self.pos + 1) == Some(b'>') {
            self.pos += 2;
            self.prev_was_value = false;
            return (
                Tok::Arrow,
                Span {
                    start,
                    end: self.pos,
                },
            );
        }
        self.pos += 1;
        self.prev_was_value = result.1;
        (
            result.0,
            Span {
                start,
                end: self.pos,
            },
        )
    }

    /// Advance until the matching close at the same nesting level
    /// is consumed. Caller pre-positions cursor at the byte AFTER
    /// the opening `{` / `[` / `(`. Returns the byte offset of the
    /// matching closer (one past it), or None if EOF reached first.
    pub fn find_matching_close(&mut self, open: u8) -> Option<usize> {
        let close = match open {
            b'{' => b'}',
            b'[' => b']',
            b'(' => b')',
            _ => return None,
        };
        let mut depth: i32 = 1;
        loop {
            let (t, span) = self.next_tok();
            match t {
                Tok::Eof => return None,
                Tok::OpenBrace if open == b'{' => depth += 1,
                Tok::CloseBrace if open == b'{' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(span.end);
                    }
                }
                Tok::OpenBracket if open == b'[' => depth += 1,
                Tok::CloseBracket if open == b'[' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(span.end);
                    }
                }
                Tok::OpenParen if open == b'(' => depth += 1,
                Tok::CloseParen if open == b'(' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(span.end);
                    }
                }
                Tok::Punct(b) if b == close => {
                    // Mismatched (close char sneaks in despite tokenizer)
                    return None;
                }
                _ => {}
            }
        }
    }

    /// Skip whitespace + comments. Useful between fields.
    pub fn skip_trivia(&mut self) {
        loop {
            let saved = self.pos;
            let saved_pwv = self.prev_was_value;
            let (t, _) = self.next_tok();
            if !matches!(t, Tok::Whitespace | Tok::LineComment | Tok::BlockComment) {
                self.pos = saved;
                self.prev_was_value = saved_pwv;
                return;
            }
        }
    }

    fn skip_template_body(&mut self) {
        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            if c == b'\\' && self.pos + 1 < self.bytes.len() {
                self.pos += 2;
                continue;
            }
            if c == b'`' {
                self.pos += 1;
                return;
            }
            if c == b'$' && self.peek_at(self.pos + 1) == Some(b'{') {
                self.pos += 2;
                // Recurse into the interpolation expression.
                let _ = self.find_matching_close(b'{');
                continue;
            }
            self.pos += 1;
        }
    }

    fn peek_at(&self, i: usize) -> Option<u8> {
        self.bytes.get(i).copied()
    }

    fn peek_is_digit(&self, i: usize) -> bool {
        self.bytes
            .get(i)
            .map(|b| b.is_ascii_digit())
            .unwrap_or(false)
    }

    /// Slice the source bytes for a span.
    pub fn slice(&self, span: Span) -> &'a [u8] {
        &self.bytes[span.start..span.end]
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Tok> {
        let mut w = Walker::new(src.as_bytes());
        let mut out = Vec::new();
        loop {
            let (t, _) = w.next_tok();
            if t == Tok::Eof {
                break;
            }
            if !matches!(t, Tok::Whitespace) {
                out.push(t);
            }
        }
        out
    }

    #[test]
    fn ident_after_arrow_is_value() {
        // `=> foo` — `foo` is an identifier value.
        let toks = tokens("=> foo");
        assert_eq!(toks, vec![Tok::Arrow, Tok::Ident]);
    }

    #[test]
    fn regex_after_colon() {
        // `filePath:/\.html$/i` — slash starts a regex.
        let toks = tokens("filePath:/\\.html$/i");
        assert_eq!(toks.last(), Some(&Tok::RegexLit));
    }

    #[test]
    fn division_after_value() {
        // `a/b` — slash is division (two idents).
        let toks = tokens("a/b");
        assert!(toks.contains(&Tok::Punct(b'/')));
    }

    #[test]
    fn template_with_interpolation() {
        let toks = tokens("`hello ${world.foo}`");
        assert_eq!(toks, vec![Tok::TemplateLit]);
    }

    #[test]
    fn nested_template() {
        let toks = tokens("`a ${`b ${c}`} d`");
        assert_eq!(toks, vec![Tok::TemplateLit]);
    }

    #[test]
    fn matching_close_brace() {
        // After the open brace, find matching close.
        let src = b"{ foo: bar, x: { y: z }, }X";
        let mut w = Walker::from_offset(src, 1);
        let end = w.find_matching_close(b'{').unwrap();
        // end is one past the closing `}`.
        assert_eq!(src[end], b'X');
    }

    #[test]
    fn matching_close_with_regex() {
        // Brace-matching must skip over regex literal contents.
        let src = b"{ filePath: /\\}html$/i, x: 1 }X";
        let mut w = Walker::from_offset(src, 1);
        let end = w.find_matching_close(b'{').unwrap();
        assert_eq!(end, src.len() - 1);
    }

    #[test]
    fn matching_close_with_template_brace() {
        // `${...}` interpolation contains `}` which must NOT close the
        // outer brace.
        let src = b"{ msg: `hi ${user.name}`, }X";
        let mut w = Walker::from_offset(src, 1);
        let end = w.find_matching_close(b'{').unwrap();
        assert_eq!(src[end], b'X');
    }

    #[test]
    fn matching_close_with_string_brace() {
        let src = b"{ msg: \"a } b\", x: 1 }X";
        let mut w = Walker::from_offset(src, 1);
        let end = w.find_matching_close(b'{').unwrap();
        assert_eq!(src[end], b'X');
    }

    #[test]
    fn line_comment_skipped() {
        let src = b"{ // }\n x: 1 }X";
        let mut w = Walker::from_offset(src, 1);
        let end = w.find_matching_close(b'{').unwrap();
        assert_eq!(src[end], b'X');
    }
}
