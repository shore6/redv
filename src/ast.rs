//! redv - AST
//!
//! C++ 版 `ast.hpp` の移植。原実装は「enum タグ + 全バリアントのフィールドを 1 構造体に
//! 同居」という形だったが、Rust 版では **データ付き enum** にして不正状態を表現不能にする。
//! これにより素子種別・文種別を追加した際にコンパイラが網羅性を検査できる。

use std::collections::HashMap;

// ---- logic (circuit) side ----------------------------------------------

/// 素子。`k` は素子種別、`n` はリピータ遅延。
#[derive(Debug, Clone, Copy)]
pub struct Elem {
    /// 'd' dust, 'x' dust-cross, 'b' block, 'r' repeater, 't' torch,
    /// 'C' comparator(compare), 'S' comparator(subtract)
    pub k: char,
    pub n: i32,
    pub line: i32,
}

#[derive(Debug, Clone)]
pub struct Port {
    pub input: bool,
    pub name: String,
    pub line: i32,
}

/// reg 宣言の初期化子。`strength == -1` は信号強度未指定。
#[derive(Debug, Clone)]
pub struct RegInit {
    pub strength: i32,
    pub tok: String,
}

/// 修飾子: plain / const / mutable
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qual {
    Plain,
    Const,
    Mutable,
}

#[derive(Debug, Clone)]
pub enum LogicStmt {
    DeclWire {
        line: i32,
        names: Vec<String>,
    },
    /// 原実装の DeclReg は常に名前ちょうど 1 つを持つ(parser がカンマ列を分割する)。
    DeclReg {
        line: i32,
        name: String,
        qual: Qual,
        init: Option<RegInit>,
    },
    /// `target = from -chunks...- to`
    ///
    /// `from_side` / `to_side` は端点に `.side` サフィックスが付いていたか
    /// (コンパレータの横入力端子指定)。
    AssignChain {
        line: i32,
        target: String,
        from: String,
        from_side: bool,
        to: String,
        to_side: bool,
        chunks: Vec<String>,
    },
    /// `target = [strength] rhs` (strength == -1 は未指定)
    AssignSingle {
        line: i32,
        target: String,
        strength: i32,
        rhs: String,
    },
    /// `output = callee(args...)` — 別 logic の階層インスタンス化。
    /// `output` は親の reg / ポート、`args` は親の reg / ポート名。
    Instance {
        line: i32,
        output: String,
        callee: String,
        args: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct LogicDef {
    pub name: String,
    pub line: i32,
    pub ports: Vec<Port>,
    pub stmts: Vec<LogicStmt>,
}

// ---- module / sim (testbench) side -------------------------------------

#[derive(Debug, Clone)]
pub enum Expr {
    Num {
        line: i32,
        num: i64,
    },
    Var {
        line: i32,
        name: String,
    },
    Time {
        line: i32,
    },
    Bin {
        line: i32,
        op: String,
        a: Box<Expr>,
        b: Box<Expr>,
    },
    Un {
        line: i32,
        op: String,
        a: Box<Expr>,
    },
}

impl Expr {
    /// 式が現れたソース行。診断用の AST アクセサ(将来のエラー位置精緻化向けに保持)。
    #[allow(dead_code)]
    pub fn line(&self) -> i32 {
        match self {
            Expr::Num { line, .. }
            | Expr::Var { line, .. }
            | Expr::Time { line }
            | Expr::Bin { line, .. }
            | Expr::Un { line, .. } => *line,
        }
    }
}

/// monitor() / wait() などの呼び出しデータ(Call と MonReg で共有)。
#[derive(Debug, Clone)]
pub struct CallData {
    pub callee: String,
    pub has_fmt: bool,
    pub fmt: String,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone)]
pub enum SimStmt {
    DeclVar {
        line: i32,
        decls: Vec<(String, Option<Expr>)>,
    },
    Assign {
        line: i32,
        target: String,
        value: Expr,
    },
    CallBind {
        line: i32,
        target: String,
        callee: String,
        bind_args: Vec<String>,
    },
    WaitTicks {
        line: i32,
        ticks: i64,
    },
    WaitInit {
        line: i32,
    },
    Call {
        line: i32,
        call: CallData,
    },
    /// `?monitor(...)`: sim 開始時にホイストされる
    MonReg {
        line: i32,
        call: CallData,
    },
    If {
        line: i32,
        cond: Expr,
        body: Vec<SimStmt>,
        else_body: Vec<SimStmt>,
    },
    While {
        line: i32,
        cond: Expr,
        body: Vec<SimStmt>,
    },
    For {
        line: i32,
        init: Option<Box<SimStmt>>,
        cond: Option<Expr>,
        post: Option<Box<SimStmt>>,
        body: Vec<SimStmt>,
    },
}

#[derive(Debug, Clone)]
pub struct ModuleDef {
    pub name: String,
    pub line: i32,
    /// sim 外の var 宣言
    pub pre: Vec<SimStmt>,
    /// sim ブロック本体(複数ブロックは連結)
    pub sim: Vec<SimStmt>,
    pub has_sim: bool,
}

#[derive(Debug, Default)]
pub struct Program {
    pub defines: HashMap<String, i64>,
    pub str_defines: HashMap<String, String>,
    pub logics: HashMap<String, LogicDef>,
    pub modules: Vec<ModuleDef>,
}
