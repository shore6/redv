//! redv - diagnostics
//!
//! エラーは `Result`(`RvResult`)で伝播する。`RvError` は行番号付きのエラー。
//! `fail(line, msg)` は `Err(..)` を返すヘルパ、`warn(line, msg)` は stderr へ警告を出す。

use std::fmt;

#[derive(Debug, Clone)]
pub struct RvError {
    pub line: i32,
    pub msg: String,
}

impl fmt::Display for RvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.msg)
        } else {
            write!(f, "{}", self.msg)
        }
    }
}

impl std::error::Error for RvError {}

pub type RvResult<T> = Result<T, RvError>;

/// `Err` を生成して返す(呼び出し側で `?` か `return` する)。
pub fn fail<T>(line: i32, msg: impl Into<String>) -> RvResult<T> {
    Err(RvError {
        line,
        msg: msg.into(),
    })
}

/// stderr へ警告を出力する。
pub fn warn(line: i32, msg: impl AsRef<str>) {
    if line > 0 {
        eprintln!("[warning] line {}: {}", line, msg.as_ref());
    } else {
        eprintln!("[warning] {}", msg.as_ref());
    }
}
