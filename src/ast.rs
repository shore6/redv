//! redv - AST
//!
//! 構文木。**データ付き enum** で不正状態を表現不能にしてある。
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
    /// `Some(n)` なら **バスポート**(`input[n]` / `output[n]`): n 本の並列レーン。
    /// 本体では内部バス reg(§4.2)と同じく添字 / バス全体で使える。`None` はスカラ。
    pub width: Option<i32>,
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
    /// DeclReg は常に名前ちょうど 1 つを持つ(parser がカンマ列を分割する)。
    ///
    /// `width` が `Some(n)` なら **バス reg**(`reg[n] name;`): n 本の並列レーン
    /// `name[0]`..`name[n-1]` を宣言する。各レーンは通常の plain 点で、`circuit` から見れば
    /// 独立したスカラ点(バスは純粋な糖衣でシミュレーション意味論は不変)。
    /// バス reg は plain のみ・初期化子不可(Phase 1a)。`width == None` は従来のスカラ reg。
    DeclReg {
        line: i32,
        name: String,
        qual: Qual,
        init: Option<RegInit>,
        width: Option<i32>,
    },
    /// `target = elem-chunks...` — wire への **素子列定義**。
    ///
    /// 端点を持たない再利用可能な素子列を wire に束縛する(接続ではない)。
    /// パーサ段階では `target = a-b-c` 一般を受け、interp が target が wire の場合に
    /// `[from] + chunks + [to]` を素子トークン列として解釈する(reg target はエラー)。
    /// `from_side` / `to_side` は素子列定義では不可(`.side` は接続のみ)。
    AssignChain {
        line: i32,
        target: String,
        from: String,
        from_side: bool,
        to: String,
        to_side: bool,
        chunks: Vec<String>,
    },
    /// `from -chunks...- to` — チェーン接続文(2 点を素子列でつなぐ)。
    ///
    /// 中間チャンクには素子に加えて **wire 名** を書け、その素子列が各箇所に
    /// 独立展開される。`from_side` / `to_side` は端点の `.side`(コンパレータ横入力)。
    ///
    /// `from_idx` / `to_idx` が `Some(k)` なら端点はバスのレーン `name[k]`。`None` かつ端点が
    /// バス名なら **バス全体**(同幅の両端を element-wise に展開する)。スカラ端点は幅 1。
    Chain {
        line: i32,
        from: String,
        from_side: bool,
        from_idx: Option<i32>,
        to: String,
        to_side: bool,
        to_idx: Option<i32>,
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
        /// `Some(e)` なら **バス var のレーン** `name[e]`(e は実行時に評価する添字式)。
        /// `None` はスカラ var。`None` かつ name がバス var の場合は評価時エラー(レーン指定が必要)。
        index: Option<Box<Expr>>,
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
    /// var 宣言。各エントリは (名前, 初期化式, バス幅)。`width` が `Some(n)` なら
    /// **バス var**(`var[n] x;`): n 本のレーン `x[0]`..`x[n-1]`(初期化式は全レーンに適用)。
    DeclVar {
        line: i32,
        decls: Vec<(String, Option<Expr>, Option<i32>)>,
    },
    /// `target[index] = value [~ width]` — var への代入。
    ///
    /// `index` が `Some(e)` なら **バス var のレーン** `target[e]` への代入。`None` で
    /// `target` がバス var の場合は **全レーンへブロードキャスト**(value を各レーンに代入)。
    /// `pulse` が `Some(width)` のとき **パルス代入**: 代入後 `width` tick 経過すると
    /// 自動的に `target` を 0 に戻す(`width` は実行されたあらゆる tick を数える)。
    Assign {
        line: i32,
        target: String,
        index: Option<Box<Expr>>,
        value: Expr,
        pulse: Option<Expr>,
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
    /// `#until(cond)` — `cond` が真(!=0)になるまで tick を進める。
    /// `$time` は `#n` 同様に進む。`INIT_TIMEOUT` 超過でエラー。
    WaitUntil {
        line: i32,
        cond: Expr,
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
