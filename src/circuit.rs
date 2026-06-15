//! redv - circuit graph & simulation engine
//!
//! C++ 版 `circuit.hpp` の移植。意味論(設計判断の確定値)は原実装と一致:
//!   * 1 tick = 1 レッドストーンティック。ゲームティック素子・サブティックパルスは扱わない。
//!   * 各 tick:
//!       1. 順序素子の出力を `delay` tick 前にサンプルした入力から計算
//!       2. 組合せ網(ダスト/ブロック/直結)を MAX 合流で不動点まで解く
//!       3. トーチ焼き切れ検出
//!       4. 順序素子が入力ノードをサンプル(履歴になる)
//!   * ダスト: tick 内即時伝搬、ダスト 1 個につき強度 -1。
//!   * ブロック: 即時、給電(>0)なら 15、さもなくば 0。
//!   * リピータ rn: out(T) = in(T-n) > 0 ? 15 : 0(1tick パルスも保持)
//!   * トーチ:      out(T) = in(T-1) > 0 ? 0 : 15
//!   * コンパレータ(チェーン内・サイド入力なし): out(T) = in(T-1)(強度パススルー)
//!   * 入力変数変更は次 tick 先頭で反映。
//!   * 観測(monitor / 出力変数)は tick 処理後の値を見る。
//!
//! 借用検査の都合上、原実装が参照イテレートしていたループはインデックス走査に変えてある
//! (`CEdge` は `Copy`、各ノード/素子は添字でアクセス)。結果は逐次キュー方式と同一。

use crate::diag::{fail, warn, RvResult};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// #init 定常判定のタイムアウト(tick)
    pub init_timeout: i64,
    /// トーチ焼き切れ: 窓内トグル回数の上限
    pub burnout_limit: i32,
    /// 監視窓(tick)
    pub burnout_window: i32,
    /// 強制 OFF 期間(tick)
    pub burnout_cooldown: i32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            init_timeout: 1000,
            burnout_limit: 8,
            burnout_window: 30,
            burnout_cooldown: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Plain,
    Block,
    Const,
    Input,
}

#[derive(Debug, Clone)]
pub struct CNode {
    pub name: String,
    pub kind: NodeKind,
    /// INPUT / CONST の駆動値
    pub base: i32,
    pub value: i32,
    pub prev: i32,
    pub has_incoming: bool,
    pub is_out_port: bool,
    pub elem_assigned: bool,
    pub is_const_qual: bool,
}

impl CNode {
    fn new(name: String, kind: NodeKind) -> Self {
        CNode {
            name,
            kind,
            base: 0,
            value: 0,
            prev: 0,
            has_incoming: false,
            is_out_port: false,
            elem_assigned: false,
            is_const_qual: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CEdge {
    pub s: usize,
    pub d: usize,
    pub decay: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqKind {
    Rep,
    Torch,
    /// コンパレータ(比較モード): out = back >= side ? back : 0
    CompCmp,
    /// コンパレータ(減算モード): out = max(0, back - side)
    CompSub,
}

#[derive(Debug, Clone)]
pub struct CSeq {
    pub kind: SeqKind,
    pub delay: i32,
    pub in_: usize,
    pub out: usize,
    /// サンプルした(後ろ)入力。front = 最古
    pub hist: VecDeque<i32>,
    /// コンパレータの横入力ノード。None なら横入力なし(= side 0)
    pub side_in: Option<usize>,
    /// サンプルした横入力。`side_in` がある時のみ使う。front = 最古
    pub side_hist: VecDeque<i32>,
    pub outv: i32,
    pub prev_out: i32,
    /// トグルした tick の履歴(トーチ)
    pub togg: VecDeque<i64>,
    /// 焼き切れクールダウン残
    pub cooldown: i32,
    pub label: String,
}

#[derive(Debug, Default)]
pub struct Circuit {
    pub cfg: Config,
    pub nodes: Vec<CNode>,
    /// union-find 親(別名併合)
    pub par: Vec<usize>,
    pub edges: Vec<CEdge>,
    pub seqs: Vec<CSeq>,
    pub tick: i64,
    pub trace: bool,
}

impl Circuit {
    pub fn new(cfg: Config, trace: bool) -> Self {
        Circuit {
            cfg,
            trace,
            ..Default::default()
        }
    }

    pub fn new_node(&mut self, nm: impl Into<String>, kind: NodeKind) -> usize {
        self.nodes.push(CNode::new(nm.into(), kind));
        let idx = self.nodes.len() - 1;
        self.par.push(idx);
        idx
    }

    pub fn find(&mut self, mut x: usize) -> usize {
        while self.par[x] != x {
            self.par[x] = self.par[self.par[x]];
            x = self.par[x];
        }
        x
    }

    pub fn merge(&mut self, a: usize, b: usize, line: i32) -> RvResult<()> {
        let a = self.find(a);
        let b = self.find(b);
        if a == b {
            return Ok(());
        }
        let rank = |k: NodeKind| match k {
            NodeKind::Const => 3,
            NodeKind::Input => 2,
            NodeKind::Block => 1,
            NodeKind::Plain => 0,
        };
        // 競合する const reg の併合はエラー
        if self.nodes[a].kind == NodeKind::Const
            && self.nodes[b].kind == NodeKind::Const
            && self.nodes[a].base != self.nodes[b].base
        {
            return fail(
                line,
                format!(
                    "conflicting const regs aliased: {} / {}",
                    self.nodes[a].name, self.nodes[b].name
                ),
            );
        }
        let (b_kind, b_base, b_inc, b_out, b_elem, b_cq) = {
            let nb = &self.nodes[b];
            (
                nb.kind,
                nb.base,
                nb.has_incoming,
                nb.is_out_port,
                nb.elem_assigned,
                nb.is_const_qual,
            )
        };
        let na = &mut self.nodes[a];
        if rank(b_kind) > rank(na.kind) {
            na.kind = b_kind;
            na.base = b_base;
        }
        na.has_incoming = na.has_incoming || b_inc;
        na.is_out_port = na.is_out_port || b_out;
        na.elem_assigned = na.elem_assigned || b_elem;
        na.is_const_qual = na.is_const_qual || b_cq;
        self.par[b] = a;
        Ok(())
    }

    pub fn add_edge(&mut self, s: usize, d: usize, decay: i32) {
        self.edges.push(CEdge { s, d, decay });
        let r = self.find(d);
        self.nodes[r].has_incoming = true;
    }

    pub fn add_seq(&mut self, kind: SeqKind, delay: i32, in_: usize, out: usize, label: String) {
        let mut hist = VecDeque::new();
        for _ in 0..delay {
            hist.push_back(0);
        }
        self.seqs.push(CSeq {
            kind,
            delay,
            in_,
            out,
            hist,
            side_in: None,
            side_hist: VecDeque::new(),
            outv: 0,
            prev_out: 0,
            togg: VecDeque::new(),
            cooldown: 0,
            label,
        });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
    }

    /// コンパレータ素子(遅延 1 tick)。`side_in` が None なら横入力 0(= パススルー)。
    pub fn add_comp(
        &mut self,
        kind: SeqKind,
        in_: usize,
        side_in: Option<usize>,
        out: usize,
        label: String,
    ) {
        let mut hist = VecDeque::new();
        hist.push_back(0);
        let mut side_hist = VecDeque::new();
        if side_in.is_some() {
            side_hist.push_back(0);
        }
        self.seqs.push(CSeq {
            kind,
            delay: 1,
            in_,
            out,
            hist,
            side_in,
            side_hist,
            outv: 0,
            prev_out: 0,
            togg: VecDeque::new(),
            cooldown: 0,
            label,
        });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
    }

    fn seq_out_of(kind: SeqKind, back: i32, side: i32) -> i32 {
        match kind {
            SeqKind::Rep => {
                if back > 0 {
                    15
                } else {
                    0
                }
            }
            SeqKind::Torch => {
                if back > 0 {
                    0
                } else {
                    15
                }
            }
            // 横入力なし(side=0)なら両モードとも back のパススルーに退化する
            SeqKind::CompCmp => {
                if back >= side {
                    back
                } else {
                    0
                }
            }
            SeqKind::CompSub => (back - side).max(0),
        }
    }

    fn contribute(&mut self, n: usize, v: i32, ch: &mut bool) {
        let nd = &mut self.nodes[n];
        match nd.kind {
            NodeKind::Const | NodeKind::Input => {}
            NodeKind::Block => {
                if v > 0 && nd.value < 15 {
                    nd.value = 15;
                    *ch = true;
                }
            }
            NodeKind::Plain => {
                if v > nd.value {
                    nd.value = v;
                    *ch = true;
                }
            }
        }
    }

    pub fn set_input(&mut self, n: usize, v: i32) {
        let r = self.find(n);
        self.nodes[r].base = v;
    }

    pub fn read(&mut self, n: usize) -> i32 {
        let r = self.find(n);
        self.nodes[r].value
    }

    /// 1 レッドストーンティック進める。何か変化した(またはする)なら true。
    pub fn step(&mut self) -> bool {
        self.tick += 1;
        let tick = self.tick;
        let bwin = self.cfg.burnout_window;
        let blim = self.cfg.burnout_limit;
        let bcool = self.cfg.burnout_cooldown;

        // 1. 履歴から順序素子出力
        for i in 0..self.seqs.len() {
            let kind = self.seqs[i].kind;
            let front = *self.seqs[i].hist.front().unwrap_or(&0);
            let side_front = *self.seqs[i].side_hist.front().unwrap_or(&0);
            let mut o = Self::seq_out_of(kind, front, side_front);
            if kind == SeqKind::Torch && self.seqs[i].cooldown > 0 {
                o = 0;
                self.seqs[i].cooldown -= 1;
            }
            self.seqs[i].outv = o;
        }

        // 2. 組合せ網の不動点(単調・MAX 合流)
        for n in 0..self.nodes.len() {
            if self.find(n) != n {
                continue;
            }
            let nd = &mut self.nodes[n];
            nd.value = match nd.kind {
                NodeKind::Const | NodeKind::Input => nd.base,
                _ => 0,
            };
        }
        {
            let mut dummy = false;
            for i in 0..self.seqs.len() {
                let out = self.find(self.seqs[i].out);
                let outv = self.seqs[i].outv;
                self.contribute(out, outv, &mut dummy);
            }
        }
        let mut ch = true;
        let mut guard: i64 = 16 * (self.edges.len() as i64 + self.seqs.len() as i64) + 64;
        while ch {
            ch = false;
            for ei in 0..self.edges.len() {
                let e = self.edges[ei];
                let s = self.find(e.s);
                let d = self.find(e.d);
                let mut v = self.nodes[s].value - e.decay;
                if v < 0 {
                    v = 0;
                }
                self.contribute(d, v, &mut ch);
            }
            guard -= 1;
            if guard <= 0 {
                break;
            }
        }

        // 3. トーチ焼き切れ + 出力変化追跡
        let mut changed = false;
        for i in 0..self.seqs.len() {
            if self.seqs[i].kind == SeqKind::Torch && self.seqs[i].outv != self.seqs[i].prev_out {
                self.seqs[i].togg.push_back(tick);
                while !self.seqs[i].togg.is_empty()
                    && *self.seqs[i].togg.front().unwrap() <= tick - bwin as i64
                {
                    self.seqs[i].togg.pop_front();
                }
                if self.seqs[i].togg.len() as i32 > blim && self.seqs[i].cooldown == 0 {
                    warn(
                        0,
                        format!(
                            "torch burnout: {} toggled {} times within {} ticks (tick {}); \
                             forced OFF for {} ticks",
                            self.seqs[i].label,
                            self.seqs[i].togg.len(),
                            bwin,
                            tick,
                            bcool
                        ),
                    );
                    self.seqs[i].cooldown = bcool;
                    self.seqs[i].togg.clear();
                }
            }
            if self.seqs[i].outv != self.seqs[i].prev_out {
                changed = true;
            }
            if self.seqs[i].cooldown > 0 {
                changed = true;
            }
            self.seqs[i].prev_out = self.seqs[i].outv;
        }

        // 4. 順序素子の入力サンプリング
        for i in 0..self.seqs.len() {
            let in_node = self.find(self.seqs[i].in_);
            let v = self.nodes[in_node].value;
            self.seqs[i].hist.push_back(v);
            self.seqs[i].hist.pop_front();
            if let Some(sn) = self.seqs[i].side_in {
                let side_node = self.find(sn);
                let sv = self.nodes[side_node].value;
                self.seqs[i].side_hist.push_back(sv);
                self.seqs[i].side_hist.pop_front();
            }
            let back = *self.seqs[i].hist.back().unwrap();
            let side = *self.seqs[i].side_hist.back().unwrap_or(&0);
            let mut uniform = true;
            for &h in &self.seqs[i].hist {
                if h != back {
                    uniform = false;
                    break;
                }
            }
            for &h in &self.seqs[i].side_hist {
                if h != side {
                    uniform = false;
                    break;
                }
            }
            if !uniform || self.seqs[i].outv != Self::seq_out_of(self.seqs[i].kind, back, side) {
                changed = true;
            }
        }

        for n in 0..self.nodes.len() {
            if self.find(n) != n {
                continue;
            }
            if self.nodes[n].value != self.nodes[n].prev {
                changed = true;
            }
            self.nodes[n].prev = self.nodes[n].value;
        }

        if self.trace {
            self.dump_trace();
        }
        changed
    }

    fn dump_trace(&mut self) {
        let mut line = format!("[tick {}]", self.tick);
        for n in 0..self.nodes.len() {
            if self.find(n) != n {
                continue;
            }
            let nd = &self.nodes[n];
            if nd.name.is_empty() || nd.name.contains('#') {
                continue;
            }
            line.push_str(&format!(" {}={}", nd.name, nd.value));
        }
        eprintln!("{}", line);
    }
}
