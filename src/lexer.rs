//! redv - lexer
//!
//! バイト列を走査してトークン列を生成する。文字列リテラルのエスケープ・`//` 行コメント・
//! `/* */` ブロックコメント・2 文字演算子 (`<=` `>=` `==` `!=` `&&` `||`) をここで処理する。

use crate::diag::{fail_at, RvResult};

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
    /// このトークンが始まる **桁**(行内の 1 始まりバイト位置)。診断のキャレット用。
    pub col: i32,
    /// ソース上でこのトークンが占めるバイト長(キャレットの下線幅)。
    pub len: i32,
}

pub struct Lexer {
    s: Vec<u8>,
    p: usize,
    line: i32,
    /// 現在行の先頭バイト位置(桁 = `p - line_start + 1`)。改行を消すたびに更新する。
    line_start: usize,
}

impl Lexer {
    pub fn new(src: impl Into<String>) -> Self {
        Lexer {
            s: src.into().into_bytes(),
            p: 0,
            line: 1,
            line_start: 0,
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
            let b = self.p; // トークン開始バイト
            let col = (self.p - self.line_start) as i32 + 1; // 1 始まりの桁

            if c.is_ascii_alphabetic() || c == b'_' {
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
                    col,
                    len: (self.p - b) as i32,
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
                    col,
                    len: (self.p - b) as i32,
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
                            self.line_start = self.p + 1;
                        }
                        bytes.push(ch);
                    }
                    self.p += 1;
                }
                if self.p >= self.s.len() {
                    return fail_at(ln, col, 1, "unterminated string literal");
                }
                self.p += 1;
                out.push(Token {
                    k: Tk::Str,
                    s: String::from_utf8_lossy(&bytes).into_owned(),
                    num: 0,
                    line: ln,
                    col,
                    len: (self.p - b) as i32,
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
                            self.line_start = self.p + 1;
                        }
                        self.p += 1;
                    }
                    if self.p + 1 >= self.s.len() {
                        return fail_at(ln, col, 2, "unterminated block comment");
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
                let plen = pc.len() as i32;
                self.p += pc.len();
                out.push(Token {
                    k: Tk::Punct,
                    s: pc,
                    num: 0,
                    line: ln,
                    col,
                    len: plen,
                });
            }
        }
        out.push(Token {
            k: Tk::End,
            s: String::new(),
            num: 0,
            line: self.line,
            col: (self.p - self.line_start) as i32 + 1,
            len: 1,
        });
        Ok(out)
    }

    fn skip_ws(&mut self) {
        while self.p < self.s.len() {
            let c = self.s[self.p];
            if c == b'\n' {
                self.line += 1;
                self.p += 1;
                self.line_start = self.p;
            } else if c.is_ascii_whitespace() {
                self.p += 1;
            } else {
                break;
            }
        }
    }
}
