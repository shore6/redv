//! redv - elaboration & module/sim interpreter
//!
//! logic 定義を回路グラフへエラボレートし、module の sim を実行する。
//! `?monitor` は sim 開始時にホイストされ、各ウェイト完了直後に発火する(Verilog $monitor 風)。
//!
//! 借用検査の都合上、`insts` / `out_bind` を走査しつつ回路を書き換える箇所は、
//! 必要な値を一旦ローカルへ集めてから適用する。

use crate::ast::*;
use crate::circuit::{Circuit, Config, NodeKind, ObsMode, SeqKind, Vcd};
use crate::diag::{fail, is_json_mode, json_escape_into, lint, warn, RvResult};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::time::{Duration, Instant};

// ---- chain token expansion ---------------------------------------------

/// チェーンの中間チャンク列を Elem 列へ展開する。
/// 各トークンは「素子チャンク(`d4ccd4` 等)」または「wire 名(再利用素子列)」。
/// wire 名は `wire_seq` を引いて再帰展開する(`visited` で循環を検出)。
/// reg / ポート名は中間に置けない(端点専用)ためエラー。
/// `used` には展開中に参照した wire 名を積む(`visited` は push/pop で消えるため
/// 別持ち)。未使用 wire の lint(issue #48)に使う。
#[allow(clippy::too_many_arguments)]
fn expand_chain_tokens(
    tokens: &[String],
    wire_seq: &HashMap<String, (i32, Vec<String>)>,
    scope: &HashMap<String, usize>,
    wire_names: &HashSet<String>,
    bus_names: &HashSet<String>,
    line: i32,
    visited: &mut Vec<String>,
    used: &mut HashSet<String>,
    out: &mut Vec<Elem>,
) -> RvResult<()> {
    for tok in tokens {
        if scope.contains_key(tok) || bus_names.contains(tok) {
            return fail(
                line,
                format!(
                    "named node '{}' cannot appear inside a chain (endpoints go at the two ends)",
                    tok
                ),
            );
        }
        if wire_names.contains(tok) {
            if visited.iter().any(|v| v == tok) {
                return fail(
                    line,
                    format!("recursive wire definition involving '{}'", tok),
                );
            }
            let seq = match wire_seq.get(tok) {
                Some((_def_line, s)) => s,
                None => {
                    return fail(
                        line,
                        format!("wire '{}' is used but never assigned an element sequence", tok),
                    )
                }
            };
            used.insert(tok.clone());
            visited.push(tok.clone());
            expand_chain_tokens(
                seq, wire_seq, scope, wire_names, bus_names, line, visited, used, out,
            )?;
            visited.pop();
        } else {
            out.extend(parse_chunk(tok, line)?);
        }
    }
    Ok(())
}

// ---- element chunk parsing: "ddr2brdccbr4d3" -> element list -----------

pub fn parse_chunk(s: &str, line: i32) -> RvResult<Vec<Elem>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'd' {
            i += 1;
            if i < b.len() && b[i].is_ascii_digit() {
                let mut n = 0i32;
                while i < b.len() && b[i].is_ascii_digit() {
                    n = n * 10 + (b[i] - b'0') as i32;
                    i += 1;
                }
                if n <= 0 {
                    return fail(line, "dust count must be >= 1");
                }
                for _ in 0..n {
                    out.push(Elem { k: 'd', n: 1, line });
                }
            } else {
                out.push(Elem { k: 'd', n: 1, line });
            }
        } else if c == b'r' {
            i += 1;
            let mut n = 1i32;
            if i < b.len() && b[i].is_ascii_digit() {
                n = (b[i] - b'0') as i32;
                i += 1;
                // `r0` = 0tick リピータ(同一 tick で再増幅する組合せ素子)。`r1`-`r4` は遅延つき。
                if !(0..=4).contains(&n) || (i < b.len() && b[i].is_ascii_digit()) {
                    return fail(line, format!("repeater delay must be 0-4 in \"{}\"", s));
                }
            }
            out.push(Elem { k: 'r', n, line });
        } else if c == b't' {
            out.push(Elem { k: 't', n: 1, line });
            i += 1;
        } else if c == b'o' {
            // オブザーバ(変化検出で 1tick パルス)。インラインチェーン専用。
            // 接尾字でエッジ判定モードを選ぶ(issue #58): `o` = 変化全部 /
            // `op` = 立ち上がり / `on` = 立ち下がり / `oe` = 2値エッジ。
            // `n` にモード(0=変化全部, 1=立ち上がり, 2=立ち下がり, 3=2値エッジ)を
            // 載せる(消費は build_chain_body。既存素子文字 c/d/o/r/t は
            // 「オブザーバ+素子」の既存解釈があるため接尾字に使えない)。
            i += 1;
            let n = match b.get(i) {
                Some(b'p') => {
                    i += 1;
                    1
                }
                Some(b'n') => {
                    i += 1;
                    2
                }
                Some(b'e') => {
                    i += 1;
                    3
                }
                _ => 0,
            };
            out.push(Elem { k: 'o', n, line });
        } else if c == b'c' {
            if i + 1 >= b.len() {
                return fail(
                    line,
                    "comparator must be written 'cc' (compare) or 'cd' (subtract)",
                );
            }
            let m = b[i + 1];
            if m == b'c' {
                out.push(Elem { k: 'C', n: 1, line });
            } else if m == b'd' {
                out.push(Elem { k: 'S', n: 1, line });
            } else {
                return fail(line, format!("comparator must be written 'cc' or 'cd' in \"{}\"", s));
            }
            i += 2;
        } else {
            return fail(line, format!("unknown element '{}' in \"{}\"", c as char, s));
        }
    }
    Ok(out)
}

// ---- width expression resolution (Phase 2) -----------------------------

/// `WidthExpr` を実 i32 へ解決する。`param_env` は logic ローカルのジェネリック param
/// 環境(`#(W=8)` で渡された実引数 + 既定値)、`defines` はトップレベル `param` / 数値
/// `#define`(`Program::defines`)。値は 1 以上でなければエラー。
fn resolve_width(
    we: &WidthExpr,
    param_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<i32> {
    let v = match we {
        WidthExpr::Lit(n) => *n as i64,
        WidthExpr::Expr(e) => eval_const_expr(e, param_env, defines, line)?,
    };
    if v < 1 {
        return fail(line, format!("bus width must be >= 1 (got {})", v));
    }
    if v > i32::MAX as i64 {
        return fail(line, format!("bus width too large: {}", v));
    }
    Ok(v as i32)
}

/// 幅式 / `#(P=expr)` 実引数の評価。`params` を最優先で引き、未マッチなら `defines`
/// にフォールバック。`$time` / 添字 / 比較・論理は不可(`param` の `eval_const` 流儀)。
fn eval_const_expr(
    e: &Expr,
    params: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    fallback_line: i32,
) -> RvResult<i64> {
    match e {
        Expr::Num { num, .. } => Ok(*num),
        Expr::Time { line } => fail(*line, "$time is not allowed in a width / param expression"),
        Expr::Var { line, name, index } => {
            if index.is_some() {
                return fail(
                    *line,
                    format!("cannot index '{}' in a width / param expression", name),
                );
            }
            if let Some(v) = params.get(name) {
                return Ok(*v);
            }
            if let Some(v) = defines.get(name) {
                return Ok(*v);
            }
            let ln = if *line == 0 { fallback_line } else { *line };
            fail(
                ln,
                format!(
                    "unknown identifier '{}' in width / param expression (no such logic parameter, \
                     param constant, or numeric #define)",
                    name
                ),
            )
        }
        Expr::Un { line, op, a } => {
            let v = eval_const_expr(a, params, defines, fallback_line)?;
            match op.as_str() {
                "-" => Ok(-v),
                "!" => Ok((v == 0) as i64),
                _ => fail(
                    *line,
                    format!("operator '{}' not allowed in a width / param expression", op),
                ),
            }
        }
        Expr::Bin { line, op, a, b } => {
            let a = eval_const_expr(a, params, defines, fallback_line)?;
            let b = eval_const_expr(b, params, defines, fallback_line)?;
            match op.as_str() {
                "+" => Ok(a + b),
                "-" => Ok(a - b),
                "*" => Ok(a * b),
                "/" => {
                    if b == 0 {
                        return fail(*line, "division by zero in width / param expression");
                    }
                    Ok(a / b)
                }
                "%" => {
                    if b == 0 {
                        return fail(*line, "modulo by zero in width / param expression");
                    }
                    Ok(a % b)
                }
                _ => fail(
                    *line,
                    format!("operator '{}' not allowed in a width / param expression", op),
                ),
            }
        }
    }
}

/// 呼び出し側の `#(P=expr, ...)` 実引数と、callee の `LogicDef.params`(宣言+既定値)を
/// 突き合わせて、callee の `param_env` を構築する。
/// - 実引数の名前が callee 宣言に無ければエラー
/// - 既定値も実引数も無い param はエラー
/// - 実引数の式は `caller_env` + `defines` で評価する(親 logic ローカル param も参照可)
fn build_callee_param_env(
    callee_name: &str,
    decl_params: &[(String, Option<i64>)],
    actual: &[(String, Expr)],
    caller_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<HashMap<String, i64>> {
    for (an, _) in actual {
        if !decl_params.iter().any(|(dn, _)| dn == an) {
            return fail(
                line,
                format!("logic '{}' has no parameter '{}'", callee_name, an),
            );
        }
    }
    let mut env: HashMap<String, i64> = HashMap::new();
    for (pn, default) in decl_params {
        if let Some((_, ae)) = actual.iter().find(|(n, _)| n == pn) {
            let v = eval_const_expr(ae, caller_env, defines, line)?;
            env.insert(pn.clone(), v);
        } else if let Some(d) = default {
            env.insert(pn.clone(), *d);
        } else {
            return fail(
                line,
                format!(
                    "logic '{}' requires parameter '{}' (no default; pass it with '#({}=...)' at the call site)",
                    callee_name, pn, pn,
                ),
            );
        }
    }
    Ok(env)
}

/// インスタンスキャッシュキー用に `param_env` を正規化文字列へ。
/// 並び順は `decl_params` 宣言順とする(`#(W=8,X=2)` と `#(X=2,W=8)` は同一インスタンス)。
fn param_env_key(decl_params: &[(String, Option<i64>)], env: &HashMap<String, i64>) -> String {
    if decl_params.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    for (pn, _) in decl_params {
        if let Some(v) = env.get(pn) {
            parts.push(format!("{}={}", pn, v));
        }
    }
    format!("#({})", parts.join(","))
}

/// 名前全体が素子列として解釈できる(= 素子名と衝突する)なら true。
/// reg / wire / ポート名がこれに該当するのは禁止する: チェーン内に置いたとき
/// 「名前付きの点」と「素子列」が曖昧になり、回路が読みにくくバグの温床になる。
/// 例: `r`(リピータ)/ `cd`(コンパレータ)/ `td`(トーチ+ダスト)。
pub fn name_collides_with_element(name: &str) -> bool {
    !name.is_empty() && parse_chunk(name, 0).is_ok()
}

/// reg 初期化子トークンがコンパレータ(`cc`/`cd`)ちょうど 1 個なら、その
/// SeqKind を返す。コンパレータでなければ None。複合チャンクはエラー。
fn comparator_mode(tok: &str, line: i32) -> RvResult<Option<SeqKind>> {
    let es = parse_chunk(tok, line)?;
    match es.first() {
        Some(e) if e.k == 'C' || e.k == 'S' => {
            if es.len() != 1 {
                return fail(
                    line,
                    format!("a comparator reg must hold exactly one comparator: \"{}\"", tok),
                );
            }
            Ok(Some(if e.k == 'S' {
                SeqKind::CompSub
            } else {
                SeqKind::CompCmp
            }))
        }
        _ => Ok(None),
    }
}

/// reg 初期化子トークンがリピーター(`r` / `r1`-`r4`)ちょうど 1 個なら、その
/// 遅延 tick を返す。リピーターでなければ None。複合チャンクはエラー。
fn repeater_delay(tok: &str, line: i32) -> RvResult<Option<i32>> {
    let es = parse_chunk(tok, line)?;
    match es.first() {
        Some(e) if e.k == 'r' => {
            if es.len() != 1 {
                return fail(
                    line,
                    format!("a repeater reg must hold exactly one repeater: \"{}\"", tok),
                );
            }
            Ok(Some(e.n))
        }
        _ => Ok(None),
    }
}

/// レーン / スライス添字 `IdxExpr` を実 i32 へ解決する(issue #89)。ジェネリック param を
/// 含む遅延式はここ(インスタンス化時)で初めて評価される。バス幅との範囲照合は
/// 呼び出し側(`bus_lane` / `bus_slice`)が行う。
fn resolve_idx(
    ie: &IdxExpr,
    param_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<i32> {
    let v = match ie {
        IdxExpr::Lit(n) => return Ok(*n),
        IdxExpr::Expr(e) => eval_const_expr(e, param_env, defines, line)?,
    };
    if v < i32::MIN as i64 || v > i32::MAX as i64 {
        return fail(line, format!("lane/slice index out of range: {}", v));
    }
    Ok(v as i32)
}

// ---- chain endpoint resolution -----------------------------------------

/// チェーン端点の解決結果。スカラ点(1 ノード)かバス(レーン列)。
/// スライス・連結は常に `Bus`(レーン列)。スカラ点 / 単一レーン `a[k]` は `Single`。
enum Ep {
    Single(usize),
    Bus(Vec<usize>),
}

impl Ep {
    /// レーンノード列(スカラは長さ 1)。
    fn lanes(&self) -> Vec<usize> {
        match self {
            Ep::Single(n) => vec![*n],
            Ep::Bus(v) => v.clone(),
        }
    }
}

/// バスのレーン `name[k]` を解決する(範囲検査つき)。
fn bus_lane(v: &[usize], name: &str, k: i32, line: i32) -> RvResult<Ep> {
    if k < 0 || (k as usize) >= v.len() {
        return fail(
            line,
            format!("bus index out of range: {}[{}] (width {})", name, k, v.len()),
        );
    }
    Ok(Ep::Single(v[k as usize]))
}

/// バスのスライス `name[hi:lo]`(包含)をレーン列へ展開する。`hi >= lo` で降順
/// `[hi, hi-1, .., lo]`、`hi < lo` で昇順 `[hi, hi+1, .., lo]`(ビット反転に使える)。
fn bus_slice(v: &[usize], name: &str, hi: i32, lo: i32, line: i32) -> RvResult<Vec<usize>> {
    let w = v.len() as i32;
    for k in [hi, lo] {
        if k < 0 || k >= w {
            return fail(
                line,
                format!(
                    "bus slice index out of range: {}[{}:{}] (width {})",
                    name, hi, lo, w
                ),
            );
        }
    }
    let mut out = Vec::new();
    if hi >= lo {
        for k in (lo..=hi).rev() {
            out.push(v[k as usize]);
        }
    } else {
        for k in hi..=lo {
            out.push(v[k as usize]);
        }
    }
    Ok(out)
}

/// 幅 `w` のバスに対するレーン選択をレーン番号列へ解決する(sim 側の束縛用、issue #118)。
/// `All` は昇順 `[0..w)`、レーン / スライスは §6.3 の並び順(範囲検査つき)。
/// sim には logic ローカル param が無いので添字は `defines` だけで解決できる。
fn sel_lane_indices(
    sel: &Sel,
    w: usize,
    name: &str,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<Vec<usize>> {
    let all: Vec<usize> = (0..w).collect();
    let no_params: HashMap<String, i64> = HashMap::new();
    match sel {
        Sel::All => Ok(all),
        Sel::Lane(ie) => {
            let k = resolve_idx(ie, &no_params, defines, line)?;
            Ok(bus_lane(&all, name, k, line)?.lanes())
        }
        Sel::Slice(hie, loe) => {
            let hi = resolve_idx(hie, &no_params, defines, line)?;
            let lo = resolve_idx(loe, &no_params, defines, line)?;
            bus_slice(&all, name, hi, lo, line)
        }
    }
}

/// sim 側インスタンスキーの引数 1 個分。添字を含めて正規化する(issue #118:
/// `g(x[0])` と `g(x[1])` は別インスタンス、同じ添字の 2 度目は共有)。
/// `x[k:k]` は `x[k]` と等価(§6.3)なのでレーン形へ正規化する。
fn sim_arg_key(
    name: &str,
    sel: &Sel,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<String> {
    let no_params: HashMap<String, i64> = HashMap::new();
    Ok(match sel {
        Sel::All => name.to_string(),
        Sel::Lane(ie) => {
            format!("{}[{}]", name, resolve_idx(ie, &no_params, defines, line)?)
        }
        Sel::Slice(hie, loe) => {
            let hi = resolve_idx(hie, &no_params, defines, line)?;
            let lo = resolve_idx(loe, &no_params, defines, line)?;
            if hi == lo {
                format!("{}[{}]", name, hi)
            } else {
                format!("{}[{}:{}]", name, hi, lo)
            }
        }
    })
}

/// 単一の名前参照(`name` / `name[k]` / `name[hi:lo]` / `name.side`)を解決する。
/// `dst` が true なら終端(信号先)としての規則: `.side` はコンパレータ/リピーター reg の
/// 横入力、無印の同 reg は後ろ入力。`dst` が false なら始端(信号源)で `.side` は不可。
/// バス named point(`reg[W] m = r;` 等、issue #95)はレーン選択つきの `.side` /
/// back / out をレーン列へ展開する。
/// 添字はここで `param_env` / `defines` 下の定数式として評価する(issue #89)。
#[allow(clippy::too_many_arguments)]
fn resolve_ref(
    name: &str,
    sel: &Sel,
    side: bool,
    dst: bool,
    scope: &HashMap<String, usize>,
    side_regs: &HashMap<String, (usize, usize)>,
    bus_side_regs: &HashMap<String, (Vec<usize>, Vec<usize>)>,
    buses: &HashMap<String, Vec<usize>>,
    wire_names: &HashSet<String>,
    param_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<Ep> {
    if side {
        if !dst {
            return fail(
                line,
                format!("'{}.side' cannot be a wire source (side is an input terminal)", name),
            );
        }
        if let Some((_back, s)) = side_regs.get(name) {
            // スカラ named point の横入力。添字は付けられない。
            return match sel {
                Sel::All => Ok(Ep::Single(*s)),
                _ => fail(
                    line,
                    format!("'{}' is a scalar comparator/repeater reg and cannot be indexed", name),
                ),
            };
        }
        if let Some((_backs, sides)) = bus_side_regs.get(name) {
            // バス named point の横入力: 全体 / レーン / スライスをレーン列で解決。
            return match sel {
                Sel::All => Ok(Ep::Bus(sides.clone())),
                Sel::Lane(ie) => {
                    let k = resolve_idx(ie, param_env, defines, line)?;
                    bus_lane(sides, name, k, line)
                }
                Sel::Slice(hie, loe) => {
                    let hi = resolve_idx(hie, param_env, defines, line)?;
                    let lo = resolve_idx(loe, param_env, defines, line)?;
                    Ok(Ep::Bus(bus_slice(sides, name, hi, lo, line)?))
                }
            };
        }
        return fail(
            line,
            format!(
                "'.side' is only valid on a comparator/repeater reg, but '{}' is not",
                name
            ),
        );
    }
    match sel {
        Sel::Lane(ie) => {
            let k = resolve_idx(ie, param_env, defines, line)?;
            // 終端のバス named point はレーンの後ろ入力(back)。
            if dst {
                if let Some((backs, _sides)) = bus_side_regs.get(name) {
                    return bus_lane(backs, name, k, line);
                }
            }
            match buses.get(name) {
                Some(v) => bus_lane(v, name, k, line),
                None => {
                    fail(line, format!("'{}' is indexed with '[{}]' but is not a bus", name, k))
                }
            }
        }
        Sel::Slice(hie, loe) => {
            let hi = resolve_idx(hie, param_env, defines, line)?;
            let lo = resolve_idx(loe, param_env, defines, line)?;
            if dst {
                if let Some((backs, _sides)) = bus_side_regs.get(name) {
                    return Ok(Ep::Bus(bus_slice(backs, name, hi, lo, line)?));
                }
            }
            match buses.get(name) {
                Some(v) => Ok(Ep::Bus(bus_slice(v, name, hi, lo, line)?)),
                None => fail(
                    line,
                    format!("'{}' is sliced with '[{}:{}]' but is not a bus", name, hi, lo),
                ),
            }
        }
        Sel::All => {
            // 終端では無印のコンパレータ/リピーター reg は後ろ入力(back)。
            if dst {
                if let Some((back, _side)) = side_regs.get(name) {
                    return Ok(Ep::Single(*back));
                }
                if let Some((backs, _sides)) = bus_side_regs.get(name) {
                    return Ok(Ep::Bus(backs.clone()));
                }
            }
            if let Some(n) = scope.get(name) {
                return Ok(Ep::Single(*n));
            }
            if let Some(v) = buses.get(name) {
                return Ok(Ep::Bus(v.clone()));
            }
            if wire_names.contains(name) {
                return fail(
                    line,
                    format!(
                        "wire '{}' cannot be a chain endpoint (a wire is an element sequence; \
                         endpoints must be reg/port)",
                        name
                    ),
                );
            }
            fail(line, format!("unknown chain endpoint: {}", name))
        }
    }
}

/// チェーン端点 `Endpoint` をレーン列へ解決する。連結は各要素のレーン列を順に連接する
/// (結果は常に `Bus`)。`dst` は始端/終端の別(`.side`・back/out の解釈に使う)。
#[allow(clippy::too_many_arguments)]
fn resolve_endpoint(
    ep: &Endpoint,
    dst: bool,
    scope: &HashMap<String, usize>,
    side_regs: &HashMap<String, (usize, usize)>,
    bus_side_regs: &HashMap<String, (Vec<usize>, Vec<usize>)>,
    buses: &HashMap<String, Vec<usize>>,
    wire_names: &HashSet<String>,
    param_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<Ep> {
    match ep {
        Endpoint::Ref { name, side, sel } => resolve_ref(
            name, sel, *side, dst, scope, side_regs, bus_side_regs, buses, wire_names, param_env,
            defines, line,
        ),
        Endpoint::Concat(elems) => {
            let mut lanes = Vec::new();
            for e in elems {
                lanes.extend(
                    resolve_endpoint(
                        e, dst, scope, side_regs, bus_side_regs, buses, wire_names, param_env,
                        defines, line,
                    )?
                    .lanes(),
                );
            }
            Ok(Ep::Bus(lanes))
        }
    }
}

/// レーン選択を診断メッセージ用の添字文字列に整形する(`` / `[3]` / `[3:0]`)。
fn sel_suffix(sel: &Sel) -> String {
    match sel {
        Sel::All => String::new(),
        Sel::Lane(ie) => format!("[{}]", idx_desc(ie)),
        Sel::Slice(hi, lo) => format!("[{}:{}]", idx_desc(hi), idx_desc(lo)),
    }
}

/// 単純参照を診断メッセージ用の文字列に整形する(`p` / `sum[0]` / `sum[3:1]`)。
fn bind_ref_desc(r: &BindRef) -> String {
    format!("{}{}", r.name, sel_suffix(&r.sel))
}

/// 連結を診断メッセージ用の文字列に整形する(`{c, s[3:0]}`)。
fn concat_desc(parts: &[BindRef]) -> String {
    let inner: Vec<String> = parts.iter().map(bind_ref_desc).collect();
    format!("{{{}}}", inner.join(", "))
}

/// 束縛 target を診断メッセージ用の文字列に整形する(`p` / `sum[0]` / `{c, s[3:0]}`)。
fn bind_target_desc(t: &BindTarget) -> String {
    match t {
        BindTarget::Ref(r) => bind_ref_desc(r),
        BindTarget::Concat(parts) => concat_desc(parts),
    }
}

/// 端点を診断メッセージ用の文字列に整形する(`p` / `p[3:0]` / `{a, b}` 等)。
fn endpoint_desc(ep: &Endpoint) -> String {
    match ep {
        Endpoint::Ref { name, side, sel } => {
            let mut s = format!("{}{}", name, sel_suffix(sel));
            if *side {
                s.push_str(".side");
            }
            s
        }
        Endpoint::Concat(elems) => {
            let inner: Vec<String> = elems.iter().map(endpoint_desc).collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// 添字 `IdxExpr` を診断メッセージ用に整形する(`3` / `(W - 1)` 等)。
/// 遅延式は評価前でも AST から字面を再構成して示す。
fn idx_desc(ie: &IdxExpr) -> String {
    match ie {
        IdxExpr::Lit(n) => n.to_string(),
        IdxExpr::Expr(e) => expr_to_string(e),
    }
}

// ---- logic instance ----------------------------------------------------

/// ポートの形。スカラ(1 ノード)かバス(レーン列)。バスポートは内部バス reg(§4.2)と
/// 同じく本体で添字 / バス全体として使える。
#[derive(Debug, Clone)]
enum PortShape {
    Scalar(usize),
    Bus(Vec<usize>),
}

impl PortShape {
    /// レーンノード列(スカラは長さ 1)。
    fn lanes(&self) -> Vec<usize> {
        match self {
            PortShape::Scalar(n) => vec![*n],
            PortShape::Bus(v) => v.clone(),
        }
    }
    fn width(&self) -> usize {
        match self {
            PortShape::Scalar(_) => 1,
            PortShape::Bus(v) => v.len(),
        }
    }
}

/// ポート列 = (ポート名, 形)の並び。`elaborate` の入出力ポート列に使う。
type Ports = Vec<(String, PortShape)>;

/// 保留中の階層インスタンス文(`(o1, o2, ...) = callee#(P=v)(args)`)。
/// 順序: (line, outputs, callee, args, call_params)。`outputs` は出力ポートと位置対応の親側
/// 端点(reg/ポート/バス reg/バスポート。レーン / スライス添字つき可)で、長さは callee の
/// 出力ポート数と一致する必要がある。
/// call_params は `#(...)` 実引数で、callee の `param_env` ビルド時に親 logic の環境下で評価する(Phase 2)。
type PendingInstance = (i32, Vec<BindTarget>, String, Vec<InstArg>, Vec<(String, Expr)>);

/// 階層インスタンスの親側端点(reg/ポート = スカラ、内部バス reg/バスポート = バス)を
/// `PortShape` として解決する。レーン `x[k]` / スライス `x[hi:lo]`(issue #118)は
/// バスの該当レーン列へ切り出す(添字はチェーン端点と同じく `param_env` 下の定数式)。
#[allow(clippy::too_many_arguments)]
fn resolve_parent_ref(
    name: &str,
    sel: &Sel,
    scope: &HashMap<String, usize>,
    buses: &HashMap<String, Vec<usize>>,
    wire_names: &HashSet<String>,
    param_env: &HashMap<String, i64>,
    defines: &HashMap<String, i64>,
    line: i32,
) -> RvResult<PortShape> {
    if wire_names.contains(name) {
        return fail(line, format!("'{}' is a wire, not a reg/port", name));
    }
    match sel {
        Sel::All => {
            if let Some(n) = scope.get(name) {
                return Ok(PortShape::Scalar(*n));
            }
            if let Some(v) = buses.get(name) {
                return Ok(PortShape::Bus(v.clone()));
            }
            fail(line, format!("unknown logic instance endpoint: {}", name))
        }
        Sel::Lane(ie) => {
            let k = resolve_idx(ie, param_env, defines, line)?;
            match buses.get(name) {
                Some(v) => Ok(PortShape::Scalar(bus_lane(v, name, k, line)?.lanes()[0])),
                None => fail(
                    line,
                    format!("'{}' is indexed with '[{}]' but is not a bus", name, k),
                ),
            }
        }
        Sel::Slice(hie, loe) => {
            let hi = resolve_idx(hie, param_env, defines, line)?;
            let lo = resolve_idx(loe, param_env, defines, line)?;
            match buses.get(name) {
                Some(v) => Ok(PortShape::Bus(bus_slice(v, name, hi, lo, line)?)),
                None => fail(
                    line,
                    format!("'{}' is sliced with '[{}:{}]' but is not a bus", name, hi, lo),
                ),
            }
        }
    }
}

/// 2 つのポート(`src` -> `dst`)をレーン対応で減衰なし結線する。幅整合は厳格。
fn connect_ports(
    c: &mut Circuit,
    src: &PortShape,
    dst: &PortShape,
    ctx: &str,
    line: i32,
) -> RvResult<()> {
    if src.width() != dst.width() {
        return fail(
            line,
            format!(
                "{}: port width mismatch ({} vs {} lane(s); scalar/bus must match)",
                ctx,
                src.width(),
                dst.width()
            ),
        );
    }
    let s = src.lanes();
    let d = dst.lanes();
    for i in 0..s.len() {
        c.add_edge(s[i], d[i], 0);
    }
    Ok(())
}

// ---- 束縛の共通検査(issue #100)------------------------------------------
// logic 呼び出しの束縛は logic 本体側(§4.5 の instances ループ / resolve_inst_arg)と
// sim 側(ensure_instance / do_call_bind)の 2 経路にあるが、callee 解決・出力ポート
// 検査・target 検査は同一セマンティクスなので、以下のヘルパーへ一本化する。
// 片方だけ直して挙動が食い違う事故を防ぐため、これらの検査を経路側へ複製しない。

/// callee 名から logic 定義を引く。
fn lookup_logic<'p>(
    logics: &'p HashMap<String, LogicDef>,
    callee: &str,
    line: i32,
) -> RvResult<&'p LogicDef> {
    match logics.get(callee) {
        Some(l) => Ok(l),
        None => fail(line, format!("unknown logic: {}", callee)),
    }
}

/// 出力ポートの無い logic は束縛できない。
fn require_output_ports(callee: &str, n_out: usize, line: i32) -> RvResult<()> {
    if n_out == 0 {
        return fail(line, format!("{} has no output port to bind", callee));
    }
    Ok(())
}

/// ネスト呼び出しにできるのは単一出力 logic だけ(LANGUAGE.md §5.6)。
fn require_single_output(callee: &str, n_out: usize, line: i32) -> RvResult<()> {
    if n_out != 1 {
        return fail(
            line,
            format!(
                "nested call to {} must have exactly 1 output port (it has {}); \
                 bind it with a tuple first, then pass the target",
                callee, n_out
            ),
        );
    }
    Ok(())
}

/// 出力ポート数と束縛タプルの target 数は厳格一致(過不足ともエラー)。
fn check_binding_arity(callee: &str, n_out: usize, n_targets: usize, line: i32) -> RvResult<()> {
    if n_targets != n_out {
        return fail(
            line,
            format!(
                "{} has {} output port(s) but the binding tuple has {} target(s)",
                callee, n_out, n_targets
            ),
        );
    }
    Ok(())
}

/// 同じ点(レーン)を束縛タプルで 2 回以上束縛するのはエラー(同じ点へ複数出力を
/// ぶつけるのは意図と違う)。字面が違っても解決後のレーンが重なれば検出する
/// (`(sum[1:0], sum[1])` / `(sum, sum[0])` 等、issue #118)。連結 target の内部で
/// 同じレーンが重なる形(`{a, a}` 等、issue #123)も検出する。
/// logic 本体側はノード id、sim 側は var レーンキーで比較する(issue #100 の共通ヘルパー)。
/// `descs` は診断用の target 字面で、`lanes` と位置対応。
fn check_target_overlap<T: PartialEq>(descs: &[String], lanes: &[Vec<T>], line: i32) -> RvResult<()> {
    for i in 0..lanes.len() {
        for a in 0..lanes[i].len() {
            if lanes[i][a + 1..].contains(&lanes[i][a]) {
                return fail(
                    line,
                    format!(
                        "target '{}' in logic-instance binding binds the same lane \
                         more than once",
                        descs[i]
                    ),
                );
            }
        }
        for j in (i + 1)..lanes.len() {
            if lanes[i].iter().any(|a| lanes[j].contains(a)) {
                return fail(
                    line,
                    format!(
                        "targets '{}' and '{}' in logic-instance binding tuple overlap \
                         (the same lane is bound more than once)",
                        descs[i], descs[j]
                    ),
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Instance {
    /// 入力レーンごとの (sim var キー, 回路ノード)。スカラ入力は 1 レーン、
    /// バス入力は幅ぶんのレーン(var キーは `xbus[0]` 等)。
    pub in_vars: Vec<String>,
    pub in_nodes: Vec<usize>,
    /// 出力ポートごとのレーンノード列(順序は callee の宣言順)。スカラ出力は長さ 1、
    /// バス出力は幅ぶんのレーン。多出力 logic は `out_ports.len() > 1` となる。
    /// 束縛先 var は呼び出しごとに `out_bind` へ登録する(複数の var が同じインスタンス
    /// 出力を観測し得るため、target はインスタンス同一性に含めない)。
    pub out_ports: Vec<Vec<usize>>,
}

struct Elaborator<'c, 'p> {
    c: &'c mut Circuit,
    /// 階層インスタンス化の解決に使う logic 定義表
    logics: &'p HashMap<String, LogicDef>,
    /// トップレベル `param` / 数値 `#define`。幅式の解決でフォールバック参照する
    /// (logic ローカル param に無い名前はここを引く)。
    defines: &'p HashMap<String, i64>,
    /// 現在エラボレート中の logic 名(再帰インスタンス化検出用)
    stack: Vec<String>,
    /// サブインスタンスのノード名を一意化する連番
    counter: usize,
    /// 静的 lint(floating-reg 等)を出し終えた logic 名。同じ logic を複数回
    /// インスタンス化しても宣言由来の警告は 1 回で済ませる(issue #48)。
    /// ModuleExec が module 単位で保持し、エラボレーションごとに借用する。
    linted: &'c mut HashSet<String>,
}

impl<'p> Elaborator<'_, 'p> {
    fn apply_elem(
        &mut self,
        node_id: usize,
        name: &str,
        tok: &str,
        strength: i32,
        qual: Qual,
        line: i32,
    ) -> RvResult<()> {
        // 素子トークンつきの強度リテラル(旧 `15b` / `3d`)は廃止した(issue #75)。
        // 固定強度は素子を伴わない裸数値(`const reg n = 15;`)で書く。
        // parse_chunk より先に検査し、旧 `15b` にもこの案内を出す。
        if strength >= 0 {
            return fail(
                line,
                format!(
                    "an element cannot take a signal strength; write a bare number instead \
                     (`const reg {} = {};`)",
                    name, strength
                ),
            );
        }
        let es = parse_chunk(tok, line)?;
        if es.len() != 1 {
            return fail(line, format!("a reg must hold exactly one element: \"{}\"", tok));
        }
        let e = es[0];
        let root = self.c.find(node_id);
        if self.c.nodes[root].elem_assigned {
            if self.c.nodes[root].is_const_qual {
                return fail(line, "cannot reassign element of a const reg");
            }
            warn(line, "reg element reassigned (last assignment wins)");
        }
        // リピータ / コンパレータは reg に格納できるが **宣言時初期化に限る**
        // (`reg m = r;` の形で back/side/out の 3 ノード束を生成する。§ DeclReg)。
        // ここに来るのは AssignSingle 経由(= 後置代入)だけなので、宣言時形へ誘導する。
        if e.k == 'r' || e.k == 'C' || e.k == 'S' {
            let kind = if e.k == 'r' { "repeater" } else { "comparator" };
            return fail(
                line,
                format!(
                    "a {} reg must be initialized at its declaration (write `reg {} = {};`); \
                     it cannot be assigned after declaration",
                    kind, name, tok
                ),
            );
        }
        // トーチは reg に格納できない(順序素子はワイヤー/チェーン内に置く)。
        if e.k == 't' {
            return fail(
                line,
                format!(
                    "element \"{}\" cannot be placed in a reg (a torch belongs inside a wire/chain)",
                    tok
                ),
            );
        }
        // オブザーバも reg に置けない(横端子を持たず、インラインチェーン専用)。
        if e.k == 'o' {
            return fail(
                line,
                format!(
                    "element \"{}\" cannot be placed in a reg (an observer belongs inline in a \
                     wire/chain, e.g. 'x - o - y;')",
                    tok
                ),
            );
        }
        if qual == Qual::Const {
            return fail(
                line,
                format!(
                    "a const reg must be initialized with a bare signal strength \
                     (e.g. `const reg {} = 15;`)",
                    name
                ),
            );
        }
        self.c.nodes[root].kind = NodeKind::Plain;
        self.c.nodes[root].elem_assigned = true;
        Ok(())
    }

    /// 裸数値初期化子(`const reg n = 15;`)を適用する。素子トークンを伴わない
    /// 強度指定は const 専用で、宣言時初期化(DeclReg)からしか到達しない。
    fn apply_bare_strength(
        &mut self,
        node_id: usize,
        strength: i32,
        qual: Qual,
        line: i32,
    ) -> RvResult<()> {
        if qual != Qual::Const {
            return fail(line, "signal-strength literals are only allowed on const reg");
        }
        if strength > 15 {
            return fail(
                line,
                format!("const signal strength out of range 0-15: {}", strength),
            );
        }
        let root = self.c.find(node_id);
        let nd = &mut self.c.nodes[root];
        nd.kind = NodeKind::Const;
        nd.base = strength;
        nd.is_const_qual = true;
        nd.elem_assigned = true;
        Ok(())
    }

    /// 1 本のスカラチェーン `fi -chunks- ti` を回路に構築する。
    /// `label` は内部ノード名の識別子(`#chN` / `#chN_i` で trace 非表示)。
    /// バスチェーンはレーンごとに本関数を呼び、各レーンで独立した素子列を展開する。
    /// `used_wires` には展開で参照した wire 名が積まれる(未使用 wire lint 用)。
    #[allow(clippy::too_many_arguments)]
    fn build_chain_body(
        &mut self,
        label: &str,
        prefix: &str,
        fi: usize,
        ti: usize,
        chunks: &[String],
        wire_seq: &HashMap<String, (i32, Vec<String>)>,
        scope: &HashMap<String, usize>,
        wire_names: &HashSet<String>,
        bus_names: &HashSet<String>,
        used_wires: &mut HashSet<String>,
        line: i32,
    ) -> RvResult<()> {
        let mut es: Vec<Elem> = Vec::new();
        let mut visited: Vec<String> = Vec::new();
        expand_chain_tokens(
            chunks, wire_seq, scope, wire_names, bus_names, line, &mut visited, used_wires,
            &mut es,
        )?;
        let mut prev = fi;
        let mut decay = 0;
        let mut idx = 0;
        for e in &es {
            idx += 1;
            match e.k {
                'd' => decay += 1,
                // 0tick リピータ(`r0`): 遅延ゼロの組合せ増幅器。順序素子ではなく
                // 不動点ループ内で評価する zero_rep として展開する。
                'r' if e.n == 0 => {
                    let ni = self
                        .c
                        .new_node(format!("{}.{}#i{}", prefix, label, idx), NodeKind::Plain);
                    let no = self
                        .c
                        .new_node(format!("{}.{}#o{}", prefix, label, idx), NodeKind::Plain);
                    self.c.add_edge(prev, ni, decay);
                    self.c.add_zero_rep(ni, no);
                    prev = no;
                    decay = 0;
                }
                // 順序素子(リピータ / トーチ / オブザーバ)。`add_seq` の履歴段数
                // (delay)で前 tick 入力を保持する。オブザーバは隣接 2 サンプルの
                // 変化検出なので履歴 2 段(delay=2)で展開する。
                'r' | 't' | 'o' => {
                    let ni = self
                        .c
                        .new_node(format!("{}.{}#i{}", prefix, label, idx), NodeKind::Plain);
                    let no = self
                        .c
                        .new_node(format!("{}.{}#o{}", prefix, label, idx), NodeKind::Plain);
                    self.c.add_edge(prev, ni, decay);
                    let (kind, dly) = match e.k {
                        'r' => (SeqKind::Rep, e.n),
                        't' => (SeqKind::Torch, 1),
                        // オブザーバ: e.n はモード(parse_chunk のエンコードを参照)
                        _ => {
                            let mode = match e.n {
                                1 => ObsMode::Rise,
                                2 => ObsMode::Fall,
                                3 => ObsMode::Edge,
                                _ => ObsMode::Any,
                            };
                            (SeqKind::Observer(mode), 2)
                        }
                    };
                    self.c.add_seq(
                        kind,
                        dly,
                        ni,
                        no,
                        format!("{}.{}[{}]", prefix, label, idx),
                        e.line,
                    );
                    prev = no;
                    decay = 0;
                }
                // インライン(チェーン内)コンパレータ: 横入力なし = パススルー
                'C' | 'S' => {
                    let ni = self
                        .c
                        .new_node(format!("{}.{}#i{}", prefix, label, idx), NodeKind::Plain);
                    let no = self
                        .c
                        .new_node(format!("{}.{}#o{}", prefix, label, idx), NodeKind::Plain);
                    self.c.add_edge(prev, ni, decay);
                    let kind = if e.k == 'S' {
                        SeqKind::CompSub
                    } else {
                        SeqKind::CompCmp
                    };
                    self.c.add_comp(
                        kind,
                        ni,
                        None,
                        no,
                        format!("{}.{}[{}]", prefix, label, idx),
                        e.line,
                    );
                    prev = no;
                    decay = 0;
                }
                _ => {}
            }
        }
        self.c.add_edge(prev, ti, decay);
        Ok(())
    }

    /// 階層インスタンスの引数 1 個を `PortShape` へ解決する(issue #97)。
    ///
    /// - 名前 … 親の reg / ポート / バス(従来どおり `resolve_parent_ref`)。レーン /
    ///   スライス添字(issue #118)は該当レーン列へ切り出される。
    /// - ネスト呼び出し `h(x)` … サブ logic を新規インスタンスとして展開し、引数を
    ///   再帰解決してサブ入力ポートへ減衰なし直結したうえで、**単一出力ポート** の形を
    ///   返す(外側の入力ポートへの直結は呼び出し側が行う)。出力ポートが 1 個で
    ///   なければエラー(多出力はタプル束縛で中間 reg に受けてから渡す)。
    #[allow(clippy::too_many_arguments)]
    fn resolve_inst_arg(
        &mut self,
        arg: &InstArg,
        scope: &HashMap<String, usize>,
        buses: &HashMap<String, Vec<usize>>,
        wire_names: &HashSet<String>,
        param_env: &HashMap<String, i64>,
        prefix: &str,
        line: i32,
    ) -> RvResult<PortShape> {
        match arg {
            InstArg::Name { name: a, sel } => {
                if wire_names.contains(a.as_str()) {
                    return fail(
                        line,
                        format!("logic instance argument '{}' must be a reg/port, not a wire", a),
                    );
                }
                resolve_parent_ref(a, sel, scope, buses, wire_names, param_env, self.defines, line)
            }
            // 連結引数(issue #123): 各要素のレーン列を並び順に連接した
            // 幅 = 要素和のバスとして渡す(チェーン端点の連結と同じ規則)。
            InstArg::Concat(parts) => {
                let mut lanes = Vec::new();
                for r in parts {
                    if wire_names.contains(r.name.as_str()) {
                        return fail(
                            line,
                            format!(
                                "logic instance argument '{}' must be a reg/port, not a wire",
                                r.name
                            ),
                        );
                    }
                    lanes.extend(
                        resolve_parent_ref(
                            &r.name,
                            &r.sel,
                            scope,
                            buses,
                            wire_names,
                            param_env,
                            self.defines,
                            line,
                        )?
                        .lanes(),
                    );
                }
                Ok(PortShape::Bus(lanes))
            }
            InstArg::Call {
                line: cl,
                callee,
                params,
                args,
            } => {
                let sub = lookup_logic(self.logics, callee, *cl)?;
                let (sub_in, sub_out) =
                    self.instantiate_sub(sub, params, param_env, prefix, *cl, args.len())?;
                require_single_output(callee, sub_out.len(), *cl)?;
                // 引数を再帰解決し、サブ logic の入力ポートへ減衰なし直結する。
                for (a, (pname, pshape)) in args.iter().zip(sub_in.iter()) {
                    let ar = self.resolve_inst_arg(
                        a, scope, buses, wire_names, param_env, prefix, *cl,
                    )?;
                    let ctx = format!("{} input port '{}'", callee, pname);
                    connect_ports(self.c, &ar, pshape, &ctx, *cl)?;
                }
                Ok(sub_out.into_iter().next().unwrap().1)
            }
        }
    }

    /// callee を **非表示のサブプレフィックス** `{prefix}/{callee}#{n}` でエラボレートする、
    /// logic 本体側 2 経路(§4.5 の instances ループ / ネスト呼び出し引数)の共通部分。
    /// 親の `param_env` を caller env として callee の `param_env` を構築する。
    fn instantiate_sub(
        &mut self,
        sub: &'p LogicDef,
        call_params: &[(String, Expr)],
        caller_env: &HashMap<String, i64>,
        parent_prefix: &str,
        line: i32,
        arg_count: usize,
    ) -> RvResult<(Ports, Ports)> {
        let sub_env = build_callee_param_env(
            &sub.name,
            &sub.params,
            call_params,
            caller_env,
            self.defines,
            line,
        )?;
        self.counter += 1;
        let sub_prefix = format!("{}/{}#{}", parent_prefix, sub.name, self.counter);
        self.elaborate(sub, &sub_prefix, false, line, arg_count, sub_env)
    }

    /// logic 定義 1 つを回路ノード群へ展開する。
    ///
    /// - `top_level == true` … 入力ポートは sim 変数で駆動する `Input` ノード。
    /// - `top_level == false` … 階層インスタンス。入力ポートは親ノードから駆動される
    ///   `Plain` ノードで、結線は呼び出し側が行う。
    ///
    /// 返り値は (入力ポート列, 出力ポート列)。各ポートは (名前, 形 PortShape)。
    fn elaborate(
        &mut self,
        l: &'p LogicDef,
        prefix: &str,
        top_level: bool,
        call_line: i32,
        arg_count: usize,
        param_env: HashMap<String, i64>,
    ) -> RvResult<(Ports, Ports)> {
        if self.stack.iter().any(|n| n == &l.name) {
            self.stack.push(l.name.clone());
            return fail(
                call_line,
                format!(
                    "recursive logic instantiation: {}",
                    self.stack.join(" -> ")
                ),
            );
        }
        self.stack.push(l.name.clone());

        // lint(issue #48)用の収集。`seq_lo` はこの elaborate(サブインスタンス込み)で
        // 新規生成した順序素子の下端。`decl_lines` は reg / バス reg / wire の宣言行、
        // `used_wires` はチェーン展開で実際に参照された wire 名。
        let seq_lo = self.c.seqs.len();
        let mut decl_lines: HashMap<String, i32> = HashMap::new();
        let mut used_wires: HashSet<String> = HashSet::new();

        let mut scope: HashMap<String, usize> = HashMap::new();
        let mut qual_of: HashMap<String, Qual> = HashMap::new();
        // コンパレータ / ロック付きリピーター reg の (back, side) ノード。scope[name] は out ノード。
        let mut side_regs: HashMap<String, (usize, usize)> = HashMap::new();
        // バス named point(`reg[W] m = r;` 等、issue #95)の (backs, sides) レーン列。
        // out レーン列は buses[name] に置く(始端・引数など out を読む経路を共用するため)。
        let mut bus_side_regs: HashMap<String, (Vec<usize>, Vec<usize>)> = HashMap::new();
        let mut wire_names: HashSet<String> = HashSet::new();
        // バス reg(`reg[n] a;`)/ バスポート。name -> レーンノード列。scope とは別空間。
        let mut buses: HashMap<String, Vec<usize>> = HashMap::new();
        let mut inputs: Ports = Vec::new();
        let mut outs: Ports = Vec::new();
        // 階層インスタンス文(全ノード確定後にまとめて結線する)。
        // 各要素は (line, output, callee, args, call_params)。call_params はジェネリック幅
        // の `#(P=expr)` 実引数で、callee の param_env をビルドする際に評価する(Phase 2)。
        let mut instances: Vec<PendingInstance> = Vec::new();
        // 無名チェーン文(wire 名を介さない直結。全ノード確定後にまとめて構築)。
        // 両端の `Endpoint`(スカラ / バス全体 / レーン / スライス / 連結)を保持する。
        let mut anon_chains: Vec<(i32, Endpoint, Endpoint, Vec<String>)> = Vec::new();

        for p in &l.ports {
            if scope.contains_key(&p.name) || buses.contains_key(&p.name) {
                return fail(p.line, format!("duplicate port name: {}", p.name));
            }
            let kind = if p.input && top_level {
                NodeKind::Input
            } else {
                NodeKind::Plain
            };
            // バスポート(`input[n]` / `output[n]`)は n 本のレーンへ展開し buses に登録。
            // 本体では内部バス reg と同じく添字 / バス全体で使える。
            let shape = if let Some(we) = &p.width {
                let w = resolve_width(we, &param_env, self.defines, p.line)?;
                let mut lanes = Vec::with_capacity(w as usize);
                for i in 0..w {
                    let id = self
                        .c
                        .new_node(format!("{}.{}[{}]", prefix, p.name, i), kind);
                    if !p.input {
                        self.c.nodes[id].is_out_port = true;
                    }
                    lanes.push(id);
                }
                buses.insert(p.name.clone(), lanes.clone());
                PortShape::Bus(lanes)
            } else {
                let id = self.c.new_node(format!("{}.{}", prefix, p.name), kind);
                if !p.input {
                    self.c.nodes[id].is_out_port = true;
                }
                scope.insert(p.name.clone(), id);
                PortShape::Scalar(id)
            };
            if p.input {
                inputs.push((p.name.clone(), shape));
            } else {
                outs.push((p.name.clone(), shape));
            }
        }
        if arg_count != inputs.len() {
            return fail(
                call_line,
                format!(
                    "{}: expected {} input argument(s), got {}",
                    l.name,
                    inputs.len(),
                    arg_count
                ),
            );
        }

        // wire 素子列定義(端点を持たない再利用可能な素子トークン列)。
        // 他 wire 名を含み得るため、Elem への展開はチェーン構築時に遅延する。
        // 再代入は警告し、最後の定義が勝つ。
        let mut wire_seq: HashMap<String, (i32, Vec<String>)> = HashMap::new();
        let mut wseq_seen: HashSet<String> = HashSet::new();

        for st in &l.stmts {
            match st {
                LogicStmt::Instance {
                    line,
                    outputs,
                    callee,
                    args,
                    params,
                } => {
                    instances.push((
                        *line,
                        outputs.clone(),
                        callee.clone(),
                        args.clone(),
                        params.clone(),
                    ));
                }
                LogicStmt::DeclWire { line, names } => {
                    for n in names {
                        if scope.contains_key(n) || wire_names.contains(n) || buses.contains_key(n) {
                            return fail(*line, format!("duplicate name: {}", n));
                        }
                        wire_names.insert(n.clone());
                        decl_lines.insert(n.clone(), *line);
                    }
                }
                LogicStmt::DeclReg {
                    line,
                    name,
                    qual,
                    init,
                    width,
                } => {
                    if scope.contains_key(name) || wire_names.contains(name) || buses.contains_key(name) {
                        return fail(*line, format!("duplicate name: {}", name));
                    }
                    decl_lines.insert(name.clone(), *line);
                    // バス reg(`reg[n] a;`): n 本の plain レーン a[0]..a[n-1] を展開する。
                    // バスは純粋な糖衣で、各レーンは独立したスカラ点(circuit 意味論は不変)。
                    if let Some(we) = width {
                        if *qual != Qual::Plain {
                            return fail(
                                *line,
                                "a bus reg must be plain (const/mutable not supported yet)",
                            );
                        }
                        let w = resolve_width(we, &param_env, self.defines, *line)?;
                        // 素子代入つきバス reg(`reg[W] m = r;` / `cc` / `cd`、issue #95):
                        // レーンごとに back/side/out の 3 ノード束を展開し、バス named point
                        // として登録する(out レーンは buses、(backs, sides) は bus_side_regs)。
                        if let Some(ri) = init {
                            if ri.strength >= 0 {
                                return fail(
                                    *line,
                                    "a bus reg cannot take a signal strength; only a comparator/\
                                     repeater element assignment is allowed (e.g. `reg[4] m = r;`)",
                                );
                            }
                            let tok = ri.tok.as_deref().unwrap(); // strength < 0 なら必ず Some
                            let comp_kind = comparator_mode(tok, *line)?;
                            let rep_delay = if comp_kind.is_none() {
                                repeater_delay(tok, *line)?
                            } else {
                                None
                            };
                            if comp_kind.is_none() && rep_delay.is_none() {
                                return fail(
                                    *line,
                                    format!(
                                        "a bus reg initializer must be a comparator or repeater \
                                         element (cc/cd/r/r1-r4): \"{}\"",
                                        tok
                                    ),
                                );
                            }
                            if rep_delay == Some(0) {
                                return fail(
                                    *line,
                                    format!(
                                        "a 0-tick repeater (r0) cannot be a lockable reg; place it \
                                         inline in a chain (e.g. 'x - r0 - {};')",
                                        name
                                    ),
                                );
                            }
                            let mut backs = Vec::with_capacity(w as usize);
                            let mut sides = Vec::with_capacity(w as usize);
                            let mut outs = Vec::with_capacity(w as usize);
                            for i in 0..w {
                                let back = self.c.new_node(
                                    format!("{}.{}[{}]#back", prefix, name, i),
                                    NodeKind::Plain,
                                );
                                let side = self.c.new_node(
                                    format!("{}.{}[{}]#side", prefix, name, i),
                                    NodeKind::Plain,
                                );
                                let out = self.c.new_node(
                                    format!("{}.{}[{}]", prefix, name, i),
                                    NodeKind::Plain,
                                );
                                let label = format!("{}.{}[{}]", prefix, name, i);
                                if let Some(ck) = comp_kind {
                                    self.c.add_comp(ck, back, Some(side), out, label, *line);
                                } else {
                                    self.c.add_rep_lock(
                                        rep_delay.unwrap(),
                                        back,
                                        side,
                                        out,
                                        label,
                                        *line,
                                    );
                                }
                                backs.push(back);
                                sides.push(side);
                                outs.push(out);
                            }
                            buses.insert(name.clone(), outs);
                            bus_side_regs.insert(name.clone(), (backs, sides));
                            continue;
                        }
                        let mut lanes = Vec::with_capacity(w as usize);
                        for i in 0..w {
                            let id = self
                                .c
                                .new_node(format!("{}.{}[{}]", prefix, name, i), NodeKind::Plain);
                            lanes.push(id);
                        }
                        buses.insert(name.clone(), lanes);
                        continue;
                    }
                    // コンパレータ reg(`reg r = cd;` / `cc`)は back/side/out の 3 ノード束。
                    // 強度つき初期化子(旧 `15b` 形)はコンパレータ / リピーターには
                    // なりえないので判定を飛ばし、apply_elem の廃止案内エラーに任せる。
                    let init_tok = init
                        .as_ref()
                        .filter(|ri| ri.strength < 0)
                        .and_then(|ri| ri.tok.as_deref());
                    let comp_kind = match init_tok {
                        Some(t) => comparator_mode(t, *line)?,
                        None => None,
                    };
                    // ロック付きリピーター reg(`reg m = r;` / `r1`-`r4`)も同じ 3 ノード束。
                    let rep_delay = if comp_kind.is_none() {
                        match init_tok {
                            Some(t) => repeater_delay(t, *line)?,
                            None => None,
                        }
                    } else {
                        None
                    };
                    if let Some(ck) = comp_kind {
                        let ri = init.as_ref().unwrap();
                        if *qual != Qual::Plain {
                            return fail(
                                *line,
                                "comparator reg must be plain (const/mutable not allowed)",
                            );
                        }
                        if ri.strength >= 0 {
                            return fail(*line, "comparator reg cannot take a signal strength");
                        }
                        let back = self
                            .c
                            .new_node(format!("{}.{}#back", prefix, name), NodeKind::Plain);
                        let side = self
                            .c
                            .new_node(format!("{}.{}#side", prefix, name), NodeKind::Plain);
                        let out = self.c.new_node(format!("{}.{}", prefix, name), NodeKind::Plain);
                        self.c.add_comp(
                            ck,
                            back,
                            Some(side),
                            out,
                            format!("{}.{}", prefix, name),
                            *line,
                        );
                        scope.insert(name.clone(), out);
                        side_regs.insert(name.clone(), (back, side));
                        qual_of.insert(name.clone(), Qual::Plain);
                    } else if let Some(dly) = rep_delay {
                        let ri = init.as_ref().unwrap();
                        if *qual != Qual::Plain {
                            return fail(
                                *line,
                                "repeater reg must be plain (const/mutable not allowed)",
                            );
                        }
                        if ri.strength >= 0 {
                            return fail(*line, "repeater reg cannot take a signal strength");
                        }
                        // 0tick リピータ(`r0`)はロック付き reg にできない(ロックは前 tick の
                        // 出力を凍結する順序素子だが、r0 には保持する状態がない)。inline 専用。
                        if dly == 0 {
                            return fail(
                                *line,
                                format!(
                                    "a 0-tick repeater (r0) cannot be a lockable reg; place it inline \
                                     in a chain (e.g. 'x - r0 - {};')",
                                    name
                                ),
                            );
                        }
                        let back = self
                            .c
                            .new_node(format!("{}.{}#back", prefix, name), NodeKind::Plain);
                        let side = self
                            .c
                            .new_node(format!("{}.{}#side", prefix, name), NodeKind::Plain);
                        let out = self.c.new_node(format!("{}.{}", prefix, name), NodeKind::Plain);
                        self.c.add_rep_lock(
                            dly,
                            back,
                            side,
                            out,
                            format!("{}.{}", prefix, name),
                            *line,
                        );
                        scope.insert(name.clone(), out);
                        side_regs.insert(name.clone(), (back, side));
                        qual_of.insert(name.clone(), Qual::Plain);
                    } else {
                        let id = self.c.new_node(format!("{}.{}", prefix, name), NodeKind::Plain);
                        scope.insert(name.clone(), id);
                        qual_of.insert(name.clone(), *qual);
                        match init {
                            Some(ri) => match &ri.tok {
                                Some(t) => {
                                    self.apply_elem(id, name, t, ri.strength, *qual, *line)?
                                }
                                None => self.apply_bare_strength(id, ri.strength, *qual, *line)?,
                            },
                            None => {
                                if *qual == Qual::Const {
                                    return fail(*line, "const reg requires an initializer");
                                }
                            }
                        }
                    }
                }
                LogicStmt::AssignSingle {
                    line,
                    target,
                    strength,
                    rhs,
                } => {
                    if wire_names.contains(target) {
                        // wire への素子列定義(単一トークン)。例: `w = r4;`
                        if *strength >= 0 {
                            return fail(
                                *line,
                                "a wire element sequence cannot take a signal strength",
                            );
                        }
                        if wseq_seen.contains(target) {
                            warn(
                                *line,
                                format!("wire '{}' reassigned (last assignment wins)", target),
                            );
                        }
                        wseq_seen.insert(target.clone());
                        wire_seq.insert(target.clone(), (*line, vec![rhs.clone()]));
                        continue;
                    }
                    if buses.contains_key(target) {
                        return fail(
                            *line,
                            format!(
                                "cannot assign to a whole bus '{}'; drive each lane with a chain \
                                 (e.g. 'src - {}[0];')",
                                target, target
                            ),
                        );
                    }
                    if buses.contains_key(rhs) {
                        return fail(
                            *line,
                            format!(
                                "cannot use a whole bus '{}' on the right-hand side of '='; \
                                 index a lane (e.g. '{}[0]') and wire it with a chain",
                                rhs, rhs
                            ),
                        );
                    }
                    let tnode = match scope.get(target) {
                        Some(n) => *n,
                        None => {
                            return fail(*line, format!("unknown assignment target: {}", target))
                        }
                    };
                    let troot = self.c.find(tnode);
                    if self.c.nodes[troot].kind == NodeKind::Input
                        && !scope.contains_key(rhs)
                        && *strength < 0
                    {
                        return fail(
                            *line,
                            format!("cannot assign an element to input port '{}'", target),
                        );
                    }
                    if *strength < 0 && scope.contains_key(rhs) {
                        self.c.merge(tnode, scope[rhs], *line)?; // alias
                    } else {
                        let q = qual_of.get(target).copied().unwrap_or(Qual::Plain);
                        self.apply_elem(tnode, target, rhs, *strength, q, *line)?;
                    }
                }
                LogicStmt::AssignChain {
                    line,
                    target,
                    from,
                    from_side,
                    to,
                    to_side,
                    chunks,
                } => {
                    // `target = a-b-c;` の `=` 形は wire への素子列定義のみ。
                    // 2 点の接続はチェーン文 `a-b-c;`(`=` なし)で行う。
                    if !wire_names.contains(target) {
                        return fail(
                            *line,
                            format!(
                                "only a wire can be assigned an element sequence ('{} = ...'); \
                                 to connect two points use a chain statement like 'a-b-c;'",
                                target
                            ),
                        );
                    }
                    if *from_side || *to_side {
                        return fail(
                            *line,
                            "'.side' cannot appear in a wire element sequence definition",
                        );
                    }
                    let mut tokens = Vec::with_capacity(chunks.len() + 2);
                    tokens.push(from.clone());
                    tokens.extend(chunks.iter().cloned());
                    tokens.push(to.clone());
                    if wseq_seen.contains(target) {
                        warn(
                            *line,
                            format!("wire '{}' reassigned (last assignment wins)", target),
                        );
                    }
                    wseq_seen.insert(target.clone());
                    wire_seq.insert(target.clone(), (*line, tokens));
                }
                LogicStmt::Chain {
                    line,
                    from,
                    to,
                    chunks,
                } => {
                    anon_chains.push((*line, from.clone(), to.clone(), chunks.clone()));
                }
            }
        }

        // wire 素子列定義の早期検証(未使用 wire も対象)。各トークンは素子チャンク
        // または他 wire 名でなければならない。reg/ポート名(= 旧 named-wire 接続形の
        // 名残)は素子列に置けない。素子展開・循環検出はチェーン構築時に遅延する。
        for (wn, (def_line, tokens)) in &wire_seq {
            for tok in tokens {
                if scope.contains_key(tok) || buses.contains_key(tok) {
                    return fail(
                        *def_line,
                        format!(
                            "named node '{}' cannot appear in wire '{}' (a wire is a reusable \
                             element sequence, not a connection); to connect two points use a \
                             chain statement like 'a-b-c;'",
                            tok, wn
                        ),
                    );
                }
                if !wire_names.contains(tok) {
                    parse_chunk(tok, *def_line)?; // 素子構文の検証
                }
            }
        }

        // チェーン構築(端点はここまでに存在しているはず)。接続は Chain 文のみ
        // (wire は素子列定義であって接続ではない)。バス端点は同幅必須で element-wise に
        // 展開する(各レーンで独立した素子列。circuit シミュレーション意味論は不変)。
        // label は内部ノード名に使う識別子('#chN' / '#chN_i' で trace 非表示)。
        let bus_names: HashSet<String> = buses.keys().cloned().collect();
        for (k, (line, from_ep, to_ep, chunks)) in anon_chains.iter().enumerate() {
            let line = *line;
            // 端点をレーン列へ解決(`.side` を源に置く誤りは resolve_endpoint が弾く)。
            // 添字の遅延式(ジェネリック param 参照)はこの `param_env` 下で評価される。
            let src = resolve_endpoint(
                from_ep, false, &scope, &side_regs, &bus_side_regs, &buses, &wire_names,
                &param_env, self.defines, line,
            )?;
            let dst = resolve_endpoint(
                to_ep, true, &scope, &side_regs, &bus_side_regs, &buses, &wire_names, &param_env,
                self.defines, line,
            )?;
            match (src, dst) {
                (Ep::Single(fi), Ep::Single(ti)) => {
                    let label = format!("#ch{}", k + 1);
                    self.build_chain_body(
                        &label, prefix, fi, ti, chunks, &wire_seq, &scope, &wire_names,
                        &bus_names, &mut used_wires, line,
                    )?;
                }
                (s, d) => {
                    // 少なくとも片方がバス(全体 / スライス / 連結)。幅(レーン数)一致で
                    // element-wise に展開する。スカラは幅 1 として扱う。
                    // 片側が幅 1 なら相手の幅へブロードキャストする(issue #63):
                    //   bus[N] - scalar … N レーンが scalar へ合流(fan-in。circuit の MAX 合流で
                    //                      scalar = max(lanes))。
                    //   scalar - bus[N] … scalar が N レーンを駆動(fan-out)。
                    // いずれも有向エッジなので逆流は起きない。両端が幅 >1 で不一致のときのみエラー。
                    let mut sl = s.lanes();
                    let mut dl = d.lanes();
                    if sl.len() != dl.len() {
                        if sl.len() == 1 {
                            sl = vec![sl[0]; dl.len()];
                        } else if dl.len() == 1 {
                            dl = vec![dl[0]; sl.len()];
                        } else {
                            return fail(
                                line,
                                format!(
                                    "bus width mismatch in chain: '{}' has {} lane(s) but '{}' has {} \
                                     (widths must be equal, or one side must be a scalar to broadcast)",
                                    endpoint_desc(from_ep),
                                    sl.len(),
                                    endpoint_desc(to_ep),
                                    dl.len()
                                ),
                            );
                        }
                    }
                    for i in 0..sl.len() {
                        let label = format!("#ch{}_{}", k + 1, i);
                        self.build_chain_body(
                            &label, prefix, sl[i], dl[i], chunks, &wire_seq, &scope,
                            &wire_names, &bus_names, &mut used_wires, line,
                        )?;
                    }
                }
            }
        }

        // 階層インスタンスの結線(親ノード <-> サブ logic のポート)。
        // スカラ/バスは PortShape として解決し、レーン対応で結線する(幅整合は厳格)。
        // callee 解決・出力ポート検査・target 検査は sim 側と共通のヘルパー(issue #100)。
        for (line, outputs, callee, args, call_params) in &instances {
            let sub = lookup_logic(self.logics, callee, *line)?;
            let mut out_refs: Vec<PortShape> = Vec::with_capacity(outputs.len());
            for output in outputs {
                for r in output.refs() {
                    if wire_names.contains(&r.name) {
                        return fail(
                            *line,
                            format!(
                                "logic instance output '{}' must be a reg/port, not a wire",
                                r.name
                            ),
                        );
                    }
                }
                out_refs.push(match output {
                    BindTarget::Ref(r) => resolve_parent_ref(
                        &r.name,
                        &r.sel,
                        &scope,
                        &buses,
                        &wire_names,
                        &param_env,
                        self.defines,
                        *line,
                    )?,
                    // 連結 target(issue #123): 各要素のレーン列を並び順に連接した
                    // バスとして受ける(チェーン端点の連結と同じ規則)。
                    BindTarget::Concat(parts) => {
                        let mut lanes = Vec::new();
                        for r in parts {
                            lanes.extend(
                                resolve_parent_ref(
                                    &r.name,
                                    &r.sel,
                                    &scope,
                                    &buses,
                                    &wire_names,
                                    &param_env,
                                    self.defines,
                                    *line,
                                )?
                                .lanes(),
                            );
                        }
                        PortShape::Bus(lanes)
                    }
                });
            }
            // 解決後レーンの重複検査(レーン / スライス target の部分重複も検出。issue #118)
            let out_descs: Vec<String> = outputs.iter().map(bind_target_desc).collect();
            let out_lanes: Vec<Vec<usize>> = out_refs.iter().map(|r| r.lanes()).collect();
            check_target_overlap(&out_descs, &out_lanes, *line)?;
            let mut arg_refs: Vec<PortShape> = Vec::new();
            for a in args {
                arg_refs.push(self.resolve_inst_arg(
                    a, &scope, &buses, &wire_names, &param_env, prefix, *line,
                )?);
            }
            // サブ logic を展開(入力ポートは Plain ノードになる)
            let (sub_in, sub_out) =
                self.instantiate_sub(sub, call_params, &param_env, prefix, *line, args.len())?;
            require_output_ports(callee, sub_out.len(), *line)?;
            check_binding_arity(callee, sub_out.len(), outputs.len(), *line)?;
            // 親引数 -> サブ入力ポート(レーン対応で減衰なし直結)
            for (arg_ref, (pname, pshape)) in arg_refs.iter().zip(sub_in.iter()) {
                let ctx = format!("{} input port '{}'", callee, pname);
                connect_ports(self.c, arg_ref, pshape, &ctx, *line)?;
            }
            // 各サブ出力ポート -> 対応する親の出力先(レーン対応)
            for (i, ((out_name, sub_shape), out_ref)) in
                sub_out.iter().zip(out_refs.iter()).enumerate()
            {
                let ctx = format!(
                    "output '{}' of {} bound to '{}'",
                    out_name, callee, out_descs[i]
                );
                connect_ports(self.c, sub_shape, out_ref, &ctx, *line)?;
            }
        }

        // 未接続 output ポート検査(仕様: エラー)。バスポートは全レーンを検査する。
        for (name, shape) in &outs {
            for node in shape.lanes() {
                let r = self.c.find(node);
                if !self.c.nodes[r].has_incoming && self.c.nodes[r].kind != NodeKind::Const {
                    return fail(
                        call_line,
                        format!(
                            "output port '{}' of logic '{}' is not driven (unconnected port)",
                            name, l.name
                        ),
                    );
                }
            }
        }

        // ---- デザインルールチェック(lint、issue #48)---------------------
        // エラー検査を通った回路にだけ警告を出す。宣言由来の静的ルールは logic 名に
        // つき 1 回(複数インスタンス化で重複させない)。
        if self.linted.insert(l.name.clone()) {
            let port_names: HashSet<&str> = l.ports.iter().map(|p| p.name.as_str()).collect();
            // 浮き reg: どこからも駆動されず、どこも駆動しない(完全に孤立)。
            // 入力ポートと併合された点は unused-input 側で拾うので除外し、
            // 別名併合で同じ点になった reg は代表ノードで 1 回だけ報告する。
            let mut reported: HashSet<usize> = HashSet::new();
            let mut names: Vec<&String> = scope.keys().collect();
            names.sort();
            for name in names {
                if port_names.contains(name.as_str()) {
                    continue;
                }
                let line = decl_lines.get(name.as_str()).copied().unwrap_or(0);
                if let Some(&(back, side)) = side_regs.get(name.as_str()) {
                    // コンパレータ / ロック付きリピーター reg: back・side とも未駆動で、
                    // out が何も駆動しない(出力ポートでもない)なら孤立。
                    let rb = self.c.find(back);
                    let rs = self.c.find(side);
                    let ro = self.c.find(scope[name.as_str()]);
                    if !self.c.nodes[rb].has_incoming
                        && !self.c.nodes[rs].has_incoming
                        && !self.c.nodes[ro].has_outgoing
                        && !self.c.nodes[ro].is_out_port
                    {
                        lint(
                            line,
                            "floating-reg",
                            format!(
                                "reg '{}' in logic '{}' is not connected to anything",
                                name, l.name
                            ),
                        );
                    }
                    continue;
                }
                let r = self.c.find(scope[name.as_str()]);
                let nd = &self.c.nodes[r];
                if nd.kind != NodeKind::Input
                    && !nd.is_out_port
                    && !nd.has_incoming
                    && !nd.has_outgoing
                    && reported.insert(r)
                {
                    lint(
                        line,
                        "floating-reg",
                        format!(
                            "reg '{}' in logic '{}' is not connected to anything",
                            name, l.name
                        ),
                    );
                }
            }
            // バス reg: 全レーン孤立のときのみ 1 回報告(部分レーン未使用は初版では見ない)。
            let mut bus_list: Vec<&String> = buses.keys().collect();
            bus_list.sort();
            for name in bus_list {
                if port_names.contains(name.as_str()) {
                    continue;
                }
                let mut all_floating = true;
                if let Some((backs, sides)) = bus_side_regs.get(name.as_str()) {
                    // バス named point: スカラ版と同じく、全レーンで back・side とも未駆動
                    // かつ out が何も駆動しない(出力ポートでもない)ときのみ孤立と見なす。
                    // out レーンは素子出力なのでエッジの有無だけでは判定できない。
                    let outs = &buses[name.as_str()];
                    for i in 0..outs.len() {
                        let rb = self.c.find(backs[i]);
                        let rs = self.c.find(sides[i]);
                        let ro = self.c.find(outs[i]);
                        if self.c.nodes[rb].has_incoming
                            || self.c.nodes[rs].has_incoming
                            || self.c.nodes[ro].has_outgoing
                            || self.c.nodes[ro].is_out_port
                        {
                            all_floating = false;
                            break;
                        }
                    }
                    if all_floating {
                        lint(
                            decl_lines.get(name.as_str()).copied().unwrap_or(0),
                            "floating-reg",
                            format!(
                                "bus reg '{}' in logic '{}' is not connected to anything",
                                name, l.name
                            ),
                        );
                    }
                    continue;
                }
                for &lane in &buses[name.as_str()] {
                    let r = self.c.find(lane);
                    let nd = &self.c.nodes[r];
                    if nd.kind == NodeKind::Input
                        || nd.is_out_port
                        || nd.has_incoming
                        || nd.has_outgoing
                    {
                        all_floating = false;
                        break;
                    }
                }
                if all_floating {
                    lint(
                        decl_lines.get(name.as_str()).copied().unwrap_or(0),
                        "floating-reg",
                        format!(
                            "bus reg '{}' in logic '{}' is not connected to anything",
                            name, l.name
                        ),
                    );
                }
            }
            // 未使用 wire: 宣言されたがどのチェーンからも展開されなかった素子列。
            let mut w_list: Vec<&String> = wire_names.iter().collect();
            w_list.sort();
            for wn in w_list {
                if !used_wires.contains(wn.as_str()) {
                    lint(
                        decl_lines.get(wn.as_str()).copied().unwrap_or(0),
                        "unused-wire",
                        format!("wire '{}' in logic '{}' is never used in a chain", wn, l.name),
                    );
                }
            }
            // 未使用 input ポート: 全レーンが logic 本体のどこも駆動しない。
            for (name, shape) in &inputs {
                let mut used = false;
                for lane in shape.lanes() {
                    let r = self.c.find(lane);
                    if self.c.nodes[r].has_outgoing {
                        used = true;
                        break;
                    }
                }
                if !used {
                    let pline = l
                        .ports
                        .iter()
                        .find(|p| &p.name == name)
                        .map(|p| p.line)
                        .unwrap_or(0);
                    lint(
                        pline,
                        "unused-input",
                        format!("input port '{}' of logic '{}' is never used", name, l.name),
                    );
                }
            }
        }

        // 到達可能性ルール(上界解析)はトップレベルのインスタンス化ごとに、この
        // elaborate(サブインスタンス込み)で新規生成した素子・ポートに限って報告する。
        // 非トップレベルでは入力ポートが親から未結線(上界が全部 0)なので判定しない。
        if top_level {
            let pot = self.c.lint_potentials();
            // 常時 ON トーチ: 後ろ入力の上界 0 = 消灯する条件が絶対に来ない。
            for si in seq_lo..self.c.seqs.len() {
                if self.c.seqs[si].kind != SeqKind::Torch {
                    continue;
                }
                let in_ = self.c.seqs[si].in_;
                let r = self.c.find(in_);
                if pot[r] == 0 {
                    let (label, line) = (self.c.seqs[si].label.clone(), self.c.seqs[si].line);
                    lint(
                        line,
                        "always-on-torch",
                        format!("torch '{}' is always ON (its input can never be powered)", label),
                    );
                }
            }
            // 到達不能な出力: 出力ポートのレーン上界 0 = どう操作しても点灯しない。
            for (name, shape) in &outs {
                let lanes = shape.lanes();
                let w = lanes.len();
                for (i, &lane) in lanes.iter().enumerate() {
                    let r = self.c.find(lane);
                    if pot[r] == 0 {
                        let pline = l
                            .ports
                            .iter()
                            .find(|p| &p.name == name)
                            .map(|p| p.line)
                            .unwrap_or(0);
                        let disp = if w == 1 {
                            name.clone()
                        } else {
                            format!("{}[{}]", name, i)
                        };
                        lint(
                            pline,
                            "unreachable-output",
                            format!(
                                "output port '{}' of logic '{}' can never be powered (always 0)",
                                disp, l.name
                            ),
                        );
                    }
                }
            }
        }

        self.stack.pop();
        Ok((inputs, outs))
    }
}

// ---- module execution --------------------------------------------------

/// `clock(var, N)` の周期トグル状態。各レベルを `hold` tick 保持し、`counter` が
/// 0 に達するたびに `level` を 0 ↔ 15 で反転する(full period = 2*hold、50% デューティ)。
struct ClockState {
    hold: i64,
    counter: i64,
    level: i64,
}

pub struct ModuleExec<'a> {
    prog: &'a Program,
    m: &'a ModuleDef,
    c: Circuit,
    vars: HashMap<String, i64>,
    insts: BTreeMap<String, Instance>,
    out_bind: BTreeMap<String, usize>,
    mons: Vec<(i32, &'a CallData)>,
    clamp_warned: HashSet<String>,
    sim_time: i64,
    /// パルス代入(`x = v ~ w`)で残り tick を持つ var → 0 で var を 0 に戻す。
    pulses: HashMap<String, i64>,
    /// `clock(x, N)` で自動トグルする var → 周期状態。tick ごとに `tick_clocks` が更新する。
    clocks: HashMap<String, ClockState>,
    /// バス var(`var[N] x;`)の幅。レーンは vars に `x[0]`..`x[N-1]` のキーで格納する。
    var_buses: HashMap<String, i32>,
    /// この module の sim tick 実行(`tick_once`)に費やした累積時間。`--time` 用。
    sim_dur: Duration,
    /// 実行した `assert` / `expect` の総数(自己検証サマリ用)。
    assert_total: i64,
    /// 失敗した `assert` / `expect` の数。1 つ以上なら非ゼロ終了。
    assert_failed: i64,
    /// `--json` モード + 複数 module のとき、各 JSON イベントに `"module"` フィールドを足す。
    /// 単一 module のときは省略(出力をすっきりさせる)。
    many_modules: bool,
    /// 静的 lint(floating-reg 等)を出し終えた logic 名(issue #48)。
    /// module 単位で持ち、同じ logic を何度インスタンス化しても宣言由来の警告は 1 回。
    linted_logics: HashSet<String>,
}

impl<'a> ModuleExec<'a> {
    pub fn new(
        prog: &'a Program,
        m: &'a ModuleDef,
        cfg: Config,
        trace: bool,
        vcd: Option<Vcd>,
        many_modules: bool,
    ) -> Self {
        ModuleExec {
            prog,
            m,
            c: Circuit::new(cfg, trace, vcd),
            vars: HashMap::new(),
            insts: BTreeMap::new(),
            out_bind: BTreeMap::new(),
            mons: Vec::new(),
            clamp_warned: HashSet::new(),
            sim_time: 0,
            pulses: HashMap::new(),
            clocks: HashMap::new(),
            var_buses: HashMap::new(),
            sim_dur: Duration::ZERO,
            assert_total: 0,
            assert_failed: 0,
            many_modules,
            linted_logics: HashSet::new(),
        }
    }

    /// バス var のレーン vars キー `name[k]` を作る。
    fn lane_key(name: &str, k: i64) -> String {
        format!("{}[{}]", name, k)
    }

    /// バス var `name` の幅を返す(バスでなければ None)。
    fn bus_width(&self, name: &str) -> Option<i32> {
        self.var_buses.get(name).copied()
    }

    pub fn run(&mut self) -> RvResult<()> {
        let m = self.m;
        self.exec_list(&m.pre)?;
        Self::collect_monitors(&m.sim, &mut self.mons);
        self.exec_list(&m.sim)?;
        Ok(())
    }

    fn collect_monitors(body: &'a [SimStmt], out: &mut Vec<(i32, &'a CallData)>) {
        for s in body {
            match s {
                SimStmt::MonReg { line, call } => out.push((*line, call)),
                SimStmt::If {
                    body, else_body, ..
                } => {
                    Self::collect_monitors(body, out);
                    Self::collect_monitors(else_body, out);
                }
                SimStmt::While { body, .. } | SimStmt::For { body, .. } => {
                    Self::collect_monitors(body, out);
                }
                _ => {}
            }
        }
    }

    fn eval_e(&self, e: &Expr) -> RvResult<i64> {
        match e {
            Expr::Num { num, .. } => Ok(*num),
            Expr::Time { .. } => Ok(self.sim_time),
            Expr::Var { line, name, index } => match index {
                None => match self.vars.get(name) {
                    Some(v) => Ok(*v),
                    None if self.var_buses.contains_key(name) => fail(
                        *line,
                        format!("'{}' is a bus var; index a lane (e.g. '{}[0]')", name, name),
                    ),
                    // var に無ければ param 定数(`param` / 数値 `#define`)を引く。
                    None => match self.prog.defines.get(name) {
                        Some(v) => Ok(*v),
                        None => fail(*line, format!("undeclared variable: {}", name)),
                    },
                },
                Some(e) => {
                    let w = match self.bus_width(name) {
                        Some(w) => w as i64,
                        None => {
                            return fail(
                                *line,
                                format!("'{}' is not a bus var; cannot index it", name),
                            )
                        }
                    };
                    let k = self.eval_e(e)?;
                    if k < 0 || k >= w {
                        return fail(
                            *line,
                            format!("bus var index out of range: {}[{}] (width {})", name, k, w),
                        );
                    }
                    let key = Self::lane_key(name, k);
                    match self.vars.get(&key) {
                        Some(v) => Ok(*v),
                        None => fail(*line, format!("undeclared variable: {}", key)),
                    }
                }
            },
            Expr::Un { op, a, .. } => {
                let v = self.eval_e(a)?;
                Ok(match op.as_str() {
                    "-" => -v,
                    "!" => (v == 0) as i64,
                    _ => v,
                })
            }
            Expr::Bin { line, op, a, b } => {
                if op == "&&" {
                    return Ok(((self.eval_e(a)? != 0) && (self.eval_e(b)? != 0)) as i64);
                }
                if op == "||" {
                    return Ok(((self.eval_e(a)? != 0) || (self.eval_e(b)? != 0)) as i64);
                }
                let a = self.eval_e(a)?;
                let b = self.eval_e(b)?;
                Ok(match op.as_str() {
                    "+" => a + b,
                    "-" => a - b,
                    "*" => a * b,
                    "/" => {
                        if b == 0 {
                            return fail(*line, "division by zero");
                        }
                        a / b
                    }
                    "%" => {
                        if b == 0 {
                            return fail(*line, "modulo by zero");
                        }
                        a % b
                    }
                    "<" => (a < b) as i64,
                    "<=" => (a <= b) as i64,
                    ">" => (a > b) as i64,
                    ">=" => (a >= b) as i64,
                    "==" => (a == b) as i64,
                    "!=" => (a != b) as i64,
                    _ => return fail(*line, format!("unknown operator: {}", op)),
                })
            }
        }
    }

    fn apply_inputs(&mut self) -> RvResult<()> {
        let mut pairs: Vec<(usize, String)> = Vec::new();
        for inst in self.insts.values() {
            for k in 0..inst.in_nodes.len() {
                pairs.push((inst.in_nodes[k], inst.in_vars[k].clone()));
            }
        }
        for (node, var) in pairs {
            let v = match self.vars.get(&var) {
                Some(v) => *v,
                None => {
                    return fail(0, format!("undeclared variable bound to logic input: {}", var))
                }
            };
            let mut vv = v;
            if !(0..=15).contains(&v) {
                let cv = if v < 0 { 0 } else { 15 };
                if !self.clamp_warned.contains(&var) {
                    warn(
                        0,
                        format!(
                            "variable '{}' value {} is outside signal range 0-15; clamped to {}",
                            var, v, cv
                        ),
                    );
                    self.clamp_warned.insert(var.clone());
                }
                vv = cv;
            }
            self.c.set_input(node, vv as i32);
        }
        Ok(())
    }

    fn apply_outputs(&mut self) {
        let pairs: Vec<(String, usize)> =
            self.out_bind.iter().map(|(k, v)| (k.clone(), *v)).collect();
        for (var, node) in pairs {
            let val = self.c.read(node) as i64;
            self.vars.insert(var, val);
        }
    }

    fn tick_once(&mut self) -> RvResult<bool> {
        let t = Instant::now();
        self.apply_inputs()?;
        let ch = self.c.step();
        self.apply_outputs();
        self.tick_pulses();
        self.tick_clocks();
        self.sim_dur += t.elapsed();
        Ok(ch)
    }

    /// 各 tick 末に保留中パルスを 1 減らし、0 に達した var を 0 へ戻す。
    fn tick_pulses(&mut self) {
        if self.pulses.is_empty() {
            return;
        }
        let mut expired: Vec<String> = Vec::new();
        for (var, left) in self.pulses.iter_mut() {
            *left -= 1;
            if *left <= 0 {
                expired.push(var.clone());
            }
        }
        for var in expired {
            self.pulses.remove(&var);
            self.vars.insert(var, 0);
        }
    }

    /// 各 tick 末にクロックの残り tick を 1 減らし、0 に達した var をトグル(0 ↔ 15)する。
    fn tick_clocks(&mut self) {
        if self.clocks.is_empty() {
            return;
        }
        let mut toggled: Vec<(String, i64)> = Vec::new();
        for (var, st) in self.clocks.iter_mut() {
            st.counter -= 1;
            if st.counter <= 0 {
                st.level = if st.level == 0 { 15 } else { 0 };
                st.counter = st.hold;
                toggled.push((var.clone(), st.level));
            }
        }
        for (var, level) in toggled {
            self.vars.insert(var, level);
        }
    }

    /// `clock(var, N)` — var を「各レベル N tick 保持」で 0/15 に自動トグルさせる。
    /// 呼び出し直後は Low(0)。clock() 自体は時刻を進めず、後続の `#n`/`wait`/`#until` が
    /// tick を刻む間にトグルする(パルス代入と同型)。同じ var への通常代入で解除される。
    fn do_clock(&mut self, line: i32, call: &CallData) -> RvResult<()> {
        if call.has_fmt || call.args.len() != 2 {
            return fail(line, "clock(var, N) takes a var name and a period");
        }
        let name = match &call.args[0] {
            Expr::Var { name, index: None, .. } => name.clone(),
            Expr::Var { .. } => {
                return fail(line, "clock(var, N): first argument must be a scalar var, not a bus lane");
            }
            _ => return fail(line, "clock(var, N): first argument must be a var name"),
        };
        if self.var_buses.contains_key(&name) {
            return fail(line, format!("clock on a bus var '{}' is not supported", name));
        }
        if !self.vars.contains_key(&name) {
            return fail(line, format!("undeclared variable: {}", name));
        }
        let n = self.eval_e(&call.args[1])?;
        if n < 1 {
            return fail(line, format!("clock period must be >= 1 (got {})", n));
        }
        self.vars.insert(name.clone(), 0);
        self.pulses.remove(&name);
        self.clocks.insert(name, ClockState { hold: n, counter: n, level: 0 });
        Ok(())
    }

    fn run_ticks(&mut self, n: i64, advance_time: bool) -> RvResult<()> {
        for _ in 0..n {
            self.tick_once()?;
            if advance_time {
                self.sim_time += 1;
            }
        }
        Ok(())
    }

    fn do_init(&mut self, line: i32) -> RvResult<()> {
        let mut t = 0i64;
        loop {
            let ch = self.tick_once()?;
            t += 1;
            if !ch {
                break;
            }
            if t > self.c.cfg.init_timeout {
                return fail(
                    line,
                    format!(
                        "#init did not reach a steady state within {} ticks (oscillating circuit? raise INIT_TIMEOUT or use wait(n) instead)",
                        self.c.cfg.init_timeout
                    ),
                );
            }
        }
        self.sim_time = 0;
        Ok(())
    }

    /// `#until(cond)` — `cond` が真になるまで tick を進める(`$time` は進む)。
    /// 既に真なら 0 tick。`INIT_TIMEOUT` 超過で発振 or 永遠に成立しないとしてエラー。
    fn do_until(&mut self, line: i32, cond: &Expr) -> RvResult<()> {
        let mut t = 0i64;
        while self.eval_e(cond)? == 0 {
            self.tick_once()?;
            self.sim_time += 1;
            t += 1;
            if t > self.c.cfg.init_timeout {
                return fail(
                    line,
                    format!(
                        "#until(...) condition not satisfied within {} ticks (oscillating circuit or unreachable condition? raise INIT_TIMEOUT or use wait(n) instead)",
                        self.c.cfg.init_timeout
                    ),
                );
            }
        }
        Ok(())
    }

    fn do_monitor(&self, line: i32, call: &CallData) -> RvResult<()> {
        if !call.has_fmt {
            return fail(line, "monitor() requires a format string as its first argument");
        }
        // 引数を評価する。バス var(添字なし)が直接渡されたときは、各レーン強度を
        // 下位ニブルからパッキングして 1 個の整数にする(lane[0] = 最下位 4 bit)。
        // 2 番目の値はバス幅 N(バスでなければ None)で、書式側の既定幅に使う。
        let mut av: Vec<(i64, Option<i32>)> = Vec::new();
        for a in &call.args {
            av.push(self.eval_monitor_arg(line, a)?);
        }
        if is_json_mode() {
            let vals: Vec<i64> = av.iter().map(|(v, _)| *v).collect();
            self.emit_monitor_json(&call.fmt, &vals);
            return Ok(());
        }
        let f: Vec<char> = call.fmt.chars().collect();
        let mut out = String::new();
        let mut ai = 0usize;
        let mut i = 0usize;
        while i < f.len() {
            let c = f[i];
            if c == '%' {
                let mut j = i + 1;
                if j < f.len() && f[j] == '%' {
                    out.push('%');
                    i = j;
                    i += 1;
                    continue;
                }
                // フラグ: 先頭が '0' で、後ろに 1 桁以上の数字が続けばゼロ埋め。
                // ('0' 単独や '0b'/'0x'/'0o' の '0' は幅 0 と解釈し、ゼロ埋め扱いしない)
                let mut zero_pad = false;
                if j < f.len() && f[j] == '0'
                    && j + 1 < f.len()
                    && f[j + 1].is_ascii_digit()
                {
                    zero_pad = true;
                    j += 1;
                }
                let mut width = 0usize;
                while j < f.len() && f[j].is_ascii_digit() {
                    width = width * 10 + (f[j] as usize - '0' as usize);
                    j += 1;
                }
                if j < f.len() && (f[j] == 't' || f[j] == 'd') {
                    return fail(
                        line,
                        format!(
                            "monitor format: type suffix '%{0}' is not supported; use '%' or '%N' for width (e.g. '%2' instead of '%2{0}')",
                            f[j]
                        ),
                    );
                }
                let base = match f.get(j) {
                    Some('b') => Some(2u32),
                    Some('x') => Some(16),
                    Some('o') => Some(8),
                    _ => None,
                };
                if base.is_some() {
                    j += 1;
                }
                let val_str = if ai < av.len() {
                    let (v, bus_w) = av[ai];
                    ai += 1;
                    // バス var 引数は無符号整数(全レーンのニブル合成)として表示する。
                    // %b / %x ではレーン境界を保つため、先に 4N bit / N 桁にゼロ埋めしてから、
                    // ユーザー指定幅 N を追加分のパディング(ユーザー指定のフラグ)で被せる。
                    let body = match bus_w {
                        Some(w) => {
                            let raw = format_uint(v as u64, base.unwrap_or(10));
                            match base {
                                Some(2) => pad_value(&raw, (w as usize) * 4, true),
                                Some(16) => pad_value(&raw, w as usize, true),
                                _ => raw,
                            }
                        }
                        None => format_int(v, base.unwrap_or(10)),
                    };
                    pad_value(&body, width, zero_pad)
                } else {
                    pad_value("<?>", width, zero_pad)
                };
                out.push_str(&val_str);
                i = j;
            } else {
                out.push(c);
                i += 1;
            }
        }
        print!("{}", out);
        std::io::stdout().flush().ok();
        Ok(())
    }

    /// monitor / `?monitor` の 1 引数を評価する。
    /// 引数がバス var(添字なし)なら、各レーン強度の下位 4 bit を `lane[0]` から
    /// 順に nibble としてパッキングし(`lane[0]` = 最下位)、合成整数を返す。
    /// 戻り値 2 番目はバス幅 N(バスでなければ None)で、書式側の既定幅に使う。
    /// バス幅 16 を超える bus var は i64 に収まらないためエラーにする。
    fn eval_monitor_arg(&self, line: i32, e: &Expr) -> RvResult<(i64, Option<i32>)> {
        if let Expr::Var { name, index: None, .. } = e {
            if let Some(&w) = self.var_buses.get(name) {
                if w > 16 {
                    return fail(
                        line,
                        format!(
                            "monitor: bus var '{}' width {} > 16 cannot be packed into one integer; \
                             monitor lanes individually with '{}[k]'",
                            name, w, name
                        ),
                    );
                }
                let mut packed: u64 = 0;
                for k in 0..w {
                    let key = Self::lane_key(name, k as i64);
                    let lane = self.vars.get(&key).copied().unwrap_or(0);
                    packed |= ((lane as u64) & 0xF) << (4 * k as u32);
                }
                return Ok((packed as i64, Some(w)));
            }
        }
        Ok((self.eval_e(e)?, None))
    }

    /// `assert(cond);` — cond を評価し、偽(= 0)なら失敗として記録し stderr に出力する。
    /// 失敗しても sim は継続する(全チェックを実行して末尾でサマリ + 非ゼロ終了)。
    fn do_assert(&mut self, line: i32, call: &CallData) -> RvResult<()> {
        if call.has_fmt || call.args.len() != 1 {
            return fail(line, "assert(cond) takes exactly one condition expression");
        }
        let v = self.eval_e(&call.args[0])?;
        self.assert_total += 1;
        if v == 0 {
            self.assert_failed += 1;
            let expr = expr_to_string(&call.args[0]);
            if is_json_mode() {
                let mut s = String::from("{\"kind\":\"assert\"");
                self.json_append_module(&mut s);
                use std::fmt::Write;
                let _ = write!(s, ",\"line\":{},\"expr\":", line);
                json_escape_into(&expr, &mut s);
                s.push('}');
                eprintln!("{}", s);
            } else {
                eprintln!("[assert] line {}: assertion failed: {}", line, expr);
            }
        }
        Ok(())
    }

    /// `expect(actual, expected);` — actual と expected を評価し、不一致なら失敗として
    /// 記録し「実際の値 / 期待値」を stderr に出力する。`assert` と同じく継続する。
    fn do_expect(&mut self, line: i32, call: &CallData) -> RvResult<()> {
        if call.has_fmt || call.args.len() != 2 {
            return fail(line, "expect(actual, expected) takes exactly two expressions");
        }
        let actual = self.eval_e(&call.args[0])?;
        let expected = self.eval_e(&call.args[1])?;
        self.assert_total += 1;
        if actual != expected {
            self.assert_failed += 1;
            let expr = expr_to_string(&call.args[0]);
            if is_json_mode() {
                let mut s = String::from("{\"kind\":\"expect\"");
                self.json_append_module(&mut s);
                use std::fmt::Write;
                let _ = write!(
                    s,
                    ",\"line\":{},\"expr\":",
                    line
                );
                json_escape_into(&expr, &mut s);
                let _ = write!(s, ",\"actual\":{},\"expected\":{}", actual, expected);
                s.push('}');
                eprintln!("{}", s);
            } else {
                eprintln!(
                    "[assert] line {}: expect failed: {} = {}, expected {}",
                    line,
                    expr,
                    actual,
                    expected
                );
            }
        }
        Ok(())
    }

    /// 1 件の `?monitor` / `monitor` を JSONL に出力する。
    fn emit_monitor_json(&self, fmt: &str, values: &[i64]) {
        let mut s = String::from("{");
        let mut need_comma = false;
        if self.many_modules {
            s.push_str("\"module\":");
            json_escape_into(&self.m.name, &mut s);
            need_comma = true;
        }
        use std::fmt::Write;
        if need_comma {
            s.push(',');
        }
        let _ = write!(s, "\"time\":{},\"values\":[", self.sim_time);
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, "{}", v);
        }
        s.push_str("],\"fmt\":");
        json_escape_into(fmt, &mut s);
        s.push('}');
        println!("{}", s);
        std::io::stdout().flush().ok();
    }

    /// 多モジュール JSON 出力時の `"module":"..."` 接頭辞を `out` に append する
    /// (頭の `,` 付き)。単一モジュールなら何もしない。
    fn json_append_module(&self, out: &mut String) {
        if self.many_modules {
            out.push_str(",\"module\":");
            json_escape_into(&self.m.name, out);
        }
    }

    fn fire_monitors(&mut self) -> RvResult<()> {
        let mons = self.mons.clone();
        for (line, call) in mons {
            self.do_monitor(line, call)?;
        }
        Ok(())
    }

    /// `x = scan();` または `x = scan("%b" | "%x" | "%o" | "%");` —
    /// stdin から空白/改行区切りの整数を 1 つ読み、変数に代入する。
    /// 書式を渡すと指定基数(2 / 16 / 8 / 10)で読む。EOF・非数値はエラー。
    fn do_scan(
        &mut self,
        line: i32,
        target: &BindTarget,
        bind_args: &[InstArg],
        fmt: Option<&str>,
    ) -> RvResult<()> {
        if !bind_args.is_empty() {
            return fail(line, "scan() takes no arguments");
        }
        // scan は logic 束縛ではないので、レーン / スライス添字(issue #118)や
        // 連結(issue #123)は対象外。
        let r = match target {
            BindTarget::Ref(r) if matches!(r.sel, Sel::All) => r,
            _ => {
                return fail(
                    line,
                    format!(
                        "scan() cannot target a bus lane/slice or concatenation '{}'; \
                         scan into a scalar var",
                        bind_target_desc(target)
                    ),
                );
            }
        };
        let target = r.name.as_str();
        if self.var_buses.contains_key(target) {
            return fail(
                line,
                format!("scan() cannot target a whole bus var '{}'; scan into a scalar var", target),
            );
        }
        if !self.vars.contains_key(target) {
            return fail(line, format!("undeclared variable: {}", target));
        }
        let base = parse_scan_fmt(line, fmt)?;
        let v = read_stdin_int(line, base)?;
        self.vars.insert(target.to_string(), v);
        Ok(())
    }

    /// インスタンスを(未生成なら)生成して正規化キーを返す。
    /// ネスト呼び出し引数(issue #97)は先に部分インスタンスを再帰的に確定し、その
    /// 正規化キーを引数キーに使う。同じ部分式は standalone 呼び出しとも同一インスタンス
    /// を共有する(「同じ logic 名と引数列は同一インスタンス」の自然な拡張)。
    fn ensure_instance(
        &mut self,
        line: i32,
        callee: &str,
        bind_args: &[InstArg],
        call_params: &[(String, Expr)],
    ) -> RvResult<String> {
        let prog: &'a Program = self.prog;
        let lit = lookup_logic(&prog.logics, callee, line)?;
        // sim 側からの呼び出しは caller env(logic ローカル param)を持たない。
        // 既定値 + `#(...)` 実引数だけで callee の `param_env` を作る。
        let caller_env: HashMap<String, i64> = HashMap::new();
        let env = build_callee_param_env(
            callee,
            &lit.params,
            call_params,
            &caller_env,
            &prog.defines,
            line,
        )?;
        // インスタンスキャッシュキーには `#(...)` 部分も含める。`#(W=4)` と `#(W=8)` は
        // 別インスタンス(別ノード群)としてエラボレートする。targets はキーに含めない
        // (同じ呼び出しを別の var 組で受けても回路は同一インスタンスを共有する)。
        let param_part = param_env_key(&lit.params, &env);
        // 引数キー: 名前は添字込みで正規化(issue #118: `g(x[0])` と `g(x[1])` は
        // 別インスタンス。`x[k:k]` は `x[k]` と等価なのでレーン形へ正規化)、ネスト
        // 呼び出しは部分インスタンスを先に確定してその正規化キーを使う(外側が
        // キャッシュ済みならネスト側もキャッシュ済みで no-op)。
        let mut arg_keys: Vec<String> = Vec::with_capacity(bind_args.len());
        for a in bind_args {
            match a {
                InstArg::Name { name: n, sel } => {
                    arg_keys.push(sim_arg_key(n, sel, &prog.defines, line)?)
                }
                // 連結引数(issue #123)は要素キーを `{...}` で束ねる。単一要素の
                // `{x}` は `x` と等価な配線になるので裸の形へ正規化する
                // (`x[k:k]` -> `x[k]` と同じ流儀。`g({x})` は `g(x)` と同一インスタンス)。
                InstArg::Concat(parts) => {
                    let mut ks: Vec<String> = Vec::with_capacity(parts.len());
                    for r in parts {
                        ks.push(sim_arg_key(&r.name, &r.sel, &prog.defines, line)?);
                    }
                    arg_keys.push(if ks.len() == 1 {
                        ks.pop().unwrap()
                    } else {
                        format!("{{{}}}", ks.join(","))
                    });
                }
                InstArg::Call {
                    line: cl,
                    callee: sub,
                    params,
                    args,
                } => {
                    if sub == "scan" {
                        return fail(*cl, "scan() cannot be nested in a logic call argument");
                    }
                    arg_keys.push(self.ensure_instance(*cl, sub, args, params)?);
                }
            }
        }
        let key = format!("{}{}({})", callee, param_part, arg_keys.join(","));
        if self.insts.contains_key(&key) {
            return Ok(key);
        }
        for a in bind_args {
            let refs: Vec<(&str, &Sel)> = match a {
                InstArg::Name { name: n, sel } => vec![(n.as_str(), sel)],
                InstArg::Concat(parts) => {
                    parts.iter().map(|r| (r.name.as_str(), &r.sel)).collect()
                }
                InstArg::Call { .. } => Vec::new(),
            };
            for (n, sel) in refs {
                if !self.vars.contains_key(n) && !self.var_buses.contains_key(n) {
                    return fail(line, format!("undeclared variable passed to logic: {}", n));
                }
                // 添字はバス var にしか付けられない(範囲検査は束縛時)。
                if !matches!(sel, Sel::All) && !self.var_buses.contains_key(n) {
                    return fail(line, format!("'{}' is not a bus var; cannot index it", n));
                }
            }
        }
        // ノード名プレフィックスはキーと分ける(issue #101)。名前の `#` はトレース /
        // VCD 非表示の印なので、`#(W=2)` 由来の `#` をキーのまま使うとジェネリック
        // インスタンスの全ノードが観測不能になる。キー中の `#` は `#(` の形でしか
        // 現れない(引数キーは識別子かネストインスタンスキー)ため、この置換で足りる。
        let prefix = key.replace("#(", "(");
        // logic を展開してポート形(スカラ/バス)を得る。
        let (inputs, outputs) = {
            let mut el = Elaborator {
                c: &mut self.c,
                logics: &prog.logics,
                defines: &prog.defines,
                stack: Vec::new(),
                counter: 0,
                linted: &mut self.linted_logics,
            };
            el.elaborate(lit, &prefix, true, line, bind_args.len(), env)?
        };
        require_output_ports(callee, outputs.len(), line)?;
        // 各引数を入力ポートにレーン対応で束縛する。var(スカラ/バス)は毎 tick の入力反映
        // (apply_inputs)対象として in_vars に載せ、ネスト呼び出しは部分インスタンスの
        // 単一出力ポートから減衰なしで直結する(回路内配線なので in_vars には載らない)。
        let mut in_vars: Vec<String> = Vec::new();
        let mut in_nodes: Vec<usize> = Vec::new();
        for (i, (arg, (pname, pshape))) in bind_args.iter().zip(inputs.iter()).enumerate() {
            let lanes = pshape.lanes();
            match arg {
                InstArg::Name { name: n, sel } => match self.bus_width(n) {
                    Some(w) => {
                        // レーン選択をレーン番号列へ解決し(All は昇順 [0..w)、issue #118)、
                        // ポートの昇順レーンと並び順で対応させる(チェーン端点と同じ規則)。
                        let idxs = sel_lane_indices(sel, w as usize, n, &prog.defines, line)?;
                        if lanes.len() != idxs.len() {
                            let what = if matches!(sel, Sel::All) {
                                format!("argument '{}' (bus var width {})", n, w)
                            } else {
                                format!(
                                    "argument '{}{}' (width {})",
                                    n,
                                    sel_suffix(sel),
                                    idxs.len()
                                )
                            };
                            return fail(
                                line,
                                format!(
                                    "{} does not match {} input port '{}' (width {})",
                                    what, callee, pname, lanes.len()
                                ),
                            );
                        }
                        for (node, k) in lanes.iter().zip(idxs.iter()) {
                            in_vars.push(Self::lane_key(n, *k as i64));
                            in_nodes.push(*node);
                        }
                    }
                    None => {
                        if lanes.len() != 1 {
                            return fail(
                                line,
                                format!(
                                    "argument '{}' is a scalar var but {} input port '{}' is a bus (width {})",
                                    n, callee, pname, lanes.len()
                                ),
                            );
                        }
                        in_vars.push(n.clone());
                        in_nodes.push(lanes[0]);
                    }
                },
                // 連結引数(issue #123): 各要素のレーンキー列を並び順に連接し、
                // ポートの昇順レーンと対応させる(チェーン端点の連結と同じ規則)。
                InstArg::Concat(parts) => {
                    let mut keys: Vec<String> = Vec::new();
                    for r in parts {
                        match self.bus_width(&r.name) {
                            Some(w) => {
                                let idxs = sel_lane_indices(
                                    &r.sel,
                                    w as usize,
                                    &r.name,
                                    &prog.defines,
                                    line,
                                )?;
                                keys.extend(
                                    idxs.into_iter().map(|k| Self::lane_key(&r.name, k as i64)),
                                );
                            }
                            // スカラ var(添字が付かないことは検査済み)
                            None => keys.push(r.name.clone()),
                        }
                    }
                    if lanes.len() != keys.len() {
                        return fail(
                            line,
                            format!(
                                "argument '{}' (width {}) does not match {} input port '{}' (width {})",
                                concat_desc(parts),
                                keys.len(),
                                callee,
                                pname,
                                lanes.len()
                            ),
                        );
                    }
                    for (node, k) in lanes.iter().zip(keys.iter()) {
                        in_vars.push(k.clone());
                        in_nodes.push(*node);
                    }
                }
                InstArg::Call {
                    line: cl,
                    callee: sub,
                    ..
                } => {
                    let sub_out = self.insts.get(&arg_keys[i]).unwrap().out_ports.clone();
                    require_single_output(sub, sub_out.len(), *cl)?;
                    // `Input` ノードはエッジ寄与を max 合流する(issue #99)ので、logic 本体側の
                    // ネストと同じく connect_ports で部分インスタンス出力から直結するだけでよい
                    // (var 駆動なし = base 0)。幅検査とエラー文も connect_ports に一本化。
                    let src = PortShape::Bus(sub_out[0].clone());
                    let ctx = format!("{} input port '{}'", callee, pname);
                    connect_ports(&mut self.c, &src, pshape, &ctx, *cl)?;
                }
            }
        }
        // 出力ポートごとのレーン列を保持(順序は callee の宣言順)。
        let out_ports: Vec<Vec<usize>> = outputs.iter().map(|(_, sh)| sh.lanes()).collect();
        self.insts.insert(
            key.clone(),
            Instance {
                in_vars,
                in_nodes,
                out_ports,
            },
        );
        Ok(key)
    }

    fn do_call_bind(
        &mut self,
        line: i32,
        targets: &[BindTarget],
        callee: &str,
        bind_args: &[InstArg],
        call_params: &[(String, Expr)],
    ) -> RvResult<()> {
        let key = self.ensure_instance(line, callee, bind_args, call_params)?;
        let out_ports = self.insts.get(&key).unwrap().out_ports.clone();
        check_binding_arity(callee, out_ports.len(), targets.len(), line)?;
        // 各 target をレーンキー列(out_bind / vars のキー、束縛順)へ解決してから、
        // 解決後レーンの重複を検査する(レーン / スライス target の部分重複も検出。issue #118)。
        let mut target_keys: Vec<Vec<String>> = Vec::with_capacity(targets.len());
        for t in targets {
            target_keys.push(self.bind_target_keys(t, line)?);
        }
        let descs: Vec<String> = targets.iter().map(bind_target_desc).collect();
        check_target_overlap(&descs, &target_keys, line)?;
        // 各出力ポートを対応する target のレーンキー列へレーン対応で束縛(呼び出しごと)。
        for ((t, keys), out_lanes) in targets.iter().zip(target_keys.iter()).zip(out_ports.iter())
        {
            if out_lanes.len() != keys.len() {
                return fail(
                    line,
                    match t {
                        BindTarget::Ref(r) if self.bus_width(&r.name).is_none() => format!(
                            "{} has a bus output (width {}); bind it to a bus var (e.g. 'var[{}] {};')",
                            callee, out_lanes.len(), out_lanes.len(), r.name
                        ),
                        BindTarget::Ref(r) if matches!(r.sel, Sel::All) => format!(
                            "output of {} (width {}) does not match bus var '{}' (width {})",
                            callee, out_lanes.len(), r.name, keys.len()
                        ),
                        _ => format!(
                            "output of {} (width {}) does not match target '{}' (width {})",
                            callee, out_lanes.len(), bind_target_desc(t), keys.len()
                        ),
                    },
                );
            }
            for (lk, node) in keys.iter().zip(out_lanes.iter()) {
                self.out_bind.insert(lk.clone(), *node);
                let val = self.c.read(*node) as i64;
                self.vars.insert(lk.clone(), val);
            }
        }
        Ok(())
    }

    /// 束縛 target をレーンキー列(out_bind / vars のキー、束縛順)へ解決する。
    /// スカラ var は自身のキー 1 個、バス var はレーン選択に応じたレーンキー列
    /// (All は昇順 [0..w)、レーン / スライスは §6.3 の並び順。issue #118)。
    /// 連結 target(issue #123)は各要素のレーンキー列を並び順に連接する。
    fn bind_target_keys(&self, t: &BindTarget, line: i32) -> RvResult<Vec<String>> {
        let mut keys: Vec<String> = Vec::new();
        for r in t.refs() {
            match self.bus_width(&r.name) {
                Some(w) => {
                    let idxs =
                        sel_lane_indices(&r.sel, w as usize, &r.name, &self.prog.defines, line)?;
                    keys.extend(idxs.into_iter().map(|k| Self::lane_key(&r.name, k as i64)));
                }
                None => {
                    if !self.vars.contains_key(&r.name) {
                        return fail(line, format!("undeclared variable: {}", r.name));
                    }
                    if !matches!(r.sel, Sel::All) {
                        return fail(
                            line,
                            format!("'{}' is not a bus var; cannot index it", r.name),
                        );
                    }
                    keys.push(r.name.clone());
                }
            }
        }
        Ok(keys)
    }

    fn exec_list(&mut self, body: &'a [SimStmt]) -> RvResult<()> {
        for s in body {
            self.exec(s)?;
        }
        Ok(())
    }

    fn exec(&mut self, s: &'a SimStmt) -> RvResult<()> {
        match s {
            SimStmt::DeclVar { line, decls } => {
                for (name, e, width) in decls {
                    if self.vars.contains_key(name) || self.var_buses.contains_key(name) {
                        return fail(*line, format!("duplicate variable: {}", name));
                    }
                    let v = match e {
                        Some(ex) => self.eval_e(ex)?,
                        None => 0,
                    };
                    match width {
                        // バス var: レーン name[0]..name[w-1] を初期化式(= 全レーン共通)で作る。
                        Some(w) => {
                            for k in 0..*w {
                                self.vars.insert(Self::lane_key(name, k as i64), v);
                            }
                            self.var_buses.insert(name.clone(), *w);
                        }
                        None => {
                            self.vars.insert(name.clone(), v);
                        }
                    }
                }
            }
            SimStmt::Assign {
                line,
                target,
                index,
                value,
                pulse,
            } => {
                let v = self.eval_e(value)?;
                // 代入先キーを決める: name[idx] / バス全体ブロードキャスト / スカラ。
                let keys: Vec<String> = match index {
                    Some(e) => {
                        let w = match self.bus_width(target) {
                            Some(w) => w as i64,
                            None => {
                                return fail(
                                    *line,
                                    format!("'{}' is not a bus var; cannot index it", target),
                                )
                            }
                        };
                        let k = self.eval_e(e)?;
                        if k < 0 || k >= w {
                            return fail(
                                *line,
                                format!(
                                    "bus var index out of range: {}[{}] (width {})",
                                    target, k, w
                                ),
                            );
                        }
                        vec![Self::lane_key(target, k)]
                    }
                    None => match self.bus_width(target) {
                        // バス全体への代入は全レーンへブロードキャスト。
                        Some(w) => {
                            if pulse.is_some() {
                                return fail(
                                    *line,
                                    format!(
                                        "pulse assignment is not supported on a whole bus var '{}'; \
                                         index a lane (e.g. '{}[0] = v ~ w;')",
                                        target, target
                                    ),
                                );
                            }
                            (0..w).map(|k| Self::lane_key(target, k as i64)).collect()
                        }
                        None => {
                            if !self.vars.contains_key(target) {
                                return fail(
                                    *line,
                                    format!("undeclared variable: {}", target),
                                );
                            }
                            vec![target.clone()]
                        }
                    },
                };
                for key in &keys {
                    self.vars.insert(key.clone(), v);
                    // var への代入は、その var に掛かっていたクロックを解除する。
                    self.clocks.remove(key);
                }
                // パルス幅 Some(w) ならリセットを予約(対象キー)。通常代入は保留中パルスを解除。
                match pulse {
                    Some(w) => {
                        let w = self.eval_e(w)?;
                        if w < 1 {
                            return fail(
                                *line,
                                format!("pulse width must be >= 1 (got {})", w),
                            );
                        }
                        for key in &keys {
                            self.pulses.insert(key.clone(), w);
                        }
                    }
                    None => {
                        for key in &keys {
                            self.pulses.remove(key);
                        }
                    }
                }
            }
            SimStmt::CallBind {
                line,
                targets,
                callee,
                bind_args,
                params,
                fmt,
            } => {
                // 同一レーンを 2 度以上束縛するのはエラー(out_bind の暗黙上書きを禁止)。
                // 検査は target をレーンキー列へ解決した後の do_call_bind で行う。
                if callee == "scan" {
                    if !params.is_empty() {
                        return fail(*line, "scan() does not take generic '#(...)' parameters");
                    }
                    if targets.len() != 1 {
                        return fail(
                            *line,
                            "scan() returns a single value; use 'v = scan(...)' not a tuple binding",
                        );
                    }
                    self.do_scan(*line, &targets[0], bind_args, fmt.as_deref())?;
                } else {
                    if fmt.is_some() {
                        return fail(*line, "format string is only allowed for scan()");
                    }
                    self.do_call_bind(*line, targets, callee, bind_args, params)?;
                }
            }
            SimStmt::WaitTicks { ticks, .. } => {
                self.run_ticks(*ticks, true)?;
                self.fire_monitors()?;
            }
            SimStmt::WaitInit { line } => {
                self.do_init(*line)?;
                self.fire_monitors()?;
            }
            SimStmt::WaitUntil { line, cond } => {
                self.do_until(*line, cond)?;
                self.fire_monitors()?;
            }
            SimStmt::MonReg { .. } => {} // hoisted at sim start
            SimStmt::Call { line, call } => {
                if call.callee == "monitor" {
                    self.do_monitor(*line, call)?;
                } else if call.callee == "wait" {
                    if call.args.len() != 1 || call.has_fmt {
                        return fail(*line, "wait(n) takes exactly one numeric argument");
                    }
                    let n = self.eval_e(&call.args[0])?;
                    if n < 0 {
                        return fail(*line, "wait(n): n must be >= 0");
                    }
                    self.run_ticks(n, false)?; // $time を進めず、monitor も発火しない
                } else if call.callee == "clock" {
                    self.do_clock(*line, call)?;
                } else if call.callee == "assert" {
                    self.do_assert(*line, call)?;
                } else if call.callee == "expect" {
                    self.do_expect(*line, call)?;
                } else {
                    return fail(*line, format!("unknown system function: {}", call.callee));
                }
            }
            SimStmt::If {
                cond,
                body,
                else_body,
                ..
            } => {
                if self.eval_e(cond)? != 0 {
                    self.exec_list(body)?;
                } else {
                    self.exec_list(else_body)?;
                }
            }
            SimStmt::While { line, cond, body } => {
                let mut guard: i64 = 10_000_000;
                while self.eval_e(cond)? != 0 {
                    self.exec_list(body)?;
                    guard -= 1;
                    if guard == 0 {
                        return fail(*line, "while loop exceeded iteration limit");
                    }
                }
            }
            SimStmt::For {
                line,
                init,
                cond,
                post,
                body,
            } => {
                if let Some(b) = init {
                    self.exec(b)?;
                }
                let mut guard: i64 = 10_000_000;
                loop {
                    let go = match cond {
                        Some(c) => self.eval_e(c)? != 0,
                        None => true,
                    };
                    if !go {
                        break;
                    }
                    self.exec_list(body)?;
                    if let Some(p) = post {
                        self.exec(p)?;
                    }
                    guard -= 1;
                    if guard == 0 {
                        return fail(*line, "for loop exceeded iteration limit");
                    }
                }
            }
        }
        Ok(())
    }
}

/// 式を診断メッセージ用の読みやすい文字列へ復元する(`assert` / `expect` 失敗時)。
/// 元ソースの字面ではなく AST からの再構成なので、空白や括弧は正規化される。
fn expr_to_string(e: &Expr) -> String {
    match e {
        Expr::Num { num, .. } => num.to_string(),
        Expr::Time { .. } => "$time".to_string(),
        Expr::Var { name, index, .. } => match index {
            Some(i) => format!("{}[{}]", name, expr_to_string(i)),
            None => name.clone(),
        },
        Expr::Un { op, a, .. } => format!("{}{}", op, expr_to_string(a)),
        Expr::Bin { op, a, b, .. } => {
            format!("({} {} {})", expr_to_string(a), op, expr_to_string(b))
        }
    }
}

/// stdin から空白/改行区切りの整数トークンを 1 つ読む。
/// 先読みは区切り 1 バイトのみなので、scan() を複数回呼んでもトークン境界は保たれる。
/// 整数 `v` を基数 `base` (2/8/10/16) で文字列化する。
/// 負値は `-` 接頭 + 絶対値で表現する(64bit ラップではなく読みやすさ優先)。
/// 16 進は小文字。
fn format_int(v: i64, base: u32) -> String {
    if base == 10 {
        return v.to_string();
    }
    let (sign, mag) = if v < 0 {
        ("-", (v as i128).unsigned_abs())
    } else {
        ("", v as u128)
    };
    let body = match base {
        2 => format!("{:b}", mag),
        8 => format!("{:o}", mag),
        16 => format!("{:x}", mag),
        _ => mag.to_string(),
    };
    format!("{}{}", sign, body)
}

/// 整数 `v` を無符号として基数 `base` (2/8/10/16) で文字列化する。
/// バス var 引数の表示に使う(ニブルパッキング後の値はそのビットパターンを保つため、
/// 符号ビット相当の上位レーンが立っていても `-` を付けず無符号として読む)。
fn format_uint(v: u64, base: u32) -> String {
    match base {
        2 => format!("{:b}", v),
        8 => format!("{:o}", v),
        10 => format!("{}", v),
        16 => format!("{:x}", v),
        _ => v.to_string(),
    }
}

/// `val` を最小幅 `width` に右寄せ整形する。
/// `zero_pad = true` のとき先頭が `-` ならその直後にゼロを挿入し、符号を幅の先頭に保つ
/// (`%04b` of -3 → `-011`)。スペース埋めは符号も含めて単純に左にスペースを足す。
fn pad_value(val: &str, width: usize, zero_pad: bool) -> String {
    let n = val.chars().count();
    if n >= width {
        return val.to_string();
    }
    let need = width - n;
    if zero_pad {
        if let Some(rest) = val.strip_prefix('-') {
            let mut s = String::from("-");
            for _ in 0..need {
                s.push('0');
            }
            s.push_str(rest);
            return s;
        }
        let mut s = String::new();
        for _ in 0..need {
            s.push('0');
        }
        s.push_str(val);
        s
    } else {
        let mut s = String::new();
        for _ in 0..need {
            s.push(' ');
        }
        s.push_str(val);
        s
    }
}

/// `scan(fmt)` の書式文字列を基数 (2 / 8 / 10 / 16) に変換する。
/// `None` または `"%"` は 10 進。`"%b"` / `"%x"` / `"%o"` は対応基数。それ以外はエラー。
fn parse_scan_fmt(line: i32, fmt: Option<&str>) -> RvResult<u32> {
    let Some(f) = fmt else { return Ok(10) };
    match f {
        "%" => Ok(10),
        "%b" => Ok(2),
        "%x" => Ok(16),
        "%o" => Ok(8),
        _ => fail(
            line,
            format!(
                "scan() format must be one of \"%\", \"%b\", \"%x\", \"%o\" (got {:?})",
                f
            ),
        ),
    }
}

fn read_stdin_int(line: i32, base: u32) -> RvResult<i64> {
    use std::io::Read;
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 1];
    let mut s = String::new();
    let is_digit_for_base = |c: char, b: u32| -> bool { c.is_digit(b) };
    loop {
        let n = match stdin.read(&mut buf) {
            Ok(n) => n,
            Err(e) => return fail(line, format!("scan(): failed to read stdin: {}", e)),
        };
        if n == 0 {
            // EOF
            if s.is_empty() {
                return fail(line, "scan(): unexpected end of input (no more integers on stdin)");
            }
            break;
        }
        let c = buf[0] as char;
        if s.is_empty() {
            if c.is_whitespace() {
                continue; // 先頭の空白は読み飛ばす
            }
            if c == '-' || c == '+' || is_digit_for_base(c, base) {
                s.push(c);
            } else {
                return fail(line, format!("scan(): expected an integer but found '{}'", c));
            }
        } else if is_digit_for_base(c, base) {
            s.push(c);
        } else {
            break; // 区切り文字でトークン終了(この 1 バイトは捨てる)
        }
    }
    let parsed = if let Some(rest) = s.strip_prefix('-') {
        i64::from_str_radix(rest, base).map(|v| -v)
    } else if let Some(rest) = s.strip_prefix('+') {
        i64::from_str_radix(rest, base)
    } else {
        i64::from_str_radix(&s, base)
    };
    match parsed {
        Ok(v) => Ok(v),
        Err(_) => fail(line, format!("scan(): invalid integer literal '{}'", s)),
    }
}

/// `run_program` のフェーズ別所要時間(`--time` 用)。
///
/// `sim` は sim tick 実行(`tick_once` = 入力反映 + `step()` 不動点 + 出力反映)に
/// 費やした累積時間。エラボレーションや monitor 出力など tick 以外の処理は含まない。
#[derive(Default, Clone, Copy)]
pub struct RunTimings {
    /// 全 module 合算の sim tick 実行時間。
    pub sim: Duration,
    /// 実行した module 数。
    pub modules: usize,
}

/// `--vcd <path>` で複数 module 時のファイル名を分割する。
/// 単一 module は `path` をそのまま使い、複数なら拡張子直前に `.<module名>` を挿入する
/// (例: `out.vcd` → `out.clock.vcd`)。拡張子が無ければ末尾に `.<module名>` を足す。
fn vcd_path_for(path: &str, module: &str, many: bool) -> String {
    if !many {
        return path.to_string();
    }
    match path.rfind('.') {
        // ディレクトリ区切りより後ろにある '.' のみ拡張子とみなす。
        Some(i) if i > path.rfind(['/', '\\']).map_or(0, |p| p + 1) => {
            format!("{}.{}{}", &path[..i], module, &path[i..])
        }
        _ => format!("{}.{}", path, module),
    }
}

pub fn run_program(prog: &Program, trace: bool, vcd: Option<&str>) -> RvResult<RunTimings> {
    let mut cfg = Config::default();
    if let Some(v) = prog.defines.get("INIT_TIMEOUT") {
        cfg.init_timeout = *v;
    }
    if let Some(v) = prog.defines.get("BURNOUT_LIMIT") {
        cfg.burnout_limit = *v as i32;
    }
    if let Some(v) = prog.defines.get("BURNOUT_WINDOW") {
        cfg.burnout_window = *v as i32;
    }
    if let Some(v) = prog.defines.get("BURNOUT_COOLDOWN") {
        cfg.burnout_cooldown = *v as i32;
    }

    if prog.modules.is_empty() {
        warn(0, "no module to run");
        return Ok(RunTimings::default());
    }
    let many = prog.modules.len() > 1;
    let mut timings = RunTimings {
        sim: Duration::ZERO,
        modules: prog.modules.len(),
    };
    // 自己検証(`assert` / `expect`)は全 module ぶん集計する。途中の失敗で打ち切らず、
    // 各 module の sim を最後まで実行してから合否を判定する(全チェックを見せるため)。
    let mut assert_total = 0i64;
    let mut assert_failed = 0i64;
    for m in &prog.modules {
        // JSON モードでは `=== module X ===` ヘッダを出さない(JSONL の純度を保つ)。
        // 代わりに各 monitor イベントに `"module"` フィールドが乗る。
        if many && !is_json_mode() {
            println!("=== module {} ===", m.name);
        }
        let vcd_sink = match vcd {
            Some(path) => {
                let p = vcd_path_for(path, &m.name, many);
                match Vcd::create(&p, &m.name) {
                    Ok(v) => Some(v),
                    Err(e) => return fail(0, format!("cannot open VCD file '{}': {}", p, e)),
                }
            }
            None => None,
        };
        let mut ex = ModuleExec::new(prog, m, cfg, trace, vcd_sink, many);
        ex.run()?;
        timings.sim += ex.sim_dur;
        assert_total += ex.assert_total;
        assert_failed += ex.assert_failed;
    }
    if assert_total > 0 {
        if is_json_mode() {
            eprintln!(
                "{{\"kind\":\"summary\",\"total\":{},\"failed\":{}}}",
                assert_total, assert_failed
            );
            if assert_failed > 0 {
                // 終了コードは非ゼロにしたいが診断は既に出したので、`line: 0` の簡易エラーで返す。
                // JSON モードでは report_error が同じ kind/msg の JSONL を出す。
                return fail(
                    0,
                    format!("assertions: {} of {} failed", assert_failed, assert_total),
                );
            }
        } else {
            if assert_failed > 0 {
                return fail(
                    0,
                    format!("assertions: {} of {} failed", assert_failed, assert_total),
                );
            }
            eprintln!("[assert] {} assertion(s), all passed", assert_total);
        }
    }
    Ok(timings)
}
