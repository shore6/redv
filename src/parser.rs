//! redv - parser
//!
//! 再帰下降パーサ。`#include` のサブパーサが同じ `Program` を共有するため、
//! `Program` は各メソッドに `&mut` 引数として渡す(構造体に保持しない)。

use crate::ast::*;
use crate::diag::{fail, fail_at, RvResult};
use crate::lexer::{Lexer, Tk, Token};
use std::collections::{HashMap, HashSet};
use std::fs;

/// `parse_callee_invocation` の返り値: (callee 名, `#(...)` ジェネリック実引数, 引数列)。
type CalleeInvocation = (String, Vec<(String, Expr)>, Vec<InstArg>);

/// バンドル済み標準ライブラリ。`#include "stdlogic"` でビルド時に埋め込まれたソースから読む。
/// ファイルシステムやインストール場所に依存せず、redv バイナリ単体でライブラリを利用できる。
const BUNDLED_STDLIBS: &[(&str, &str)] = &[
    ("stdlogic", include_str!("stdlib/stdlogic.rv")),
    ("stdmem", include_str!("stdlib/stdmem.rv")),
];

fn bundled_stdlib(name: &str) -> Option<&'static str> {
    BUNDLED_STDLIBS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

pub fn dir_of(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(p) => path[..p].to_string(),
        None => ".".to_string(),
    }
}

/// 端点が「裸の名前」(添字・`.side`・連結なし)ならその名前を返す。
/// チェーンの中間チャンク(素子列 / wire 名)はこの形でなければならない。
fn endpoint_bare_name(ep: &Endpoint) -> Option<String> {
    match ep {
        Endpoint::Ref { name, side: false, sel: Sel::All } => Some(name.clone()),
        _ => None,
    }
}

/// 端点を「名前 + `.side` + レーン選択」へ落とす。連結は `=` 右辺では使えないのでエラー。
fn endpoint_simple(ep: Endpoint, line: i32) -> RvResult<(String, bool, Sel)> {
    match ep {
        Endpoint::Ref { name, side, sel } => Ok((name, side, sel)),
        Endpoint::Concat(_) => fail(
            line,
            "a concatenation '{...}' is not allowed on the right-hand side of '='; \
             use a chain to wire a bus",
        ),
    }
}

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    base_dir: String,
    /// パラメータ定数表(`param` / 数値 `#define`)。幅 `[expr]` と式の解決に使う
    /// **パース時** のミラー。確定値は `prog.defines` 側にも書く(interp が読む)。
    consts: HashMap<String, i64>,
    /// 現在パース中の logic のジェネリック幅 param 名(Phase 2)。
    /// この集合に含まれる名前を参照する `[expr]` は `WidthExpr::Expr` で遅延し、
    /// elaborate 時に呼び出しごとの `param_env` で解決する。空なら logic 内でも従来通り
    /// 即時 `WidthExpr::Lit` に簡約(後方互換)。
    logic_params: HashSet<String>,
}

impl Parser {
    pub fn new(toks: Vec<Token>, base_dir: String) -> Self {
        Parser {
            toks,
            i: 0,
            base_dir,
            consts: HashMap::new(),
            logic_params: HashSet::new(),
        }
    }

    pub fn parse_file(&mut self, prog: &mut Program) -> RvResult<()> {
        while self.cur().k != Tk::End {
            if self.is_punct("#") {
                self.i += 1;
                self.parse_directive(prog)?;
            } else if self.is_ident("param") {
                self.i += 1;
                self.parse_param(prog)?;
            } else if self.is_ident("logic") {
                self.i += 1;
                self.parse_logic(prog)?;
            } else if self.is_ident("module") {
                self.i += 1;
                self.parse_module(prog)?;
            } else {
                return self.err_cur(
                    "expected 'param', 'logic', 'module', or '#' directive at top level",
                );
            }
        }
        Ok(())
    }

    // ---- token helpers -----------------------------------------------------

    fn cur(&self) -> &Token {
        &self.toks[self.i]
    }
    fn peek(&self, n: usize) -> &Token {
        let idx = (self.i + n).min(self.toks.len() - 1);
        &self.toks[idx]
    }
    /// 現在トークンの位置(行・桁・長さ)を指すエラー。構文エラーのキャレットは
    /// 「想定外だった今のトークン」を指すのが自然なので、ここに集約する。
    fn err_cur<T>(&self, msg: impl Into<String>) -> RvResult<T> {
        let t = self.cur();
        fail_at(t.line, t.col, t.len, msg)
    }
    fn is_punct(&self, s: &str) -> bool {
        self.cur().k == Tk::Punct && self.cur().s == s
    }
    fn is_ident(&self, s: &str) -> bool {
        self.cur().k == Tk::Ident && self.cur().s == s
    }
    fn expect_punct(&mut self, s: &str) -> RvResult<()> {
        if !self.is_punct(s) {
            return self.err_cur(format!("expected '{}'", s));
        }
        self.i += 1;
        Ok(())
    }
    fn expect_ident(&mut self, what: &str) -> RvResult<String> {
        if self.cur().k != Tk::Ident {
            return self.err_cur(format!("expected {}", what));
        }
        let r = self.cur().s.clone();
        self.i += 1;
        Ok(r)
    }

    /// reg / wire / ポートの宣言名が素子名(`d` / `r` / `cd` 等)と衝突しないか検査する。
    /// 衝突する名前はチェーン内で素子列と区別できず曖昧になるためエラーにする。
    fn check_decl_name(&self, name: &str, kind: &str, line: i32) -> RvResult<()> {
        if crate::interp::name_collides_with_element(name) {
            return fail(
                line,
                format!(
                    "{} name '{}' collides with an element name (e.g. 'd', 'r', 'cd'); \
                     pick a name that is not an element sequence",
                    kind, name
                ),
            );
        }
        Ok(())
    }

    // ---- directives --------------------------------------------------------

    fn parse_directive(&mut self, prog: &mut Program) -> RvResult<()> {
        let ln = self.cur().line;
        let d = self.expect_ident("directive name after '#'")?;
        if d == "define" {
            let name = self.expect_ident("define name")?;
            // 値は定数式(リテラル・他の数値 define / param ・`+ - * / %`・単項・括弧)。
            let e = self.parse_expr()?;
            let v = self.eval_const(&e)?;
            self.consts.insert(name.clone(), v);
            prog.defines.insert(name, v);
        } else if d == "include" {
            let fn_;
            match self.cur().k {
                Tk::Str => {
                    fn_ = self.cur().s.clone();
                    self.i += 1;
                }
                // 引用符なしの Ident(`#include stdlogic`)は受理しない。識別子の
                // 字句規則に合う名前しか書けず、書ける名前の境界を仕様が背負うことに
                // なるため、1.0 で仕様面を最小にする方針に合わせ Str に限定した(issue #111)。
                Tk::Ident => {
                    return fail(
                        ln,
                        format!(
                            "#include expects a quoted file name: #include \"{}\"",
                            self.cur().s
                        ),
                    )
                }
                _ => return fail(ln, "#include expects a quoted file name"),
            }
            // バンドル済み stdlib (`stdlogic` 等) は埋め込みソースから読む。
            // 同じ stdlib を 2 度 include しても 2 度目以降は no-op で重複定義エラーを避ける。
            if let Some(src) = bundled_stdlib(&fn_) {
                if prog.included_stdlibs.insert(fn_.clone()) {
                    let toks = Lexer::new(src.to_string()).run()?;
                    let mut sub = Parser::new(toks, self.base_dir.clone());
                    sub.consts = self.consts.clone();
                    sub.parse_file(prog)?;
                    self.consts.extend(sub.consts);
                }
                return Ok(());
            }
            self.include_file(&fn_, ln, prog)?;
        } else {
            return fail(ln, format!("unknown directive: #{}", d));
        }
        Ok(())
    }

    fn include_file(&mut self, fn_: &str, ln: i32, prog: &mut Program) -> RvResult<()> {
        let cands = [
            format!("{}/{}", self.base_dir, fn_),
            format!("{}/{}.rv", self.base_dir, fn_),
            fn_.to_string(),
            format!("{}.rv", fn_),
        ];
        for c in &cands {
            if let Ok(src) = fs::read_to_string(c) {
                let toks = Lexer::new(src).run()?;
                let mut sub = Parser::new(toks, dir_of(c));
                // 外側で定義済みの param/define を include 先からも参照できるよう引き継ぐ。
                sub.consts = self.consts.clone();
                sub.parse_file(prog)?;
                // include 先で定義された param/define を外側へ戻す。
                self.consts.extend(sub.consts);
                return Ok(());
            }
        }
        fail(ln, format!("cannot open include file: {}", fn_))
    }

    // ---- param -------------------------------------------------------------

    /// `param NAME = <定数式> ;` を解析。値は `self.consts`(パース時解決用)と
    /// `prog.defines`(interp が読む)の両方に登録する。
    fn parse_param(&mut self, prog: &mut Program) -> RvResult<()> {
        let name = self.expect_ident("param name")?;
        self.expect_punct("=")?;
        let e = self.parse_expr()?;
        self.expect_punct(";")?;
        let v = self.eval_const(&e)?;
        self.consts.insert(name.clone(), v);
        prog.defines.insert(name, v);
        Ok(())
    }

    /// 定数式 `e` を `self.consts` を引いて畳み込む。`$time` / 添字 / 比較・論理は不可。
    fn eval_const(&self, e: &Expr) -> RvResult<i64> {
        match e {
            Expr::Num { num, .. } => Ok(*num),
            Expr::Time { line } => fail(*line, "$time is not allowed in a constant expression"),
            Expr::Var { line, name, index } => {
                if index.is_some() {
                    return fail(
                        *line,
                        format!("cannot index '{}' in a constant expression", name),
                    );
                }
                match self.consts.get(name) {
                    Some(v) => Ok(*v),
                    None => fail(
                        *line,
                        format!(
                            "unknown constant '{}' (declare it with 'param {} = ...;')",
                            name, name
                        ),
                    ),
                }
            }
            Expr::Un { line, op, a } => {
                let v = self.eval_const(a)?;
                match op.as_str() {
                    "-" => Ok(-v),
                    "!" => Ok((v == 0) as i64),
                    _ => fail(*line, format!("operator '{}' not allowed in a constant expression", op)),
                }
            }
            Expr::Bin { line, op, a, b } => {
                let a = self.eval_const(a)?;
                let b = self.eval_const(b)?;
                match op.as_str() {
                    "+" => Ok(a + b),
                    "-" => Ok(a - b),
                    "*" => Ok(a * b),
                    "/" => {
                        if b == 0 {
                            return fail(*line, "division by zero in a constant expression");
                        }
                        Ok(a / b)
                    }
                    "%" => {
                        if b == 0 {
                            return fail(*line, "modulo by zero in a constant expression");
                        }
                        Ok(a % b)
                    }
                    _ => fail(
                        *line,
                        format!("operator '{}' not allowed in a constant expression", op),
                    ),
                }
            }
        }
    }

    /// バス幅 `[<定数式>]`(省略可)を解析する。`[` が無ければ `None`。
    /// `what` は診断用のラベル(例: "var[")。`param` / 数値 `#define` のみ参照可で、
    /// 値はパース時に i32 へ即時解決する(`var[N]` 等、ジェネリック幅 param 非対応コンテキスト用)。
    fn parse_width(&mut self, what: &str) -> RvResult<Option<i32>> {
        if !self.is_punct("[") {
            return Ok(None);
        }
        let ln = self.cur().line;
        self.i += 1;
        if self.is_punct("]") {
            return fail(ln, format!("expected a bus width after '{}'", what));
        }
        let e = self.parse_expr()?;
        self.expect_punct("]")?;
        let n = self.eval_const(&e)?;
        if n < 1 {
            return fail(ln, format!("bus width must be >= 1 (got {})", n));
        }
        Ok(Some(n as i32))
    }

    /// 同上、ただし **logic 本体内** の幅式用(`input[W]` / `reg[W+1]` 等)。
    /// 現在の logic に宣言されたジェネリック param(`self.logic_params`)を参照する式は
    /// `WidthExpr::Expr` のまま残し、elaborate 時に呼び出しごとの環境で解決する。
    /// param を参照しない式は `WidthExpr::Lit(n)` へ即時簡約する(従来挙動と一致)。
    fn parse_width_expr(&mut self, what: &str) -> RvResult<Option<WidthExpr>> {
        if !self.is_punct("[") {
            return Ok(None);
        }
        let ln = self.cur().line;
        self.i += 1;
        if self.is_punct("]") {
            return fail(ln, format!("expected a bus width after '{}'", what));
        }
        let e = self.parse_expr()?;
        self.expect_punct("]")?;
        // logic param を含まなければ即時簡約(回帰しないように Lit に倒す)。
        if self.logic_params.is_empty() || !Self::expr_refs_logic_param(&e, &self.logic_params) {
            let n = self.eval_const(&e)?;
            if n < 1 {
                return fail(ln, format!("bus width must be >= 1 (got {})", n));
            }
            return Ok(Some(WidthExpr::Lit(n as i32)));
        }
        Ok(Some(WidthExpr::Expr(e)))
    }

    /// 式 `e` が `params` 内の名前を参照しているか(再帰)。
    fn expr_refs_logic_param(e: &Expr, params: &HashSet<String>) -> bool {
        match e {
            Expr::Num { .. } | Expr::Time { .. } => false,
            Expr::Var { name, index, .. } => {
                if params.contains(name) {
                    return true;
                }
                if let Some(ix) = index {
                    Self::expr_refs_logic_param(ix, params)
                } else {
                    false
                }
            }
            Expr::Un { a, .. } => Self::expr_refs_logic_param(a, params),
            Expr::Bin { a, b, .. } => {
                Self::expr_refs_logic_param(a, params) || Self::expr_refs_logic_param(b, params)
            }
        }
    }

    /// `(t1, t2, ...)` — 多出力束縛 / インスタンス化の **target タプル**を解析。
    /// 文頭 `(` の直後から `)` までを消費する。各 target は名前 + 任意のレーン / スライス添字
    /// (`sum[0]` / `sum[3:1]`、issue #118)または連結 `{c, s[3:0]}`(issue #123)で、
    /// `.side` は不可。字面が同じ参照の重複はここでエラー(解決後レーンの重複は interp が
    /// 検査)。空タプル `()` もエラー。
    /// `kind` は診断用ラベル(例: "logic instance output (reg/port name)")。
    fn parse_target_tuple(&mut self, kind: &str) -> RvResult<Vec<BindTarget>> {
        let ln = self.cur().line;
        self.expect_punct("(")?;
        if self.is_punct(")") {
            return fail(
                ln,
                "empty target tuple '()' is not allowed in a logic-instance binding",
            );
        }
        let mut out: Vec<BindTarget> = Vec::new();
        loop {
            let name_ln = self.cur().line;
            let t = if self.is_punct("{") {
                BindTarget::Concat(self.parse_concat_refs()?)
            } else {
                let name = self.expect_ident(kind)?;
                let sel = self.parse_sel()?;
                if self.is_punct(".") {
                    return fail(
                        name_ln,
                        format!(
                            "'.side' is not a valid logic-output binding target ('{}')",
                            name
                        ),
                    );
                }
                BindTarget::Ref(BindRef { name, sel })
            };
            // 字面重複の早期検査(リテラル添字のみ。式添字や部分重複は interp が
            // 解決後レーンで検査する)。連結は要素単位で比較し、連結内部の重複
            // (`{a, a}`)も同時に拾う。
            for (ri, r) in t.refs().iter().enumerate() {
                if let Some(key) = Self::target_key(&r.name, &r.sel) {
                    let dup_in_self = t.refs()[..ri].iter().any(|pr| {
                        Self::target_key(&pr.name, &pr.sel).as_deref() == Some(key.as_str())
                    });
                    let dup_in_prev = out.iter().any(|prev| {
                        prev.refs().iter().any(|pr| {
                            Self::target_key(&pr.name, &pr.sel).as_deref() == Some(key.as_str())
                        })
                    });
                    if dup_in_self || dup_in_prev {
                        return fail(
                            name_ln,
                            format!("duplicate target '{}' in logic-instance binding tuple", key),
                        );
                    }
                }
            }
            out.push(t);
            if self.is_punct(",") {
                self.i += 1;
                continue;
            }
            break;
        }
        self.expect_punct(")")?;
        Ok(out)
    }

    /// target の字面キー(重複検査用)。添字がリテラルへ確定済みの場合のみ `Some`
    /// (`p` / `p[0]` / `p[3:1]`)。ジェネリック param の遅延式を含む場合は `None`
    /// (パース時に比較できないので interp のレーン重複検査に任せる)。
    fn target_key(name: &str, sel: &Sel) -> Option<String> {
        match sel {
            Sel::All => Some(name.to_string()),
            Sel::Lane(IdxExpr::Lit(k)) => Some(format!("{}[{}]", name, k)),
            Sel::Slice(IdxExpr::Lit(h), IdxExpr::Lit(l)) => Some(format!("{}[{}:{}]", name, h, l)),
            _ => None,
        }
    }

    /// `callee #(P=v, ...) (args...)` を 1 つ消費する。`;` は呼び出し側で消費する。
    /// 引数は名前(レーン / スライス添字つき可)またはネスト呼び出し。
    /// 返り値は (callee 名, `#(...)` 実引数, 引数列)。
    fn parse_callee_invocation(&mut self) -> RvResult<CalleeInvocation> {
        let callee = self.expect_ident("logic name")?;
        let call_params = self.parse_logic_call_params()?;
        self.expect_punct("(")?;
        let args = self.parse_inst_args()?;
        self.expect_punct(")")?;
        Ok((callee, call_params, args))
    }

    /// 連結 `{e1, e2, ...}` を単純参照(名前 + レーン選択)の列として読む
    /// (logic 呼び出しの引数 / 束縛 target 用、issue #123)。文頭の `{` から `}` まで消費する。
    /// 要素の制約はチェーン端点の連結(§6.3)と同じ: `.side`・ネスト連結は不可。
    /// 加えて引数 / target 位置固有のネスト呼び出しも不可。
    fn parse_concat_refs(&mut self) -> RvResult<Vec<BindRef>> {
        let ln = self.cur().line;
        self.expect_punct("{")?;
        if self.is_punct("}") {
            return fail(ln, "empty concatenation '{}' is not allowed");
        }
        let mut parts = Vec::new();
        loop {
            if self.is_punct("{") {
                return fail(self.cur().line, "nested concatenation '{...}' is not allowed");
            }
            let name = self.expect_ident("concatenation element (reg/port/var name)")?;
            if self.is_punct("(") || self.is_punct("#") {
                return fail(
                    self.cur().line,
                    "a nested logic call cannot appear inside a concatenation '{...}'",
                );
            }
            let sel = self.parse_sel()?;
            if self.is_punct(".") {
                return fail(
                    self.cur().line,
                    "'.side' cannot appear inside a concatenation '{...}'",
                );
            }
            parts.push(BindRef { name, sel });
            if self.is_punct(",") {
                self.i += 1;
                continue;
            }
            break;
        }
        self.expect_punct("}")?;
        Ok(parts)
    }

    /// logic 呼び出しの引数列(`(` の直後から `)` の手前まで)を消費する。
    /// 各引数は名前(レーン `x[k]` / スライス `x[hi:lo]` 添字つき可、issue #118。
    /// 添字は §6.3.1 の定数式)か、連結 `{a, b[2]}`(issue #123)か、
    /// `h(x)` / `h#(W=4)(x)` 形の **ネスト呼び出し**(issue #97。
    /// 引数位置の `ident (` / `ident #` / `ident [` / `{` は従来エラーだったトークン列への純加算)。
    fn parse_inst_args(&mut self) -> RvResult<Vec<InstArg>> {
        let mut args = Vec::new();
        if !self.is_punct(")") {
            loop {
                let ln = self.cur().line;
                if self.is_punct("{") {
                    args.push(InstArg::Concat(self.parse_concat_refs()?));
                    if self.is_punct(",") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
                let an = self.expect_ident("logic input (reg/port/var name)")?;
                if self.is_punct("(") || self.is_punct("#") {
                    let params = self.parse_logic_call_params()?;
                    self.expect_punct("(")?;
                    let sub = self.parse_inst_args()?;
                    self.expect_punct(")")?;
                    args.push(InstArg::Call {
                        line: ln,
                        callee: an,
                        params,
                        args: sub,
                    });
                } else {
                    let sel = self.parse_sel()?;
                    args.push(InstArg::Name { name: an, sel });
                }
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
        }
        Ok(args)
    }

    /// `#( P=v, Q=w, ... )` — ジェネリック幅 param の **実引数列** を解析。
    /// 識別子直後の `#` を消費して `(...)` を読む。`#(` が来ていなければ空の Vec。
    /// 値式はそのまま AST に残し、elaborate 時に評価する(`self.consts` を流す)。
    fn parse_logic_call_params(&mut self) -> RvResult<Vec<(String, Expr)>> {
        if !self.is_punct("#") {
            return Ok(Vec::new());
        }
        self.i += 1;
        self.expect_punct("(")?;
        let mut out: Vec<(String, Expr)> = Vec::new();
        if !self.is_punct(")") {
            loop {
                let ln = self.cur().line;
                let name = self.expect_ident("logic parameter name")?;
                if out.iter().any(|(n, _)| n == &name) {
                    return fail(ln, format!("duplicate logic parameter '{}' in '#(...)'", name));
                }
                self.expect_punct("=")?;
                let e = self.parse_expr()?;
                out.push((name, e));
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(out)
    }

    // ---- logic -------------------------------------------------------------

    fn parse_logic(&mut self, prog: &mut Program) -> RvResult<()> {
        let line = self.cur().line;
        let name = self.expect_ident("logic name")?;
        if prog.logics.contains_key(&name) {
            return fail(line, format!("duplicate logic definition: {}", name));
        }
        // ジェネリック幅 param 宣言 `#(W=4, X)`(Phase 2)。logic 名直後・ポート列の前にだけ書ける。
        // 各 param は名前と省略可能な既定値(`= <定数式>`、`self.consts` で即時評価)。
        let mut params: Vec<(String, Option<i64>)> = Vec::new();
        let mut local_params: HashSet<String> = HashSet::new();
        if self.is_punct("#") {
            self.i += 1;
            self.expect_punct("(")?;
            if !self.is_punct(")") {
                loop {
                    let ln = self.cur().line;
                    let pname = self.expect_ident("logic parameter name")?;
                    if local_params.contains(&pname) {
                        return fail(
                            ln,
                            format!("duplicate logic parameter '{}' in '#(...)'", pname),
                        );
                    }
                    let default = if self.is_punct("=") {
                        self.i += 1;
                        let e = self.parse_expr()?;
                        Some(self.eval_const(&e)?)
                    } else {
                        None
                    };
                    local_params.insert(pname.clone());
                    params.push((pname, default));
                    if self.is_punct(",") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
            }
            self.expect_punct(")")?;
        }
        // ポート列・本体を `self.logic_params` のスコープでパースする(本体の `[expr]` が
        // ローカル param を参照したとき遅延式へ落とすため)。終端で必ず clear する。
        self.logic_params = local_params;
        let result = self.parse_logic_body(line, name, params, prog);
        self.logic_params.clear();
        result
    }

    fn parse_logic_body(
        &mut self,
        line: i32,
        name: String,
        params: Vec<(String, Option<i64>)>,
        prog: &mut Program,
    ) -> RvResult<()> {
        let mut ports = Vec::new();
        self.expect_punct("(")?;
        if !self.is_punct(")") {
            loop {
                let pl = self.cur().line;
                let pk = self.expect_ident("'input' or 'output'")?;
                if pk != "input" && pk != "output" {
                    return fail(pl, "port must start with 'input' or 'output'");
                }
                // バスポート `input[N]` / `output[N]`(`[N]` は input/output と名前の間)。
                // 幅はリテラル・`param` 定数・定数式・logic ローカル param(Phase 2)を許可。
                let width = self.parse_width_expr(&format!("{}[", pk))?;
                let pname = self.expect_ident("port name")?;
                self.check_decl_name(&pname, "port", pl)?;
                ports.push(Port {
                    input: pk == "input",
                    name: pname,
                    line: pl,
                    width,
                });
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
        }
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        while !self.is_punct("}") {
            self.parse_logic_stmt(&mut stmts)?;
        }
        self.expect_punct("}")?;
        prog.logics.insert(
            name.clone(),
            LogicDef {
                name,
                line,
                ports,
                stmts,
                params,
            },
        );
        Ok(())
    }

    fn parse_logic_stmt(&mut self, stmts: &mut Vec<LogicStmt>) -> RvResult<()> {
        let ln = self.cur().line;
        // 多出力インスタンス化: `(o1, o2, ...) = callee#(...)(args...);`
        // 文頭 `(` は従来エラーだったので純加算。1 個でも `(o) = callee(...)` の形を許す。
        if self.is_punct("(") {
            let outputs = self.parse_target_tuple("logic instance output (reg/port name)")?;
            self.expect_punct("=")?;
            let (callee, call_params, args) = self.parse_callee_invocation()?;
            self.expect_punct(";")?;
            stmts.push(LogicStmt::Instance {
                line: ln,
                outputs,
                callee,
                args,
                params: call_params,
            });
            return Ok(());
        }
        if self.is_ident("wire") {
            self.i += 1;
            let mut names = vec![self.expect_ident("wire name")?];
            while self.is_punct(",") {
                self.i += 1;
                names.push(self.expect_ident("wire name")?);
            }
            for n in &names {
                self.check_decl_name(n, "wire", ln)?;
            }
            self.expect_punct(";")?;
            stmts.push(LogicStmt::DeclWire { line: ln, names });
            return Ok(());
        }

        let mut qual = Qual::Plain;
        if self.is_ident("const") {
            qual = Qual::Const;
            self.i += 1;
        } else if self.is_ident("mutable")
            && self.peek(1).k == Tk::Ident
            && self.peek(1).s == "reg"
        {
            // `mutable` 修飾子は plain と等価だったため廃止(issue #142)。
            // `mutable reg` の並びに限って移行先を案内する。単独の `mutable` は
            // 通常の識別子(reg/wire 名やチェーン端点)として扱う。
            return fail(
                ln,
                "'mutable' was removed; a plain reg behaves the same (write 'reg n = d;')",
            );
        }

        if self.is_ident("reg") {
            self.i += 1;
            // バス幅 `reg[N]`(`[N]` は reg と名前の間に置き、宣言列全体に適用)。
            // 幅はリテラル・`param` 定数・定数式・logic ローカル param(Phase 2)を許可。
            let width = self.parse_width_expr("reg[")?;
            loop {
                let name = self.expect_ident("reg name")?;
                self.check_decl_name(&name, "reg", ln)?;
                let mut init = None;
                if self.is_punct("=") {
                    // バス reg の初期化子はコンパレータ / リピーターの素子代入
                    // (`reg[W] m = r;` 等)に限る。妥当性判定は interp が行う(issue #95)。
                    self.i += 1;
                    let mut strength = -1;
                    if self.cur().k == Tk::Num {
                        strength = self.cur().num as i32;
                        self.i += 1;
                    }
                    // 数値の後に素子トークンが続かなければ裸数値初期化
                    // (`const reg n = 15;`)。数値なしなら素子トークン必須。
                    let tok = if strength >= 0 && self.cur().k != Tk::Ident {
                        None
                    } else {
                        Some(self.expect_ident("element")?)
                    };
                    init = Some(RegInit { strength, tok });
                }
                if width.is_some() && qual != Qual::Plain {
                    return fail(ln, "a bus reg must be plain (const not supported yet)");
                }
                stmts.push(LogicStmt::DeclReg {
                    line: ln,
                    name,
                    qual,
                    init,
                    width: width.clone(),
                });
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
            self.expect_punct(";")?;
            return Ok(());
        }

        if qual != Qual::Plain {
            return fail(ln, "'const' must be followed by 'reg'");
        }

        // 先頭の端点を読む(スカラ / バス全体 / レーン / スライス / 連結)。
        // 次が '-' ならチェーン接続文、'=' なら代入/インスタンス。
        let head = self.parse_endpoint()?;
        if self.is_punct("-") {
            // チェーン接続文:  from -chunks...- to
            let mut parts = vec![head];
            while self.is_punct("-") {
                self.i += 1;
                parts.push(self.parse_endpoint()?);
            }
            self.expect_punct(";")?;
            let n = parts.len();
            // 中間チャンク(素子列 / wire 名)は裸の名前のみ。
            // レーン/スライス添字・`.side`・連結 `{...}` は端点 2 つにしか付けられない。
            let mut chunks = Vec::with_capacity(n.saturating_sub(2));
            for ep in &parts[1..n - 1] {
                match endpoint_bare_name(ep) {
                    Some(name) => chunks.push(name),
                    None => {
                        return fail(
                            ln,
                            "a lane/slice index, '.side', or concatenation '{...}' cannot appear \
                             on a mid-chain chunk (only the two endpoints can)",
                        )
                    }
                }
            }
            let from = parts.first().unwrap().clone();
            let to = parts.last().unwrap().clone();
            stmts.push(LogicStmt::Chain {
                line: ln,
                from,
                to,
                chunks,
            });
            return Ok(());
        }
        // チェーンでない: head は代入 / インスタンス化の target。レーン / スライス添字と
        // 連結 `{...}`(issue #123)は logic 呼び出しの束縛 target(issue #118)として
        // のみ許し、素子代入では不可。
        let bind_target = match head {
            Endpoint::Ref { name, side, sel } => {
                if side {
                    return fail(ln, "'.side' is only valid as a chain endpoint");
                }
                BindTarget::Ref(BindRef { name, sel })
            }
            Endpoint::Concat(elems) => {
                // 連結要素は parse_ref(false) 経由なので常に `.side` なしの Ref。
                let parts = elems
                    .into_iter()
                    .filter_map(|e| match e {
                        Endpoint::Ref { name, sel, .. } => Some(BindRef { name, sel }),
                        Endpoint::Concat(_) => None,
                    })
                    .collect();
                BindTarget::Concat(parts)
            }
        };
        self.expect_punct("=")?;
        // 階層インスタンス化(1 出力):  out = callee#(P=v, ...)(args...)
        // タプル形 `(o1, o2) = callee(...)` は文頭 `(` 分岐で別に扱う(多出力)。
        // ジェネリック幅 param は `#(...)` で実引数を渡す(Phase 2)。`#(` 部分は省略可。
        if self.cur().k == Tk::Ident
            && self.peek(1).k == Tk::Punct
            && (self.peek(1).s == "(" || self.peek(1).s == "#")
        {
            let (callee, call_params, args) = self.parse_callee_invocation()?;
            self.expect_punct(";")?;
            stmts.push(LogicStmt::Instance {
                line: ln,
                outputs: vec![bind_target],
                callee,
                args,
                params: call_params,
            });
            return Ok(());
        }
        // 以降は素子代入 / wire 素子列定義。target は裸の名前のみ(添字・連結は不可)。
        let (tname, tsel) = match bind_target {
            BindTarget::Ref(r) => (r.name, r.sel),
            BindTarget::Concat(_) => {
                return fail(
                    ln,
                    "a concatenation '{...}' target must bind a logic call \
                     (e.g. '{c, s} = g(x);')",
                )
            }
        };
        let target = match tsel {
            Sel::All => tname,
            Sel::Lane(_) => {
                return fail(
                    ln,
                    "cannot assign to a bus lane '[..]'; drive it with a chain \
                     (e.g. 'src - a[0];') or bind a logic output (e.g. 'a[0] = g(x);')",
                )
            }
            Sel::Slice(..) => {
                return fail(
                    ln,
                    "cannot assign to a bus slice '[..:..]'; drive it with a chain \
                     (e.g. 'src - a[3:0];') or bind a logic output (e.g. 'a[3:0] = g(x);')",
                )
            }
        };
        if self.cur().k == Tk::Num {
            let strength = self.cur().num as i32;
            self.i += 1;
            let rhs = self.expect_ident("element after signal strength")?;
            stmts.push(LogicStmt::AssignSingle {
                line: ln,
                target,
                strength,
                rhs,
            });
        } else {
            // 各パートは「素子チャンク or 端点」。端点には `.side` が付き得る。
            // `=` の右辺は wire 素子列定義 / エイリアスで、バス(レーン/スライス/連結)は不可。
            let mut parts: Vec<(String, bool, Sel)> =
                vec![endpoint_simple(self.parse_endpoint()?, ln)?];
            while self.is_punct("-") {
                self.i += 1;
                parts.push(endpoint_simple(self.parse_endpoint()?, ln)?);
            }
            // 添字/スライスは代入(wire 素子列定義 / エイリアス)では使えない(Phase 1a)。
            for (tok, _side, sel) in &parts {
                if !matches!(sel, Sel::All) {
                    return fail(
                        ln,
                        format!(
                            "a lane index '[..]' is not allowed on the right-hand side of '='; \
                             use a chain to wire a bus lane (e.g. '{}[..] - dst;')",
                            tok
                        ),
                    );
                }
            }
            if parts.len() == 1 {
                let (rhs, side, _sel) = parts.into_iter().next().unwrap();
                if side {
                    return fail(ln, "'.side' is only valid as a wire endpoint");
                }
                stmts.push(LogicStmt::AssignSingle {
                    line: ln,
                    target,
                    strength: -1,
                    rhs,
                });
            } else {
                // 中間チャンク(素子列)に `.side` は付けられない
                for (tok, side, _sel) in &parts[1..parts.len() - 1] {
                    if *side {
                        return fail(
                            ln,
                            format!("'.side' cannot appear on a mid-wire element chunk '{}'", tok),
                        );
                    }
                }
                let (from, from_side, _) = parts.first().unwrap().clone();
                let (to, to_side, _) = parts.last().unwrap().clone();
                let chunks: Vec<String> = parts[1..parts.len() - 1]
                    .iter()
                    .map(|(t, _, _)| t.clone())
                    .collect();
                stmts.push(LogicStmt::AssignChain {
                    line: ln,
                    target,
                    from,
                    from_side,
                    to,
                    to_side,
                    chunks,
                });
            }
        }
        self.expect_punct(";")?;
        Ok(())
    }

    /// チェーンの 1 パート(素子チャンク or 端点)を `Endpoint` として読む。
    /// 端点は素朴な名前・レーン `name[k]`・スライス `name[hi:lo]`・連結 `{...}` のいずれか。
    /// `.side`(コンパレータ/リピーターの横入力端子)も付き得る(連結内は不可)。
    fn parse_endpoint(&mut self) -> RvResult<Endpoint> {
        if self.is_punct("{") {
            // 連結 `{e1, e2, ...}`。各要素は名前/レーン/スライス(`.side`・ネスト不可)。
            let ln = self.cur().line;
            self.i += 1;
            if self.is_punct("}") {
                return fail(ln, "empty concatenation '{}' is not allowed");
            }
            let mut elems = Vec::new();
            loop {
                if self.is_punct("{") {
                    return fail(self.cur().line, "nested concatenation '{...}' is not allowed");
                }
                elems.push(self.parse_ref(false)?);
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
            self.expect_punct("}")?;
            return Ok(Endpoint::Concat(elems));
        }
        self.parse_ref(true)
    }

    /// 名前 + レーン選択(`[k]` / `[hi:lo]`)+ 任意の `.side` を読み、`Endpoint::Ref` を返す。
    /// `allow_side` が false(連結要素)なら `.side` はエラー。添字と `.side` の併用は
    /// バス named point のレーン横入力(`m[k].side`)として受理する(妥当性は interp)。
    fn parse_ref(&mut self, allow_side: bool) -> RvResult<Endpoint> {
        let name = self.expect_ident("element chunk or endpoint")?;
        let sel = self.parse_sel()?;
        let mut side = false;
        if self.is_punct(".") {
            let ln = self.cur().line;
            self.i += 1;
            let suf = self.expect_ident("terminal name after '.'")?;
            if suf != "side" {
                return fail(
                    ln,
                    format!("unknown terminal '.{}' (only '.side' is supported)", suf),
                );
            }
            if !allow_side {
                return fail(ln, "'.side' cannot appear inside a concatenation '{...}'");
            }
            // レーン / スライス添字との併用(`m[k].side` / `m[hi:lo].side`)はバス
            // named point の横入力として合法(issue #95)。妥当性は interp が判定する。
            side = true;
        }
        Ok(Endpoint::Ref { name, side, sel })
    }

    /// バスのレーン選択 `[k]`(単一)/ `[hi:lo]`(スライス、包含)を読む。`[` が無ければ `All`。
    /// 添字は定数式(issue #89)。ジェネリック param を含む式のみ `IdxExpr::Expr` で遅延し、
    /// それ以外はパース時に `IdxExpr::Lit` へ即時解決する(`parse_width_expr` と同じ二相解決)。
    fn parse_sel(&mut self) -> RvResult<Sel> {
        if !self.is_punct("[") {
            return Ok(Sel::All);
        }
        let ln = self.cur().line;
        self.i += 1;
        if self.is_punct("]") {
            return fail(ln, "expected a lane index or slice after '['");
        }
        let a = self.parse_idx_expr(ln)?;
        if self.is_punct(":") {
            self.i += 1;
            let b = self.parse_idx_expr(ln)?;
            self.expect_punct("]")?;
            Ok(Sel::Slice(a, b))
        } else {
            self.expect_punct("]")?;
            Ok(Sel::Lane(a))
        }
    }

    /// レーン / スライス添字の定数式を 1 つ読む。logic param を含まなければ即時簡約。
    /// 範囲検査(バス幅との照合)は elaborate 側で行うため、ここでは値域のみ確認する。
    fn parse_idx_expr(&mut self, ln: i32) -> RvResult<IdxExpr> {
        let e = self.parse_expr()?;
        self.idx_expr_of(e, ln)
    }

    /// 解析済みの式をレーン / スライス添字 `IdxExpr` へ変換する(`parse_idx_expr` の後半。
    /// sim のレーン束縛 target のように、式を読んでから添字と確定する経路と共用する)。
    fn idx_expr_of(&mut self, e: Expr, ln: i32) -> RvResult<IdxExpr> {
        if !self.logic_params.is_empty() && Self::expr_refs_logic_param(&e, &self.logic_params) {
            return Ok(IdxExpr::Expr(e));
        }
        let n = self.eval_const(&e)?;
        if n < i32::MIN as i64 || n > i32::MAX as i64 {
            return fail(ln, format!("lane/slice index out of range: {}", n));
        }
        Ok(IdxExpr::Lit(n as i32))
    }

    // ---- module ------------------------------------------------------------

    fn parse_module(&mut self, prog: &mut Program) -> RvResult<()> {
        let line = self.cur().line;
        let name = self.expect_ident("module name")?;
        // module は引数を取らない。旧記法 `module name() { ... }` は廃止済み(issue #96)。
        if self.is_punct("(") {
            return self.err_cur("module takes no arguments; write `module name { ... }`");
        }
        self.expect_punct("{")?;
        let mut pre = Vec::new();
        let mut sim = Vec::new();
        while !self.is_punct("}") {
            if self.is_ident("var") {
                pre.push(self.parse_var_decl()?);
            } else if self.is_ident("sim") {
                self.i += 1;
                self.expect_punct("{")?;
                while !self.is_punct("}") {
                    sim.push(self.parse_sim_stmt()?);
                }
                self.expect_punct("}")?;
            } else {
                return self.err_cur("expected 'var' or 'sim' in module body");
            }
        }
        self.expect_punct("}")?;
        prog.modules.push(ModuleDef { name, line, pre, sim });
        Ok(())
    }

    fn parse_var_decl(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;
        self.i += 1; // 'var'
        // バス var `var[N]`(`[N]` は var と名前の間。宣言列全体に適用)。
        // 幅はリテラルのほか param 定数や定数式(`var[W]` / `var[W+1]`)を許可。
        let width = self.parse_width("var[")?;
        let mut decls = Vec::new();
        loop {
            let n = self.expect_ident("variable name")?;
            let e = if self.is_punct("=") {
                self.i += 1;
                Some(self.parse_expr()?)
            } else {
                None
            };
            decls.push((n, e, width));
            if self.is_punct(",") {
                self.i += 1;
                continue;
            }
            break;
        }
        self.expect_punct(";")?;
        Ok(SimStmt::DeclVar { line: ln, decls })
    }

    fn parse_block_or_single(&mut self) -> RvResult<Vec<SimStmt>> {
        let mut out = Vec::new();
        if self.is_punct("{") {
            self.i += 1;
            while !self.is_punct("}") {
                out.push(self.parse_sim_stmt()?);
            }
            self.expect_punct("}")?;
        } else {
            out.push(self.parse_sim_stmt()?);
        }
        Ok(out)
    }

    fn parse_assign_no_semi(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;
        let target = self.expect_ident("assignment target")?;
        let index = if self.is_punct("[") {
            self.i += 1;
            let e = self.parse_expr()?;
            self.expect_punct("]")?;
            Some(Box::new(e))
        } else {
            None
        };
        self.expect_punct("=")?;
        let value = self.parse_expr()?;
        Ok(SimStmt::Assign {
            line: ln,
            target,
            index,
            value,
            pulse: None,
        })
    }

    fn parse_sim_stmt(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;

        if self.is_punct("#") {
            self.i += 1;
            let s = if self.cur().k == Tk::Num {
                let ticks = self.cur().num;
                self.i += 1;
                SimStmt::WaitTicks { line: ln, ticks }
            } else if self.is_ident("init") {
                self.i += 1;
                SimStmt::WaitInit { line: ln }
            } else if self.is_ident("until") {
                self.i += 1;
                self.expect_punct("(")?;
                let cond = self.parse_expr()?;
                self.expect_punct(")")?;
                SimStmt::WaitUntil { line: ln, cond }
            } else {
                return self.err_cur("expected '#<ticks>', '#init', or '#until(cond)'");
            };
            if self.is_punct(";") {
                self.i += 1; // optional ';'
            }
            return Ok(s);
        }

        if self.is_punct("?") {
            self.i += 1;
            let call = self.parse_call()?;
            if call.callee != "monitor" {
                return fail(ln, "'?' prefix is only supported for monitor()");
            }
            return Ok(SimStmt::MonReg { line: ln, call });
        }

        if self.is_ident("var") {
            return self.parse_var_decl();
        }
        if self.is_ident("sim") {
            return self.err_cur("nested sim block is not allowed");
        }

        // 多出力束縛: `(t1, t2, ...) = callee#(...)(args...);`
        // 文頭 `(` は従来エラーだったので純加算。1 個でも `(t) = callee(...)` の形を許す。
        if self.is_punct("(") {
            let targets = self.parse_target_tuple("variable name (logic outputs bind to vars)")?;
            self.expect_punct("=")?;
            let (callee, call_params, bind_args) = self.parse_callee_invocation()?;
            self.expect_punct(";")?;
            return Ok(SimStmt::CallBind {
                line: ln,
                targets,
                callee,
                bind_args,
                params: call_params,
                fmt: None,
            });
        }

        // 連結 target の 1 出力束縛: `{c, s[3:0]} = callee#(...)(args...);`(issue #123)。
        // 文頭 `{` は従来エラーだったので純加算。
        if self.is_punct("{") {
            let parts = self.parse_concat_refs()?;
            self.expect_punct("=")?;
            return self.parse_call_bind_rest(ln, BindTarget::Concat(parts));
        }

        if self.is_ident("if") {
            self.i += 1;
            self.expect_punct("(")?;
            let cond = self.parse_expr()?;
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            let else_body = if self.is_ident("else") {
                self.i += 1;
                self.parse_block_or_single()?
            } else {
                Vec::new()
            };
            return Ok(SimStmt::If {
                line: ln,
                cond,
                body,
                else_body,
            });
        }

        if self.is_ident("while") {
            self.i += 1;
            self.expect_punct("(")?;
            let cond = self.parse_expr()?;
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            return Ok(SimStmt::While {
                line: ln,
                cond,
                body,
            });
        }

        if self.is_ident("for") {
            self.i += 1;
            self.expect_punct("(")?;
            let init = if !self.is_punct(";") {
                Some(Box::new(self.parse_assign_no_semi()?))
            } else {
                None
            };
            self.expect_punct(";")?;
            let cond = if !self.is_punct(";") {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect_punct(";")?;
            let post = if !self.is_punct(")") {
                Some(Box::new(self.parse_assign_no_semi()?))
            } else {
                None
            };
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            return Ok(SimStmt::For {
                line: ln,
                init,
                cond,
                post,
                body,
            });
        }

        if self.cur().k == Tk::Ident {
            let name = self.cur().s.clone();
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "(" {
                let call = self.parse_call()?;
                return Ok(SimStmt::Call { line: ln, call });
            }
            // バス var のレーン代入 `name[idx] = value [~ pulse];`、または
            // レーン / スライスへの logic 束縛 `name[k] = g(...);` / `name[hi:lo] = g(...);`
            // (issue #118。従来はどちらも構文エラーだったトークン列への純加算)。
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "[" {
                self.i += 1; // name
                self.expect_punct("[")?;
                let idx = self.parse_expr()?;
                // スライスは logic 束縛 target 専用(§6.3 は端点 / 束縛 target のみ)。
                if self.is_punct(":") {
                    self.i += 1;
                    let lo = self.parse_expr()?;
                    self.expect_punct("]")?;
                    self.expect_punct("=")?;
                    if !self.at_callee_invocation() {
                        return fail(
                            ln,
                            "a bus slice '[..:..]' can only be the target of a logic call \
                             binding (e.g. 'y[3:0] = g(x);')",
                        );
                    }
                    let sel = Sel::Slice(self.idx_expr_of(idx, ln)?, self.idx_expr_of(lo, ln)?);
                    return self.parse_call_bind_rest(ln, BindTarget::Ref(BindRef { name, sel }));
                }
                self.expect_punct("]")?;
                self.expect_punct("=")?;
                // レーンへの logic 束縛(issue #118)。束縛は静的なので添字は定数式に限る
                // (実行時 var 添字が使える通常のレーン代入と違う点)。
                if self.at_callee_invocation() {
                    let sel = Sel::Lane(self.idx_expr_of(idx, ln)?);
                    return self.parse_call_bind_rest(ln, BindTarget::Ref(BindRef { name, sel }));
                }
                let value = self.parse_expr()?;
                let pulse = if self.is_punct("~") {
                    self.i += 1;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_punct(";")?;
                return Ok(SimStmt::Assign {
                    line: ln,
                    target: name,
                    index: Some(Box::new(idx)),
                    value,
                    pulse,
                });
            }
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "=" {
                self.i += 2;
                // CallBind(1 出力):  target = callee#(P=v, ...)(args...)
                // タプル形 `(t1, t2) = callee(...)` は文頭 `(` 分岐で別に扱う(多出力)。
                // ジェネリック幅 param は `#(...)` で実引数を渡す(Phase 2)。`#(` は省略可。
                if self.at_callee_invocation() {
                    return self.parse_call_bind_rest(
                        ln,
                        BindTarget::Ref(BindRef {
                            name,
                            sel: Sel::All,
                        }),
                    );
                }
                // plain Assign(`~ width` を付けるとパルス代入)
                let value = self.parse_expr()?;
                let pulse = if self.is_punct("~") {
                    self.i += 1;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_punct(";")?;
                return Ok(SimStmt::Assign {
                    line: ln,
                    target: name,
                    index: None,
                    value,
                    pulse,
                });
            }
        }

        fail(ln, "unexpected token in sim block")
    }

    /// 現在位置が logic 呼び出し `callee (` / `callee #(...)` の先頭か(`=` の直後で使う)。
    fn at_callee_invocation(&self) -> bool {
        self.cur().k == Tk::Ident
            && self.peek(1).k == Tk::Punct
            && (self.peek(1).s == "(" || self.peek(1).s == "#")
    }

    /// sim の 1-target CallBind の残り(callee 以降)を解析する。`=` まで消費済みで、
    /// 現在位置は callee の ident。`scan("%x")` の書式文字列もここで受理する。
    fn parse_call_bind_rest(&mut self, ln: i32, target: BindTarget) -> RvResult<SimStmt> {
        let callee = self.cur().s.clone();
        self.i += 1; // skip callee
        let call_params = self.parse_logic_call_params()?; // `#(...)` があれば消費
        self.expect_punct("(")?;
        let mut bind_args = Vec::new();
        // `scan("%x")` のような書式文字列を受理する(scan のみ。
        // 他の callee は ident / ネスト呼び出しの引数列として読む)。
        let mut scan_fmt: Option<String> = None;
        if callee == "scan" && self.cur().k == Tk::Str {
            scan_fmt = Some(self.cur().s.clone());
            self.i += 1;
        } else {
            bind_args = self.parse_inst_args()?;
        }
        self.expect_punct(")")?;
        self.expect_punct(";")?;
        Ok(SimStmt::CallBind {
            line: ln,
            targets: vec![target],
            callee,
            bind_args,
            params: call_params,
            fmt: scan_fmt,
        })
    }

    /// callee が現在位置にある呼び出し `name ( ... ) ;` を解析。末尾 ';' まで消費。
    fn parse_call(&mut self) -> RvResult<CallData> {
        let callee = self.expect_ident("function name")?;
        let mut has_fmt = false;
        let mut fmt = String::new();
        let mut args = Vec::new();
        self.expect_punct("(")?;
        if !self.is_punct(")") {
            if self.cur().k == Tk::Str {
                has_fmt = true;
                fmt = self.cur().s.clone();
                self.i += 1;
                if self.is_punct(",") {
                    self.i += 1;
                } else if !self.is_punct(")") {
                    return self.err_cur(
                        "expected ',' or ')' after format string",
                    );
                }
            }
            if !self.is_punct(")") {
                loop {
                    args.push(self.parse_expr()?);
                    if self.is_punct(",") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        self.expect_punct(";")?;
        Ok(CallData {
            callee,
            has_fmt,
            fmt,
            args,
        })
    }

    // ---- expressions -------------------------------------------------------

    fn mk_bin(op: &str, a: Expr, b: Expr, ln: i32) -> Expr {
        Expr::Bin {
            line: ln,
            op: op.to_string(),
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn parse_expr(&mut self) -> RvResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_and()?;
        while self.is_punct("||") {
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin("||", a, self.parse_and()?, ln);
        }
        Ok(a)
    }
    fn parse_and(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_eq()?;
        while self.is_punct("&&") {
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin("&&", a, self.parse_eq()?, ln);
        }
        Ok(a)
    }
    fn parse_eq(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_rel()?;
        while self.is_punct("==") || self.is_punct("!=") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_rel()?, ln);
        }
        Ok(a)
    }
    fn parse_rel(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_add()?;
        while self.is_punct("<") || self.is_punct("<=") || self.is_punct(">") || self.is_punct(">=")
        {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_add()?, ln);
        }
        Ok(a)
    }
    fn parse_add(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_mul()?;
        while self.is_punct("+") || self.is_punct("-") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_mul()?, ln);
        }
        Ok(a)
    }
    fn parse_mul(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_unary()?;
        while self.is_punct("*") || self.is_punct("/") || self.is_punct("%") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_unary()?, ln);
        }
        Ok(a)
    }
    fn parse_unary(&mut self) -> RvResult<Expr> {
        if self.is_punct("-") || self.is_punct("!") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            return Ok(Expr::Un {
                line: ln,
                op,
                a: Box::new(self.parse_unary()?),
            });
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> RvResult<Expr> {
        let ln = self.cur().line;
        if self.cur().k == Tk::Num {
            let num = self.cur().num;
            self.i += 1;
            return Ok(Expr::Num { line: ln, num });
        }
        if self.is_punct("$") {
            self.i += 1;
            let n = self.expect_ident("system variable name after '$'")?;
            if n != "time" {
                return fail(ln, format!("unknown system variable: ${}", n));
            }
            return Ok(Expr::Time { line: ln });
        }
        if self.cur().k == Tk::Ident {
            let name = self.cur().s.clone();
            self.i += 1;
            // バス var のレーン参照 `name[expr]`(添字は実行時に評価)。
            let index = if self.is_punct("[") {
                self.i += 1;
                let e = self.parse_expr()?;
                self.expect_punct("]")?;
                Some(Box::new(e))
            } else {
                None
            };
            return Ok(Expr::Var {
                line: ln,
                name,
                index,
            });
        }
        if self.is_punct("(") {
            self.i += 1;
            let e = self.parse_expr()?;
            self.expect_punct(")")?;
            return Ok(e);
        }
        self.err_cur("expected expression")
    }
}
