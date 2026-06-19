//! redv - lexer
//!
//! バイト列を走査してトークン列を生成する。文字列リテラルのエスケープ・`//` 行コメント・
//! `/* */` ブロックコメント・2 文字演算子 (`<=` `>=` `==` `!=` `&&` `||`) をここで処理する。

use crate::diag::{fail, RvResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tk {
    Ident,
    Num,
    Str,
    Punct,
    End,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub k: Tk,
    /// ident text / string contents / punct text
    pub s: String,
    /// Num の値
    pub num: i64,
    pub line: i32,
}

pub struct Lexer {
    s: Vec<u8>,
    p: usize,
    line: i32,
}

impl Lexer {
    pub fn new(src: impl Into<String>) -> Self {
        Lexer {
            s: src.into().into_bytes(),
            p: 0,
            line: 1,
        }
    }

    pub fn run(mut self) -> RvResult<Vec<Token>> {
        let mut out: Vec<Token> = Vec::new();
        loop {
            self.skip_ws();
            if self.p >= self.s.len() {
                break;
            }
            let c = self.s[self.p];
            let ln = self.line;

            if c.is_ascii_alphabetic() || c == b'_' {
                let b = self.p;
                while self.p < self.s.len()
                    && (self.s[self.p].is_ascii_alphanumeric() || self.s[self.p] == b'_')
                {
                    self.p += 1;
                }
                let text = String::from_utf8_lossy(&self.s[b..self.p]).into_owned();
                out.push(Token {
                    k: Tk::Ident,
                    s: text,
                    num: 0,
                    line: ln,
                });
            } else if c.is_ascii_digit() {
                let mut v: i64 = 0;
                while self.p < self.s.len() && self.s[self.p].is_ascii_digit() {
                    v = v * 10 + (self.s[self.p] - b'0') as i64;
                    self.p += 1;
                }
                out.push(Token {
                    k: Tk::Num,
                    s: String::new(),
                    num: v,
                    line: ln,
                });
            } else if c == b'"' {
                self.p += 1;
                let mut bytes: Vec<u8> = Vec::new();
                while self.p < self.s.len() && self.s[self.p] != b'"' {
                    let ch = self.s[self.p];
                    if ch == b'\\' && self.p + 1 < self.s.len() {
                        self.p += 1;
                        let e = self.s[self.p];
                        match e {
                            b'n' => bytes.push(b'\n'),
                            b't' => bytes.push(b'\t'),
                            b'\\' => bytes.push(b'\\'),
                            b'"' => bytes.push(b'"'),
                            _ => {
                                bytes.push(b'\\');
                                bytes.push(e);
                            }
                        }
                    } else {
                        if ch == b'\n' {
                            self.line += 1;
                        }
                        bytes.push(ch);
                    }
                    self.p += 1;
                }
                if self.p >= self.s.len() {
                    return fail(ln, "unterminated string literal");
                }
                self.p += 1;
                out.push(Token {
                    k: Tk::Str,
                    s: String::from_utf8_lossy(&bytes).into_owned(),
                    num: 0,
                    line: ln,
                });
            } else {
                // comments
                if self.p + 1 < self.s.len() && self.s[self.p] == b'/' && self.s[self.p + 1] == b'/'
                {
                    while self.p < self.s.len() && self.s[self.p] != b'\n' {
                        self.p += 1;
                    }
                    continue;
                }
                if self.p + 1 < self.s.len() && self.s[self.p] == b'/' && self.s[self.p + 1] == b'*'
                {
                    self.p += 2;
                    while self.p + 1 < self.s.len()
                        && !(self.s[self.p] == b'*' && self.s[self.p + 1] == b'/')
                    {
                        if self.s[self.p] == b'\n' {
                            self.line += 1;
                        }
                        self.p += 1;
                    }
                    if self.p + 1 >= self.s.len() {
                        return fail(ln, "unterminated block comment");
                    }
                    self.p += 2;
                    continue;
                }

                let mut pc = String::from(c as char);
                if self.p + 1 < self.s.len() {
                    let d = &self.s[self.p..self.p + 2];
                    for t in ["<=", ">=", "==", "!=", "&&", "||"] {
                        if d == t.as_bytes() {
                            pc = t.to_string();
                            break;
                        }
                    }
                }
                self.p += pc.len();
                out.push(Token {
                    k: Tk::Punct,
                    s: pc,
                    num: 0,
                    line: ln,
                });
            }
        }
        out.push(Token {
            k: Tk::End,
            s: String::new(),
            num: 0,
            line: self.line,
        });
        Ok(out)
    }

    fn skip_ws(&mut self) {
        while self.p < self.s.len() {
            let c = self.s[self.p];
            if c == b'\n' {
                self.line += 1;
                self.p += 1;
            } else if c.is_ascii_whitespace() {
                self.p += 1;
            } else {
                break;
            }
        }
    }
}
