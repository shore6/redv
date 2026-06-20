//! redv - diagnostics
//!
//! エラーは `Result`(`RvResult`)で伝播する。`RvError` は行・桁付きのエラー。
//! `fail(line, msg)` は行レベルの `Err(..)` を返すヘルパ、`fail_at(line, col, len, msg)` は
//! キャレット位置(桁・下線幅)付き。`warn(line, msg)` は stderr へ警告を出す。
//!
//! 表示は Rust コンパイラ風のキャレット診断(`--> file:line:col` + ソース行 + `^` 下線)。
//! ソース行を引くため、`set_source()` で字句解析前にファイル名と全行を登録しておく(issue #47)。

use std::fmt;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct RvError {
    pub line: i32,
    /// 1 始まりの桁。`0` なら桁不明(行レベル: 行の内容全体を下線する)。
    pub col: i32,
    /// 下線するバイト幅(`col > 0` のとき有効)。
    pub len: i32,
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

/// `Err`(行レベル = 桁不明)を生成して返す(呼び出し側で `?` か `return` する)。
pub fn fail<T>(line: i32, msg: impl Into<String>) -> RvResult<T> {
    Err(RvError {
        line,
        col: 0,
        len: 0,
        msg: msg.into(),
    })
}

/// `Err`(桁・下線幅つき)を生成して返す。lexer / parser のようにトークン位置が
/// 分かる箇所で使い、ソース行の正確な列にキャレットを出す。
pub fn fail_at<T>(line: i32, col: i32, len: i32, msg: impl Into<String>) -> RvResult<T> {
    Err(RvError {
        line,
        col,
        len: len.max(1),
        msg: msg.into(),
    })
}

// ---- source registry (caret rendering) ---------------------------------

struct Source {
    file: String,
    lines: Vec<String>,
}

static SOURCE: OnceLock<Source> = OnceLock::new();

/// 診断のキャレット表示で引くソースを登録する(字句解析前に 1 回)。
pub fn set_source(file: &str, src: &str) {
    let _ = SOURCE.set(Source {
        file: file.to_string(),
        lines: src.lines().map(|l| l.to_string()).collect(),
    });
}

/// エラーを Rust 風のキャレット診断で stderr に出す(main の終了処理から呼ぶ)。
pub fn report_error(e: &RvError) {
    render("error", e.line, e.col, e.len, &e.msg);
}

/// stderr へ警告を出力する(エラーと同じキャレット表示)。
pub fn warn(line: i32, msg: impl AsRef<str>) {
    render("warning", line, 0, 0, msg.as_ref());
}

/// 文字列 `s` の表示上の桁数(タブは 1 桁、その他は 1 文字 1 桁で数える簡易版)。
/// マルチバイト(日本語など)の全角幅は考慮しないが、エラー行は ASCII のコード片が
/// 大半なのでキャレットはおおむね合う。
fn display_cols(s: &str) -> usize {
    s.chars().count()
}

/// 共通のキャレット描画。`kind` は "error" / "warning"。
/// `col == 0` は行レベル(ソース行の内容全体 — 行末コメントは除く — を下線)。
fn render(kind: &str, line: i32, col: i32, len: i32, msg: &str) {
    let src = SOURCE.get();
    // ソース未登録 / 行番号なし / 行範囲外 はキャレットなしの簡易表示にフォールバック。
    let line_text = match src {
        Some(s) if line >= 1 && (line as usize) <= s.lines.len() => &s.lines[line as usize - 1],
        _ => {
            if line > 0 {
                eprintln!("[{}] line {}: {}", kind, line, msg);
            } else {
                eprintln!("[{}] {}", kind, msg);
            }
            return;
        }
    };
    let file = &src.unwrap().file;

    // 下線範囲(バイト位置 [u0, u1))を決める。
    let bytes = line_text.as_bytes();
    let (u0, u1) = if col > 0 {
        let start = (col as usize - 1).min(bytes.len());
        let end = (start + len.max(1) as usize).min(bytes.len());
        (start, end)
    } else {
        // 行レベル: 行頭の空白を飛ばし、行末コメント `//` の手前までを内容とみなす。
        let code_end = line_text.find("//").unwrap_or(bytes.len());
        let first = (0..code_end)
            .find(|&i| !bytes[i].is_ascii_whitespace())
            .unwrap_or(0);
        let last = (first..code_end)
            .rev()
            .find(|&i| !bytes[i].is_ascii_whitespace())
            .map(|i| i + 1)
            .unwrap_or(first + 1);
        (first, last.min(bytes.len()))
    };
    let shown_col = u0 as i32 + 1;

    // キャレットのインデント幅 / 下線幅は **文字数** で測る(バイトではなく)。
    // バイト境界が文字中なら get() が None になるので空文字で安全に退避する。
    let indent = display_cols(line_text.get(..u0).unwrap_or(""));
    let underline = display_cols(line_text.get(u0..u1).unwrap_or("")).max(1);

    let gw = line.to_string().len();
    let pad = " ".repeat(gw);

    eprintln!("[{}] {}", kind, msg);
    eprintln!("{}--> {}:{}:{}", pad, file, line, shown_col);
    eprintln!("{} |", pad);
    eprintln!("{:>gw$} | {}", line, line_text, gw = gw);
    eprintln!(
        "{} | {}{}",
        pad,
        " ".repeat(indent),
        "^".repeat(underline)
    );
}
