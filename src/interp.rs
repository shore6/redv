//! redv - elaboration & module/sim interpreter
//!
//! C++ 版 `interp.hpp` の移植。logic 定義を回路グラフへエラボレートし、module の sim を実行する。
//! `?monitor` は sim 開始時にホイストされ、各ウェイト完了直後に発火する(Verilog $monitor 風)。
//!
//! 借用検査の都合上、`insts` / `out_bind` を走査しつつ回路を書き換える箇所は、
//! 必要な値を一旦ローカルへ集めてから適用する(結果は原実装と同一)。

use crate::ast::*;
use crate::circuit::{Circuit, Config, NodeKind, SeqKind};
use crate::diag::{fail, warn, RvResult};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

// ---- chain token expansion ---------------------------------------------

/// チェーンの中間チャンク列を Elem 列へ展開する。
/// 各トークンは「素子チャンク(`d4ccd4` 等)」または「wire 名(再利用素子列)」。
/// wire 名は `wire_seq` を引いて再帰展開する(`visited` で循環を検出)。
/// reg / ポート名は中間に置けない(端点専用)ためエラー。
#[allow(clippy::too_many_arguments)]
fn expand_chain_tokens(
    tokens: &[String],
    wire_seq: &HashMap<String, (i32, Vec<String>)>,
    scope: &HashMap<String, usize>,
    wire_names: &HashSet<String>,
    line: i32,
    visited: &mut Vec<String>,
    out: &mut Vec<Elem>,
) -> RvResult<()> {
    for tok in tokens {
        if scope.contains_key(tok) {
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
            visited.push(tok.clone());
            expand_chain_tokens(seq, wire_seq, scope, wire_names, line, visited, out)?;
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
            if i < b.len() && b[i] == b'x' {
                out.push(Elem { k: 'x', n: 1, line });
                i += 1;
            } else if i < b.len() && b[i].is_ascii_digit() {
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
                if !(1..=4).contains(&n) || (i < b.len() && b[i].is_ascii_digit()) {
                    return fail(line, format!("repeater delay must be 1-4 in \"{}\"", s));
                }
            }
            out.push(Elem { k: 'r', n, line });
        } else if c == b't' {
            out.push(Elem { k: 't', n: 1, line });
            i += 1;
        } else if c == b'b' {
            out.push(Elem { k: 'b', n: 1, line });
            i += 1;
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

/// 名前全体が素子列として解釈できる(= 素子名と衝突する)なら true。
/// reg / wire / ポート名がこれに該当するのは禁止する: チェーン内に置いたとき
/// 「名前付きの点」と「素子列」が曖昧になり、回路が読みにくくバグの温床になる。
/// 例: `b`(ブロック)/ `r`(リピータ)/ `cd`(コンパレータ)/ `tb`(トーチ+ブロック)。
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

// ---- logic instance ----------------------------------------------------

#[derive(Debug, Clone)]
pub struct Instance {
    pub in_vars: Vec<String>,
    pub in_nodes: Vec<usize>,
    pub out_ports: Vec<(String, usize)>,
}

struct Elaborator<'c, 'p> {
    c: &'c mut Circuit,
    /// 階層インスタンス化の解決に使う logic 定義表
    logics: &'p HashMap<String, LogicDef>,
    /// 現在エラボレート中の logic 名(再帰インスタンス化検出用)
    stack: Vec<String>,
    /// サブインスタンスのノード名を一意化する連番
    counter: usize,
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
        if qual == Qual::Const {
            if strength < 0 {
                return fail(line, "const reg requires a signal strength (e.g. 15b)");
            }
            if strength > 15 {
                return fail(
                    line,
                    format!("const signal strength out of range 0-15: {}", strength),
                );
            }
            let nd = &mut self.c.nodes[root];
            nd.kind = NodeKind::Const;
            nd.base = strength;
            nd.is_const_qual = true;
        } else {
            if strength >= 0 {
                return fail(line, "signal-strength literals are only allowed on const reg");
            }
            self.c.nodes[root].kind = if e.k == 'b' {
                NodeKind::Block
            } else {
                NodeKind::Plain
            };
        }
        self.c.nodes[root].elem_assigned = true;
        Ok(())
    }

    /// sim から呼ばれる最上位エラボレート。入力ポートは sim 変数で駆動する
    /// `Input` ノードになる。
    fn build(
        &mut self,
        l: &'p LogicDef,
        prefix: &str,
        arg_vars: &[String],
        call_line: i32,
    ) -> RvResult<Instance> {
        let (in_nodes, outs) = self.elaborate(l, prefix, true, call_line, arg_vars.len())?;
        Ok(Instance {
            in_vars: arg_vars.to_vec(),
            in_nodes,
            out_ports: outs,
        })
    }

    /// logic 定義 1 つを回路ノード群へ展開する。
    ///
    /// - `top_level == true` … 入力ポートは sim 変数で駆動する `Input` ノード。
    /// - `top_level == false` … 階層インスタンス。入力ポートは親ノードから駆動される
    ///   `Plain` ノードで、結線は呼び出し側が行う。
    ///
    /// 返り値は (ポート順の入力ノード列, 出力ポート列)。
    fn elaborate(
        &mut self,
        l: &'p LogicDef,
        prefix: &str,
        top_level: bool,
        call_line: i32,
        arg_count: usize,
    ) -> RvResult<(Vec<usize>, Vec<(String, usize)>)> {
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

        let mut scope: HashMap<String, usize> = HashMap::new();
        let mut qual_of: HashMap<String, Qual> = HashMap::new();
        // コンパレータ / ロック付きリピーター reg の (back, side) ノード。scope[name] は out ノード。
        let mut side_regs: HashMap<String, (usize, usize)> = HashMap::new();
        let mut wire_names: HashSet<String> = HashSet::new();
        let mut in_order: Vec<String> = Vec::new();
        let mut outs: Vec<(String, usize)> = Vec::new();
        // 階層インスタンス文(全ノード確定後にまとめて結線する)
        let mut instances: Vec<(i32, String, String, Vec<String>)> = Vec::new();
        // 無名チェーン文(wire 名を介さない直結。全ノード確定後にまとめて構築)
        let mut anon_chains: Vec<(i32, String, bool, String, bool, Vec<String>)> = Vec::new();

        for p in &l.ports {
            if scope.contains_key(&p.name) {
                return fail(p.line, format!("duplicate port name: {}", p.name));
            }
            let kind = if p.input {
                if top_level {
                    NodeKind::Input
                } else {
                    NodeKind::Plain
                }
            } else {
                NodeKind::Plain
            };
            let id = self.c.new_node(format!("{}.{}", prefix, p.name), kind);
            if !p.input {
                self.c.nodes[id].is_out_port = true;
            }
            scope.insert(p.name.clone(), id);
            if p.input {
                in_order.push(p.name.clone());
            } else {
                outs.push((p.name.clone(), id));
            }
        }
        if arg_count != in_order.len() {
            return fail(
                call_line,
                format!(
                    "{}: expected {} input argument(s), got {}",
                    l.name,
                    in_order.len(),
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
                    output,
                    callee,
                    args,
                } => {
                    instances.push((*line, output.clone(), callee.clone(), args.clone()));
                }
                LogicStmt::DeclWire { line, names } => {
                    for n in names {
                        if scope.contains_key(n) || wire_names.contains(n) {
                            return fail(*line, format!("duplicate name: {}", n));
                        }
                        wire_names.insert(n.clone());
                    }
                }
                LogicStmt::DeclReg {
                    line,
                    name,
                    qual,
                    init,
                } => {
                    if scope.contains_key(name) || wire_names.contains(name) {
                        return fail(*line, format!("duplicate name: {}", name));
                    }
                    // コンパレータ reg(`reg r = cd;` / `cc`)は back/side/out の 3 ノード束。
                    let comp_kind = match init {
                        Some(ri) => comparator_mode(&ri.tok, *line)?,
                        None => None,
                    };
                    // ロック付きリピーター reg(`reg m = r;` / `r1`-`r4`)も同じ 3 ノード束。
                    let rep_delay = if comp_kind.is_none() {
                        match init {
                            Some(ri) => repeater_delay(&ri.tok, *line)?,
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
                        let back = self
                            .c
                            .new_node(format!("{}.{}#back", prefix, name), NodeKind::Plain);
                        let side = self
                            .c
                            .new_node(format!("{}.{}#side", prefix, name), NodeKind::Plain);
                        let out = self.c.new_node(format!("{}.{}", prefix, name), NodeKind::Plain);
                        self.c
                            .add_rep_lock(dly, back, side, out, format!("{}.{}", prefix, name));
                        scope.insert(name.clone(), out);
                        side_regs.insert(name.clone(), (back, side));
                        qual_of.insert(name.clone(), Qual::Plain);
                    } else {
                        let id = self.c.new_node(format!("{}.{}", prefix, name), NodeKind::Plain);
                        scope.insert(name.clone(), id);
                        qual_of.insert(name.clone(), *qual);
                        match init {
                            Some(ri) => {
                                self.apply_elem(id, name, &ri.tok, ri.strength, *qual, *line)?
                            }
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
                    from_side,
                    to,
                    to_side,
                    chunks,
                } => {
                    anon_chains.push((
                        *line,
                        from.clone(),
                        *from_side,
                        to.clone(),
                        *to_side,
                        chunks.clone(),
                    ));
                }
            }
        }

        // wire 素子列定義の早期検証(未使用 wire も対象)。各トークンは素子チャンク
        // または他 wire 名でなければならない。reg/ポート名(= 旧 named-wire 接続形の
        // 名残)は素子列に置けない。素子展開・循環検出はチェーン構築時に遅延する。
        for (wn, (def_line, tokens)) in &wire_seq {
            for tok in tokens {
                if scope.contains_key(tok) {
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
        // (wire は素子列定義であって接続ではない)。
        // label は内部ノード名に使う識別子('#chN' で trace 非表示)。
        let mut jobs: Vec<(String, i32, String, bool, String, bool, Vec<String>)> = Vec::new();
        for (k, (line, from, from_side, to, to_side, chunks)) in anon_chains.iter().enumerate() {
            jobs.push((
                format!("#ch{}", k + 1),
                *line,
                from.clone(),
                *from_side,
                to.clone(),
                *to_side,
                chunks.clone(),
            ));
        }

        for (wn, line, from, from_side, to, to_side, chunks) in &jobs {
            let (line, from_side, to_side) = (*line, *from_side, *to_side);
            // 始端(信号源)。`.side` は入力専用なので源にはできない。
            if from_side {
                return fail(
                    line,
                    format!("'{}.side' cannot be a wire source (side is an input terminal)", from),
                );
            }
            let fi = match scope.get(from) {
                Some(n) => *n,
                None if wire_names.contains(from) => {
                    return fail(
                        line,
                        format!(
                            "wire '{}' cannot be a chain endpoint (a wire is an element sequence; \
                             endpoints must be reg/port)",
                            from
                        ),
                    )
                }
                None => return fail(line, format!("unknown chain endpoint: {}", from)),
            };
            // 終端(信号先)。コンパレータ / リピーター reg は `.side`=横入力 / 無印=後ろ入力。
            let ti = if to_side {
                match side_regs.get(to) {
                    Some((_back, side)) => *side,
                    None => {
                        return fail(
                            line,
                            format!(
                                "'.side' is only valid on a comparator/repeater reg, but '{}' is not",
                                to
                            ),
                        )
                    }
                }
            } else if let Some((back, _side)) = side_regs.get(to) {
                *back
            } else {
                match scope.get(to) {
                    Some(n) => *n,
                    None if wire_names.contains(to) => {
                        return fail(
                            line,
                            format!(
                                "wire '{}' cannot be a chain endpoint (a wire is an element \
                                 sequence; endpoints must be reg/port)",
                                to
                            ),
                        )
                    }
                    None => return fail(line, format!("unknown chain endpoint: {}", to)),
                }
            };
            // チャンクを Elem 列へ展開。wire 名は素子列として再帰展開する。
            let mut es: Vec<Elem> = Vec::new();
            let mut visited: Vec<String> = Vec::new();
            expand_chain_tokens(
                chunks,
                &wire_seq,
                &scope,
                &wire_names,
                line,
                &mut visited,
                &mut es,
            )?;
            let mut prev = fi;
            let mut decay = 0;
            let mut idx = 0;
            for e in &es {
                idx += 1;
                match e.k {
                    'd' | 'x' => decay += 1,
                    'b' => {
                        let nn = self
                            .c
                            .new_node(format!("{}.{}#b{}", prefix, wn, idx), NodeKind::Block);
                        self.c.add_edge(prev, nn, decay);
                        prev = nn;
                        decay = 0;
                    }
                    'r' | 't' => {
                        let ni = self
                            .c
                            .new_node(format!("{}.{}#i{}", prefix, wn, idx), NodeKind::Plain);
                        let no = self
                            .c
                            .new_node(format!("{}.{}#o{}", prefix, wn, idx), NodeKind::Plain);
                        self.c.add_edge(prev, ni, decay);
                        let kind = if e.k == 'r' {
                            SeqKind::Rep
                        } else {
                            SeqKind::Torch
                        };
                        let dly = if e.k == 'r' { e.n } else { 1 };
                        self.c.add_seq(
                            kind,
                            dly,
                            ni,
                            no,
                            format!("{}.{}[{}]", prefix, wn, idx),
                        );
                        prev = no;
                        decay = 0;
                    }
                    // インライン(チェーン内)コンパレータ: 横入力なし = パススルー
                    'C' | 'S' => {
                        let ni = self
                            .c
                            .new_node(format!("{}.{}#i{}", prefix, wn, idx), NodeKind::Plain);
                        let no = self
                            .c
                            .new_node(format!("{}.{}#o{}", prefix, wn, idx), NodeKind::Plain);
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
                            format!("{}.{}[{}]", prefix, wn, idx),
                        );
                        prev = no;
                        decay = 0;
                    }
                    _ => {}
                }
            }
            self.c.add_edge(prev, ti, decay);
        }

        // 階層インスタンスの結線(親ノード <-> サブ logic のポート)
        for (line, output, callee, args) in &instances {
            let sub = match self.logics.get(callee) {
                Some(s) => s,
                None => return fail(*line, format!("unknown logic: {}", callee)),
            };
            // 親側の結線端点を解決(reg / ポートのみ。wire は不可)
            if wire_names.contains(output) {
                return fail(
                    *line,
                    format!("logic instance output '{}' must be a reg/port, not a wire", output),
                );
            }
            let out_node = match scope.get(output) {
                Some(n) => *n,
                None => return fail(*line, format!("unknown instance output target: {}", output)),
            };
            let mut arg_nodes: Vec<usize> = Vec::new();
            for a in args {
                if wire_names.contains(a) {
                    return fail(
                        *line,
                        format!("logic instance argument '{}' must be a reg/port, not a wire", a),
                    );
                }
                match scope.get(a) {
                    Some(n) => arg_nodes.push(*n),
                    None => {
                        return fail(*line, format!("unknown logic instance argument: {}", a))
                    }
                }
            }
            // サブ logic を展開(入力ポートは Plain ノードになる)
            self.counter += 1;
            let sub_prefix = format!("{}/{}#{}", prefix, callee, self.counter);
            let (sub_in, sub_out) =
                self.elaborate(sub, &sub_prefix, false, *line, args.len())?;
            if sub_out.is_empty() {
                return fail(*line, format!("{} has no output port to bind", callee));
            }
            if sub_out.len() > 1 {
                return fail(
                    *line,
                    format!(
                        "{} has multiple output ports; the binding form 'out = logic(...)' supports exactly one",
                        callee
                    ),
                );
            }
            // 親の引数ノード -> サブ入力ポート(減衰なし直結)
            for (pa, si) in arg_nodes.iter().zip(sub_in.iter()) {
                self.c.add_edge(*pa, *si, 0);
            }
            // サブ出力ポート -> 親の出力先(減衰なし直結)
            self.c.add_edge(sub_out[0].1, out_node, 0);
        }

        // 未接続 output ポート検査(仕様: エラー)
        for (name, node) in &outs {
            let r = self.c.find(*node);
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

        self.stack.pop();
        let in_nodes = in_order.iter().map(|n| scope[n]).collect();
        Ok((in_nodes, outs))
    }
}

// ---- module execution --------------------------------------------------

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
}

impl<'a> ModuleExec<'a> {
    pub fn new(prog: &'a Program, m: &'a ModuleDef, cfg: Config, trace: bool) -> Self {
        ModuleExec {
            prog,
            m,
            c: Circuit::new(cfg, trace),
            vars: HashMap::new(),
            insts: BTreeMap::new(),
            out_bind: BTreeMap::new(),
            mons: Vec::new(),
            clamp_warned: HashSet::new(),
            sim_time: 0,
        }
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
            Expr::Var { line, name } => match self.vars.get(name) {
                Some(v) => Ok(*v),
                None => fail(*line, format!("undeclared variable: {}", name)),
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
        self.apply_inputs()?;
        let ch = self.c.step();
        self.apply_outputs();
        Ok(ch)
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

    fn do_monitor(&self, line: i32, call: &CallData) -> RvResult<()> {
        if !call.has_fmt {
            return fail(line, "monitor() requires a format string as its first argument");
        }
        let mut av: Vec<i64> = Vec::new();
        for a in &call.args {
            av.push(self.eval_e(a)?);
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
                let mut width = 0usize;
                while j < f.len() && f[j].is_ascii_digit() {
                    width = width * 10 + (f[j] as usize - '0' as usize);
                    j += 1;
                }
                if j < f.len() && (f[j] == 't' || f[j] == 'd') {
                    j += 1;
                }
                let mut v = if ai < av.len() {
                    let s = av[ai].to_string();
                    ai += 1;
                    s
                } else {
                    "<?>".to_string()
                };
                while v.chars().count() < width {
                    v.insert(0, ' ');
                }
                out.push_str(&v);
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

    fn fire_monitors(&mut self) -> RvResult<()> {
        let mons = self.mons.clone();
        for (line, call) in mons {
            self.do_monitor(line, call)?;
        }
        Ok(())
    }

    /// `x = scan();` — stdin から空白/改行区切りの整数を 1 つ読み、変数に代入する。
    /// EOF・非数値はエラー(厳しめ診断)。
    fn do_scan(&mut self, line: i32, target: &str, bind_args: &[String]) -> RvResult<()> {
        if !bind_args.is_empty() {
            return fail(line, "scan() takes no arguments");
        }
        if !self.vars.contains_key(target) {
            return fail(line, format!("undeclared variable: {}", target));
        }
        let v = read_stdin_int(line)?;
        self.vars.insert(target.to_string(), v);
        Ok(())
    }

    fn do_call_bind(
        &mut self,
        line: i32,
        target: &str,
        callee: &str,
        bind_args: &[String],
    ) -> RvResult<()> {
        let prog: &'a Program = self.prog;
        let lit = match prog.logics.get(callee) {
            Some(l) => l,
            None => return fail(line, format!("unknown logic: {}", callee)),
        };
        let key = format!("{}({})", callee, bind_args.join(","));
        if !self.insts.contains_key(&key) {
            for a in bind_args {
                if !self.vars.contains_key(a) {
                    return fail(line, format!("undeclared variable passed to logic: {}", a));
                }
            }
            let inst = {
                let mut el = Elaborator {
                    c: &mut self.c,
                    logics: &prog.logics,
                    stack: Vec::new(),
                    counter: 0,
                };
                el.build(lit, &key, bind_args, line)?
            };
            self.insts.insert(key.clone(), inst);
        }
        let (out_node, ports_len) = {
            let inst = self.insts.get(&key).unwrap();
            (inst.out_ports.first().map(|p| p.1), inst.out_ports.len())
        };
        if ports_len == 0 {
            return fail(line, format!("{} has no output port to bind", callee));
        }
        if ports_len > 1 {
            return fail(
                line,
                format!(
                    "{} has multiple output ports; the binding form 'v = logic(...)' supports exactly one",
                    callee
                ),
            );
        }
        if !self.vars.contains_key(target) {
            return fail(line, format!("undeclared variable: {}", target));
        }
        let node = out_node.unwrap();
        self.out_bind.insert(target.to_string(), node);
        let val = self.c.read(node) as i64;
        self.vars.insert(target.to_string(), val);
        Ok(())
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
                for (name, e) in decls {
                    if self.vars.contains_key(name) {
                        return fail(*line, format!("duplicate variable: {}", name));
                    }
                    let v = match e {
                        Some(ex) => self.eval_e(ex)?,
                        None => 0,
                    };
                    self.vars.insert(name.clone(), v);
                }
            }
            SimStmt::Assign { line, target, value } => {
                if !self.vars.contains_key(target) {
                    return fail(*line, format!("undeclared variable: {}", target));
                }
                let v = self.eval_e(value)?;
                self.vars.insert(target.clone(), v);
            }
            SimStmt::CallBind {
                line,
                target,
                callee,
                bind_args,
            } => {
                if callee == "scan" {
                    self.do_scan(*line, target, bind_args)?;
                } else {
                    self.do_call_bind(*line, target, callee, bind_args)?;
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

/// stdin から空白/改行区切りの整数トークンを 1 つ読む。
/// 先読みは区切り 1 バイトのみなので、scan() を複数回呼んでもトークン境界は保たれる。
fn read_stdin_int(line: i32) -> RvResult<i64> {
    use std::io::Read;
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 1];
    let mut s = String::new();
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
            if c == '-' || c == '+' || c.is_ascii_digit() {
                s.push(c);
            } else {
                return fail(line, format!("scan(): expected an integer but found '{}'", c));
            }
        } else if c.is_ascii_digit() {
            s.push(c);
        } else {
            break; // 区切り文字でトークン終了(この 1 バイトは捨てる)
        }
    }
    match s.parse::<i64>() {
        Ok(v) => Ok(v),
        Err(_) => fail(line, format!("scan(): invalid integer literal '{}'", s)),
    }
}

pub fn run_program(prog: &Program, trace: bool) -> RvResult<()> {
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
        return Ok(());
    }
    let many = prog.modules.len() > 1;
    for m in &prog.modules {
        if many {
            println!("=== module {} ===", m.name);
        }
        let mut ex = ModuleExec::new(prog, m, cfg, trace);
        ex.run()?;
    }
    Ok(())
}
