//! redv - AST
//!
//! 構文木。**データ付き enum** で不正状態を表現不能にしてある。
//! これにより素子種別・文種別を追加した際にコンパイラが網羅性を検査できる。

use std::collections::{HashMap, HashSet};

// ---- logic (circuit) side ----------------------------------------------

/// 素子。`k` は素子種別、`n` はリピータ遅延またはオブザーバのエッジ判定モード
/// (0=変化全部, 1=立ち上がり, 2=立ち下がり, 3=2値エッジ。`parse_chunk` 参照)。
#[derive(Debug, Clone, Copy)]
pub struct Elem {
    /// 'd' dust, 'b' block, 'r' repeater, 't' torch, 'o' observer,
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
    /// `Some(we)` なら **バスポート**(`input[n]` / `output[n]`): n 本の並列レーン。
    /// 本体では内部バス reg(§4.2)と同じく添字 / バス全体で使える。`None` はスカラ。
    /// 幅は `WidthExpr`(リテラル即決 or 遅延式)で、logic のジェネリック param を
    /// 含む場合のみ式のまま残り、インスタンス時に解決される(§ Phase 2)。
    pub width: Option<WidthExpr>,
}

/// バス幅。`Lit(n)` は parse 時に解決済みのリテラル(既存挙動)。
/// `Expr(e)` は logic ローカル param を含むため elaborate 時に `param_env` 下で解決する
/// **遅延式**(Phase 2 のジェネリック幅。例: `input[W]` / `reg[W+1]`)。
#[derive(Debug, Clone)]
pub enum WidthExpr {
    Lit(i32),
    Expr(Expr),
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

/// レーン / スライスの添字。`Lit(k)` は parse 時に解決済みのリテラル(既存挙動)。
/// `Expr(e)` は logic ローカルのジェネリック param を含むため elaborate 時に
/// `param_env` 下で解決する **遅延式**(issue #89。`WidthExpr` と同じ二相解決)。
#[derive(Debug, Clone)]
pub enum IdxExpr {
    Lit(i32),
    Expr(Expr),
}

/// チェーン端点のレーン選択。バス名や reg 名に付く。
/// 添字は定数式(リテラル・`param`・数値 `#define`・`+ - * / %`・単項 `-`・括弧)。
#[derive(Debug, Clone)]
pub enum Sel {
    /// 添字なし。スカラ点 / バス全体。
    All,
    /// `name[k]` — 単一レーン。
    Lane(IdxExpr),
    /// `name[hi:lo]` — スライス(包含)。`hi >= lo` で降順、`hi < lo` で昇順。
    Slice(IdxExpr, IdxExpr),
}

/// logic 呼び出しの引数。裸の名前(reg / ポート / var / バス)か、ネストした
/// logic 呼び出し(issue #97)。ネスト呼び出しは **出力ポートちょうど 1 個** の logic に
/// 限り、その出力ポートが外側の入力ポートへ減衰なしで直結される(中間 reg / var を
/// 手書きした場合と等価な配線)。logic 本体(`Instance`)と sim(`CallBind`)で共用する。
#[derive(Debug, Clone)]
pub enum InstArg {
    Name(String),
    Call {
        line: i32,
        callee: String,
        params: Vec<(String, Expr)>,
        args: Vec<InstArg>,
    },
}

/// チェーン端点。スカラ点 / バス全体 / レーン / スライス / 連結のいずれか。
/// レーン列(`Vec<usize>`)に解決され、両端の幅が一致すれば element-wise 接続される。
#[derive(Debug, Clone)]
pub enum Endpoint {
    /// 名前 + レーン選択 + `.side`(コンパレータ/リピーター横入力)。
    Ref { name: String, side: bool, sel: Sel },
    /// `{e1, e2, ...}` — 連結(左から順にレーンを連接)。各要素は `Ref`(`side` 不可)。
    Concat(Vec<Endpoint>),
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
        width: Option<WidthExpr>,
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
    /// 独立展開される。両端は `Endpoint`(スカラ / バス全体 / レーン / スライス / 連結)。
    /// 端点はレーン列に解決され、両端の幅(レーン数)が一致すれば element-wise 接続される。
    Chain {
        line: i32,
        from: Endpoint,
        to: Endpoint,
        chunks: Vec<String>,
    },
    /// `target = [strength] rhs` (strength == -1 は未指定)
    AssignSingle {
        line: i32,
        target: String,
        strength: i32,
        rhs: String,
    },
    /// `(o1, o2, ...) = callee#(P=v, ...)(args...)` — 別 logic の階層インスタンス化。
    /// `outputs` は親の reg / ポート列(出力ポートと位置対応)、`args` は親の reg / ポート名
    /// またはネストした logic 呼び出し(`InstArg`、issue #97)。
    /// `params` はジェネリック幅の実引数(`#(W=8)` 等。空なら既定値で展開する。Phase 2)。
    /// 出力 1 個の `out = callee(...)` は `outputs = vec![out]` として正規化する(`(out) = ...` も同義)。
    Instance {
        line: i32,
        outputs: Vec<String>,
        callee: String,
        args: Vec<InstArg>,
        params: Vec<(String, Expr)>,
    },
}

#[derive(Debug, Clone)]
pub struct LogicDef {
    pub name: String,
    pub line: i32,
    pub ports: Vec<Port>,
    pub stmts: Vec<LogicStmt>,
    /// ジェネリック幅パラメータ宣言 `#(W=4, X)`(Phase 2)。各エントリは (名前, 既定値)。
    /// 既定値が `None` の param は呼び出し側で `#(W=...)` 必須。空なら従来の非ジェネリック logic。
    pub params: Vec<(String, Option<i64>)>,
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
    /// `(t1, t2, ...) = callee#(P=v, ...)(args)` — sim から logic をインスタンス化する束縛。
    /// `targets` は出力ポートと位置対応の束縛先 var 列(スカラ var / バス var)。出力 1 個の
    /// `t = callee(...)` は `targets = vec![t]` として正規化する(`(t) = ...` も同義)。
    /// `bind_args` は var 名またはネストした logic 呼び出し(`InstArg`、issue #97)。
    /// `params` はジェネリック幅の実引数(`#(W=8)` 等。空なら既定値で展開する。Phase 2)。
    /// `fmt` は `scan("%x")` のような書式付き入力で使う(scan のみ。それ以外は常に `None`。
    /// scan は常に `targets.len() == 1`)。
    CallBind {
        line: i32,
        targets: Vec<String>,
        callee: String,
        bind_args: Vec<InstArg>,
        params: Vec<(String, Expr)>,
        fmt: Option<String>,
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
    /// 既に取り込んだバンドル stdlib 名(`#include "stdlogic"` の重複取り込み防止用)。
    /// 同じ stdlib を複数ファイルから include しても 2 度目以降は no-op になる。
    pub included_stdlibs: HashSet<String>,
}
