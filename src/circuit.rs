//! redv - circuit graph & simulation engine
//!
//! 意味論(設計判断の確定値):
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
//! 借用検査の都合上、ループは参照イテレートでなくインデックス走査にしてある
//! (`CEdge` は `Copy`、各ノード/素子は添字でアクセス)。結果は逐次キュー方式と同一。

use crate::diag::{fail, warn, RvResult};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};

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
    /// このノードがどこかを駆動しているか(エッジの源・順序素子の後ろ/横入力・
    /// 0tick リピータの入力のいずれか)。lint の浮きノード検出に使う(issue #48)。
    pub has_outgoing: bool,
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
            has_outgoing: false,
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

/// オブザーバのエッジ判定モード(issue #58)。判定式以外(1tick パルス・強度 15・
/// 履歴 2 段・インラインチェーン専用)は全モード共通。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObsMode {
    /// `o`: 変化全部。立ち上がり・立ち下がり・強度内変化(5→10 等)を拾う。
    Any,
    /// `op`: 立ち上がりのみ(posedge)。`prev == 0 && cur > 0`。
    Rise,
    /// `on`: 立ち下がりのみ(negedge)。`prev > 0 && cur == 0`。
    Fall,
    /// `oe`: 2値エッジ。0↔正 のトグルだけ拾い、強度内変化は無視。`(prev>0) != (cur>0)`。
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqKind {
    Rep,
    Torch,
    /// コンパレータ(比較モード): out = back >= side ? back : 0
    CompCmp,
    /// コンパレータ(減算モード): out = max(0, back - side)
    CompSub,
    /// オブザーバ(変化検出): 隣接 2 サンプルがモードの判定式を満たせば
    /// 1tick・強度 15 のパルス。履歴 2 段(delay=2)に乗り、既定 `Any` は
    /// `out(T) = in(T-2) != in(T-1) ? 15 : 0`(エッジ亜種は `ObsMode`)。
    /// 出力は `(back,side)` の純関数ではない(前後 2 サンプルの比較)ので、
    /// `seq_out_of` ではなく `observer_out` で step() から直接計算する。
    Observer(ObsMode),
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
    /// この素子を生成したソース行。lint(常時 ON トーチ等)の報告位置に使う(issue #48)。
    pub line: i32,
}

/// 0tick リピータ(`r0`)。遅延ゼロの組合せ増幅器: `out = in > 0 ? 15 : 0` を
/// **同一 tick** で確定する。入力について単調(in 増→out 増)なので、組合せ網の
/// MAX 不動点ループにそのまま参加でき、決定性・順序非依存を保てる。
#[derive(Debug, Clone)]
pub struct ZeroRep {
    pub in_: usize,
    pub out: usize,
}

/// VCD(Value Change Dump)波形出力。`--vcd <path>` で有効化される。
///
/// 公開ノード(`dump_trace` と同じく名前が空でなく `#` を含まないルートノード)を
/// 4bit ベクタ信号として書き出す。時刻は生 tick(`Circuit::tick`、`#init` 整定
/// ティックも含む。`-t` トレースと一致)。値変化のみを各 `#<tick>` 節に出す。
#[derive(Debug)]
pub struct Vcd {
    w: BufWriter<File>,
    /// `$scope module <scope> $end` に使う名前(= module 名)。
    scope: String,
    /// ヘッダ確定後に固定する信号表。`started` が true になるまで空。
    sigs: Vec<VcdSig>,
    /// ヘッダ + 初期 `$dumpvars` を書き終えたか。
    started: bool,
}

/// VCD 信号 1 本。`node` はルートノード添字、`code` は VCD 識別子、`last` は直近出力値。
#[derive(Debug)]
struct VcdSig {
    node: usize,
    code: String,
    last: i32,
}

impl Vcd {
    /// 出力ファイルを作成して VCD ライタを用意する。ヘッダはまだ書かない
    /// (信号名はエラボレーション後にしか分からないため、最初の dump で遅延出力)。
    pub fn create(path: &str, scope: &str) -> std::io::Result<Vcd> {
        let f = File::create(path)?;
        Ok(Vcd {
            w: BufWriter::new(f),
            scope: scope.to_string(),
            sigs: Vec::new(),
            started: false,
        })
    }

    /// VCD 識別子(印字可能 ASCII 33..=126 の base-94)。i ごとに一意。
    fn id(i: usize) -> String {
        let mut n = i;
        let mut s = String::new();
        loop {
            s.push((33u8 + (n % 94) as u8) as char);
            n /= 94;
            if n == 0 {
                break;
            }
            n -= 1;
        }
        s
    }
}

#[derive(Debug, Default)]
pub struct Circuit {
    pub cfg: Config,
    pub nodes: Vec<CNode>,
    /// union-find 親(別名併合)
    pub par: Vec<usize>,
    pub edges: Vec<CEdge>,
    pub seqs: Vec<CSeq>,
    /// 0tick リピータ(遅延ゼロの組合せ増幅器)。`seqs` と違い不動点ループ内で評価する。
    pub zero_reps: Vec<ZeroRep>,
    pub tick: i64,
    pub trace: bool,
    /// `--vcd` 指定時の波形出力。None なら何もしない。
    pub vcd: Option<Vcd>,
}

impl Circuit {
    pub fn new(cfg: Config, trace: bool, vcd: Option<Vcd>) -> Self {
        Circuit {
            cfg,
            trace,
            vcd,
            ..Default::default()
        }
    }

    pub fn new_node(&mut self, nm: impl Into<String>, kind: NodeKind) -> usize {
        self.nodes.push(CNode::new(nm.into(), kind));
        let idx = self.nodes.len() - 1;
        self.par.push(idx);
        idx
    }

    /// union-find の代表(根)を返す。経路を半分に詰めながら辿る(path halving)ので
    /// 償却ほぼ定数。回路の全走査は `find(n) == n`(= 代表ノード)だけを対象にする。
    pub fn find(&mut self, mut x: usize) -> usize {
        while self.par[x] != x {
            self.par[x] = self.par[self.par[x]];
            x = self.par[x];
        }
        x
    }

    /// 2 つのノードを「同じ点」として併合する(エイリアス `x = y;` や階層直結で使う)。
    /// 種別は **ランク**(`Const` > `Input` > `Block` > `Plain`)が高い方を残し、各フラグは
    /// OR で合成する。base 値の食い違う const 同士の併合は曖昧なのでエラー。
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
        let (b_kind, b_base, b_inc, b_outg, b_out, b_elem, b_cq) = {
            let nb = &self.nodes[b];
            (
                nb.kind,
                nb.base,
                nb.has_incoming,
                nb.has_outgoing,
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
        na.has_outgoing = na.has_outgoing || b_outg;
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
        let rs = self.find(s);
        self.nodes[rs].has_outgoing = true;
    }

    pub fn add_seq(
        &mut self,
        kind: SeqKind,
        delay: i32,
        in_: usize,
        out: usize,
        label: String,
        line: i32,
    ) {
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
            line,
        });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
        let ri = self.find(in_);
        self.nodes[ri].has_outgoing = true;
    }

    /// 0tick リピータ(遅延ゼロの組合せ増幅器)を登録する。`out` は組合せ網内で
    /// `in_` の確定値から毎回計算されるので、出力ノードを `has_incoming` にしておく。
    pub fn add_zero_rep(&mut self, in_: usize, out: usize) {
        self.zero_reps.push(ZeroRep { in_, out });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
        let ri = self.find(in_);
        self.nodes[ri].has_outgoing = true;
    }

    /// コンパレータ素子(遅延 1 tick)。`side_in` が None なら横入力 0(= パススルー)。
    pub fn add_comp(
        &mut self,
        kind: SeqKind,
        in_: usize,
        side_in: Option<usize>,
        out: usize,
        label: String,
        line: i32,
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
            line,
        });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
        let ri = self.find(in_);
        self.nodes[ri].has_outgoing = true;
        if let Some(sn) = side_in {
            let rs = self.find(sn);
            self.nodes[rs].has_outgoing = true;
        }
    }

    /// ロック可能リピーター(遅延 n tick)。`side_in` は横(ロック)入力ノード。
    /// 横入力 > 0 の間は出力を直前の値で凍結し、0 に戻ると通常動作へ復帰する。
    /// ロックは 1 tick 反応(横入力の履歴は 1 段)。
    pub fn add_rep_lock(
        &mut self,
        delay: i32,
        in_: usize,
        side_in: usize,
        out: usize,
        label: String,
        line: i32,
    ) {
        let mut hist = VecDeque::new();
        for _ in 0..delay {
            hist.push_back(0);
        }
        let mut side_hist = VecDeque::new();
        side_hist.push_back(0);
        self.seqs.push(CSeq {
            kind: SeqKind::Rep,
            delay,
            in_,
            out,
            hist,
            side_in: Some(side_in),
            side_hist,
            outv: 0,
            prev_out: 0,
            togg: VecDeque::new(),
            cooldown: 0,
            label,
            line,
        });
        let r = self.find(out);
        self.nodes[r].has_incoming = true;
        let ri = self.find(in_);
        self.nodes[ri].has_outgoing = true;
        let rs = self.find(side_in);
        self.nodes[rs].has_outgoing = true;
    }

    /// リピーターがロック中か(ロック付きリピーターで横入力 > 0)。
    fn rep_locked(seq: &CSeq, side: i32) -> bool {
        seq.kind == SeqKind::Rep && seq.side_in.is_some() && side > 0
    }

    /// オブザーバ出力。隣接 2 サンプル(`prev = in(T-2)` / `cur = in(T-1)`)が
    /// モードの判定式を満たせば 1tick・強度 15 のパルス、満たさなければ 0。
    fn observer_out(mode: ObsMode, prev: i32, cur: i32) -> i32 {
        let fire = match mode {
            ObsMode::Any => prev != cur,
            ObsMode::Rise => prev == 0 && cur > 0,
            ObsMode::Fall => prev > 0 && cur == 0,
            ObsMode::Edge => (prev > 0) != (cur > 0),
        };
        if fire {
            15
        } else {
            0
        }
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
            // オブザーバは隣接 2 サンプルの比較なので (back,side) だけでは決まらない。
            // step() の phase1 / phase4 が hist の前後から `observer_out` で直接計算する
            // ため、ここには到達しない(網羅性のためのアーム)。
            SeqKind::Observer(_) => 0,
        }
    }

    /// ノード `n` に値 `v` を **合流** させ、値が増えたら `ch` を立てる(不動点ループの収束判定用)。
    /// `n` は事前に `find` した代表ノードを渡す。種別ごとの合流規則がそのまま MAX 合流の単調性を担う:
    ///
    /// - `Const` は駆動値固定なので無視、
    /// - `Block` は給電(`v>0`)で 15 にラッチ(2 値)、
    /// - `Plain`/`Input` は max(増加方向のみ更新)。`Input` は毎 tick `base`(var 駆動値)へ
    ///   リセットされるので、実効値は max(var 駆動, エッジ寄与) になる(issue #99)。
    ///   var とワイヤーが同じ点へ給電して max 合流する形で、階層インスタンスの
    ///   入力ポート(`Plain`)と同じセマンティクス。
    fn contribute(&mut self, n: usize, v: i32, ch: &mut bool) {
        let nd = &mut self.nodes[n];
        match nd.kind {
            NodeKind::Const => {}
            NodeKind::Block => {
                if v > 0 && nd.value < 15 {
                    nd.value = 15;
                    *ch = true;
                }
            }
            NodeKind::Plain | NodeKind::Input => {
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

    /// lint 用の **上界解析**(issue #48)。各ノードが今後の実行で到達しうる値の
    /// 上界を、`step()` と同じ MAX 合流の不動点で求めて返す(添字は全ノード分、
    /// 代表ノードのみ有効)。入力は最大値 15、const は宣言値を源とし、素子は
    ///
    /// - リピータ / オブザーバ: 後ろ入力の上界 > 0 なら 15、さもなくば 0
    /// - トーチ: 常に 15(履歴が 0 始まりなので必ず一度は点灯する)
    /// - コンパレータ(比較 / 減算): 後ろ入力の上界(横入力は減らす方向にしか効かない)
    ///
    /// で上へ伝える。過大評価はあっても過小評価はないので、**上界 0 = そのノードは
    /// 絶対に >0 にならない** が保証され、誤検出なしの到達不能判定に使える。
    pub fn lint_potentials(&mut self) -> Vec<i32> {
        let mut pot = vec![0i32; self.nodes.len()];
        // 代表ノード(根)だけ初期化する。`par[n] == n` は `find(n) == n` と等価。
        for (n, p) in pot.iter_mut().enumerate() {
            if self.par[n] != n {
                continue;
            }
            *p = match self.nodes[n].kind {
                NodeKind::Input => 15,
                NodeKind::Const => self.nodes[n].base,
                _ => 0,
            };
        }
        // contribute() と同じ合流規則(Block は >0 で 15 ラッチ、Plain/Input は MAX、
        // Const は固定)。Input は初期値 15(var の最大)なので join は実質増えないが、
        // step() と規則を揃えておく。単調なので step() と同じ発散ガードで必ず収束する。
        let join = |nodes: &Vec<CNode>, pot: &mut Vec<i32>, n: usize, v: i32, ch: &mut bool| {
            match nodes[n].kind {
                NodeKind::Const => {}
                NodeKind::Block => {
                    if v > 0 && pot[n] < 15 {
                        pot[n] = 15;
                        *ch = true;
                    }
                }
                NodeKind::Plain | NodeKind::Input => {
                    if v > pot[n] {
                        pot[n] = v;
                        *ch = true;
                    }
                }
            }
        };
        let mut ch = true;
        let mut guard: i64 =
            16 * (self.edges.len() as i64 + self.seqs.len() as i64 + self.zero_reps.len() as i64)
                + 64;
        while ch {
            ch = false;
            for ei in 0..self.edges.len() {
                let e = self.edges[ei];
                let s = self.find(e.s);
                let d = self.find(e.d);
                let v = (pot[s] - e.decay).max(0);
                join(&self.nodes, &mut pot, d, v, &mut ch);
            }
            for zi in 0..self.zero_reps.len() {
                let s = self.find(self.zero_reps[zi].in_);
                let d = self.find(self.zero_reps[zi].out);
                let v = if pot[s] > 0 { 15 } else { 0 };
                join(&self.nodes, &mut pot, d, v, &mut ch);
            }
            for si in 0..self.seqs.len() {
                let s = self.find(self.seqs[si].in_);
                let d = self.find(self.seqs[si].out);
                let v = match self.seqs[si].kind {
                    SeqKind::Rep | SeqKind::Observer(_) => {
                        if pot[s] > 0 {
                            15
                        } else {
                            0
                        }
                    }
                    SeqKind::Torch => 15,
                    SeqKind::CompCmp | SeqKind::CompSub => pot[s],
                };
                join(&self.nodes, &mut pot, d, v, &mut ch);
            }
            guard -= 1;
            if guard <= 0 {
                break;
            }
        }
        pot
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
            // オブザーバは履歴の前後 2 サンプル(in(T-2) / in(T-1))の変化検出。
            // ロック付きリピーターは横入力 > 0 の間、出力を直前値で凍結する。
            let mut o = if let SeqKind::Observer(mode) = kind {
                let prev = front; // hist.front() = in(T-2)
                let cur = *self.seqs[i].hist.back().unwrap_or(&0); // hist.back() = in(T-1)
                Self::observer_out(mode, prev, cur)
            } else if Self::rep_locked(&self.seqs[i], side_front) {
                self.seqs[i].prev_out
            } else {
                Self::seq_out_of(kind, front, side_front)
            };
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
        // 単調 MAX 更新なので必ず有限回で収束するが、念のため発散ガードを置く。
        // 1 ノードは最大 15 段しか増えないので、(全伝搬要素数)×16 + 余裕で十分上限。
        let mut guard: i64 =
            16 * (self.edges.len() as i64 + self.seqs.len() as i64 + self.zero_reps.len() as i64)
                + 64;
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
            // 0tick リピータ: 入力の確定値から同一 tick で出力(in>0?15:0)を合流する。
            // 入力は MAX 合流で単調増加するので出力も単調、不動点は決定的に収束する。
            for zi in 0..self.zero_reps.len() {
                let s = self.find(self.zero_reps[zi].in_);
                let d = self.find(self.zero_reps[zi].out);
                let v = if self.nodes[s].value > 0 { 15 } else { 0 };
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
            // オブザーバはサンプル後の履歴前後(in(T-1) / in(T))にエッジが残って
            // いれば次 tick でパルスする = 未整定。ロック中は出力が凍結されるので、
            // 期待出力は現在の出力(= 据え置き)。
            let expected = if let SeqKind::Observer(mode) = self.seqs[i].kind {
                let prev = *self.seqs[i].hist.front().unwrap_or(&0);
                let cur = *self.seqs[i].hist.back().unwrap_or(&0);
                Self::observer_out(mode, prev, cur)
            } else if Self::rep_locked(&self.seqs[i], side) {
                self.seqs[i].outv
            } else {
                Self::seq_out_of(self.seqs[i].kind, back, side)
            };
            if !uniform || self.seqs[i].outv != expected {
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
        if self.vcd.is_some() {
            self.dump_vcd();
        }
        changed
    }

    /// この tick の公開ノード値を VCD に書き出す。最初の呼び出しでヘッダ +
    /// 初期 `$dumpvars`(全 0)を出し、以降は値変化のあった信号だけを `#<tick>` 節に出す。
    fn dump_vcd(&mut self) {
        // self.nodes を借りるため Vcd を一旦取り出して使い終わったら戻す(借用衝突回避)。
        let mut v = match self.vcd.take() {
            Some(v) => v,
            None => return,
        };

        if !v.started {
            // 信号表を確定(ルート・名前付き・`#` を含まないノード = dump_trace と同基準)。
            let mut idx = 0usize;
            for n in 0..self.nodes.len() {
                if self.find(n) != n {
                    continue;
                }
                let nd = &self.nodes[n];
                if nd.name.is_empty() || nd.name.contains('#') {
                    continue;
                }
                v.sigs.push(VcdSig {
                    node: n,
                    code: Vcd::id(idx),
                    last: 0,
                });
                idx += 1;
            }
            // ヘッダ。$date/$version は決定性のため出さない(ゴールデン固定のため)。
            let _ = writeln!(v.w, "$comment redv VCD output $end");
            let _ = writeln!(v.w, "$timescale 100 ms $end");
            let _ = writeln!(v.w, "$scope module {} $end", v.scope);
            for s in &v.sigs {
                let _ = writeln!(v.w, "$var wire 4 {} {} $end", s.code, self.nodes[s.node].name);
            }
            let _ = writeln!(v.w, "$upscope $end");
            let _ = writeln!(v.w, "$enddefinitions $end");
            // 初期状態(全 0)を #0 で確定。
            let _ = writeln!(v.w, "#0");
            let _ = writeln!(v.w, "$dumpvars");
            for s in &v.sigs {
                let _ = writeln!(v.w, "b{:04b} {}", s.last & 0xF, s.code);
            }
            let _ = writeln!(v.w, "$end");
            v.started = true;
        }

        // この tick の変化を収集してから書く(変化が無ければ #<tick> 節は出さない)。
        let mut changes: Vec<(String, i32)> = Vec::new();
        for s in &mut v.sigs {
            let cur = self.nodes[s.node].value;
            if cur != s.last {
                changes.push((s.code.clone(), cur));
                s.last = cur;
            }
        }
        if !changes.is_empty() {
            let _ = writeln!(v.w, "#{}", self.tick);
            for (code, cur) in changes {
                let _ = writeln!(v.w, "b{:04b} {}", cur & 0xF, code);
            }
        }

        self.vcd = Some(v);
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
