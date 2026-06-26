# redv アーキテクチャ / 内部設計

この文書は **redv のコードを読む/直す開発者** 向けに、技術スタック・コンパイル
パイプライン・各モジュールの責務・シミュレーションエンジンの内部設計・横断的な設計判断を
まとめたものです。

- **言語仕様(`.rv` の文法・素子・セマンティクスの確定値)は [LANGUAGE.md](LANGUAGE.md) が一次情報。**
  本文書は「言語が何をするか」ではなく **「処理系がそれをどう実装しているか」** を扱います。
- 概要・ビルド・使い方は [../README.md](../README.md)。
- 開発ワークフロー(issue 起点)は `.claude/skills/issue-dev/SKILL.md`。

---

## 1. 技術スタック

| 項目 | 内容 |
|---|---|
| 言語 | Rust(edition 2021) |
| 依存クレート | **ゼロ**(標準ライブラリのみ。`cargo build` だけで通る) |
| バイナリ | 単一 CLI `redv`(`src/main.rs`) |
| バージョン | `Cargo.toml` の `version` が唯一の正。`main.rs` が `env!("CARGO_PKG_VERSION")` で埋め込む |
| リリースビルド | `opt-level = 2`(`Cargo.toml` の `[profile.release]`) |
| テスト | `tests/golden.rs`(ゴールデン + CLI + エラーケース)。`cargo test` が回帰の砦 |
| CI | `.github/workflows/ci.yml`(build / test / clippy。`-D warnings` は付けない) |

設計上の指針:

- **依存ゼロを維持する。** 外部クレートを足す変更は原則 NG(プロジェクトの売りの一つ)。
- **`unsafe` を使わない。** 借用検査に素直に従う(下記「借用検査との付き合い方」参照)。
- 各モジュール冒頭の `//!` doc コメントに、そのモジュールの責務と設計上の判断をまとめている。

---

## 2. コンパイルパイプライン

`src/` はパイプライン上流から下流へ素直に並ぶ。データは一方向に流れる。

```
                                         ┌─────────────────────────────────────────┐
  .rv ソース                              │              interp.rs                  │
     │                                   │                                         │
     ▼          ┌──────────┐  Token 列   │  ┌────────────┐   Circuit   ┌─────────┐ │
  read_to_string│ lexer.rs ├────────────▶│  │ Elaborator │────────────▶│ circuit │ │
     │          └──────────┘             │  │ (logic→回路)│  ノード/エッジ│  .rs    │ │
     │          ┌──────────┐  Program    │  └────────────┘  順序素子     │ step()  │ │
     └─────────▶│ parser.rs├────────────▶│  ┌────────────┐             │ 不動点  │ │
                └────┬─────┘  (ast.rs)   │  │ ModuleExec │◀────────────│ エンジン │ │
                     │                   │  │ (sim 実行)  │  read/set    └─────────┘ │
              ast.rs (構文木)            │  └─────┬──────┘                          │
                     │                   └────────┼─────────────────────────────────┘
           diag.rs (fail/warn/caret)              ▼
                                            stdout(monitor)/ stderr(警告・assert)/ exit code
```

| ファイル | 行数目安 | 責務 |
|---|---|---|
| `main.rs` | 小 | CLI エントリ。引数解析・ファイル読込・各フェーズ呼び出し・終了コード・`--time` 計測 |
| `lexer.rs` | 小 | 字句解析。バイト列 → `Token` 列。コメント除去・2 文字演算子・文字列エスケープ |
| `parser.rs` | 中 | 再帰下降構文解析。`Token` 列 → `ast::Program`。`#include` はサブパーサで同じ `Program` を共有。バンドル済み標準ライブラリ(`stdlib/*.rv`)を `include_str!` で埋め込み、`#include "stdlogic"` 等で読む |
| `ast.rs` | 小 | 構文木。**データ付き enum** で不正状態を型で排除 |
| `interp.rs` | 大 | **エラボレーション**(`Elaborator`: logic→回路グラフ)+ **sim 実行**(`ModuleExec`)+ monitor 出力 |
| `circuit.rs` | 中 | 回路グラフ(ノード/エッジ/順序素子)+ **ティックシミュレーションエンジン** `step()` |
| `diag.rs` | 小 | 診断。`RvError`(行・桁付き)・`fail()` / `fail_at()`(Err 生成)・`warn()`・Rust 風キャレット描画 |

`main.rs` の処理順(`run_program` までの流れ):

```rust
let toks = lexer::Lexer::new(src).run()?;            // 1. 字句解析
let mut ps = parser::Parser::new(toks, dir);
ps.parse_file(&mut prog)?;                           // 2. 構文解析 → Program
let timings = interp::run_program(&prog, trace, vcd)?; // 3. エラボレーション + sim 実行
```

エラーは `diag::RvResult`(= `Result<T, RvError>`)で `?` 伝播し、`main` で `[error] ...` を
stderr に出して終了コード 1。CLI 引数エラーやファイル無しは終了コード 2(`main.rs`)。

---

## 3. モジュール詳細

### 3.1 `lexer.rs` — 字句解析

`Lexer::run()` がバイト列を走査して `Vec<Token>` を返す。

- トークン種別 `Tk`: `Ident` / `Num` / `Str` / `Punct` / `End`。
- `Token` は `{ k, s, num, line, col, len }`。`Num` は値を `num` に、`Ident`/`Str`/`Punct` はテキストを `s` に持つ。
- `Num` は 10 進、`0b...`(2 進)、`0x...`(16 進)の 3 形式を同じ `Tk::Num` に落とす。接頭辞直後に有効な数字が無いとき(`0b` 単独や `0xg` 等)はリテラル化せず `0` と後続を別トークンに分割して、強度ブロック `0b`(= 強度 0 + `b`)の従来解釈を温存する。
- コメント `//`(行)・`/* */`(範囲)はトークンを生成せずスキップ。未終端ブロックコメント / 文字列はエラー。
- 2 文字演算子 `<= >= == != && ||` は先読み 1 バイトでまとめる。それ以外の記号は 1 文字 `Punct`。
- 行番号 `line` ・桁 `col`(行内 1 始まりバイト位置)・長さ `len` を各トークンに付け、診断のキャレットに使う
  (`col` は `line_start` = 現在行先頭バイトから算出。改行を消すたびに更新)。

**ポイント**: `.`・`[`・`]`・`~`・`$`・`#`・`?` などは全て単一文字 `Punct` として落ちる。
これらの**意味付けは parser 以降**が行う(`.side` / `[k]` / パルス `~` / `$time` / `#init` / `?monitor`)。

### 3.2 `parser.rs` — 構文解析

`Parser` は `{ toks, i, base_dir, consts }`。再帰下降で `Program` を構築する。`Program` は構造体に保持せず
**各メソッドへ `&mut` 引数で渡す**(`#include` のサブパーサが同じ `Program` を共有するため)。
`consts` は `param` / 数値 `#define` の **パース時** 定数表で、確定値は `prog.defines` 側にも書く
(深い宣言メソッドは `prog` を持たないため、幅 `[expr]` 解決にはこのミラーを引く。`#include` は
サブパーサへ `consts` を引き継ぎ、戻りでマージする)。

`#include` は `BUNDLED_STDLIBS` テーブルに引いてヒットした名前(`stdlogic` 等)を、`include_str!` で
ビルド時に埋め込んだ標準ライブラリのソースから読む。`prog.included_stdlibs` にバンドル名を記録し、
2 度目以降の取り込みは no-op にして重複定義エラーを避ける。バンドルに無い名前は従来どおりファイル
include へフォールバックする(`base_dir` 相対 → 絶対の順で探す)。

トップレベルは `param` / `#`(ディレクティブ)/ `logic` / `module` の 4 種(`parse_file`)。
`param NAME = <定数式>;` は `eval_const`(`consts` を引いて `+ - * / %`・単項・括弧を畳む)で評価し、
バス幅 `[expr]`(`parse_width`)も同じ `eval_const` を使う。interp の `eval_e` は sim 式中の未知名を
`prog.defines` にフォールバックして param 参照を解決する。

- **logic** (`parse_logic` / `parse_logic_stmt`): ポート列 → 本体文。本体文は `wire` 宣言・`reg` 宣言・
  チェーン文・代入/インスタンス化。
  - **文の判別**: 先頭の識別子(端点。`.side` / `[k]` が付き得る ← `parse_chain_part`)を読み、
    次が `-` なら **チェーン接続文**(`from -..- to`)、`=` なら **代入 / インスタンス化**。文頭の `(` は
    **多出力タプル束縛** `(o1, o2, ...) = callee(...)` に割り当てる(従来エラーだった列に純加算)。
  - `out = callee(args...)` および `(o1, o2, ...) = callee(args...)` は階層インスタンス化(`Instance`)。
    parser はどちらも `Instance.outputs: Vec<String>` に正規化する。`= [strength] elem` は `AssignSingle`。
    `target = a-b-c`(`=` ありで複数チャンク)は wire への素子列定義(`AssignChain`)。
- **module** (`parse_module` / `parse_sim_stmt`): `var` 宣言と `sim { ... }` ブロック。
  - sim 文は `#n`/`#init`、`?monitor`、`var`、`if`/`while`/`for`、呼び出し `f(...)`、代入 `x = ...`、
    バスレーン代入 `x[i] = ...`、束縛 `v = callee(args)` / `(t1, t2, ...) = callee(args)`。後者の文頭 `(` も
    従来エラーだったので純加算で割り当てる。
  - 式は標準的な優先順位カスケード(`parse_or` → `and` → `eq` → `rel` → `add` → `mul` → `unary` → `primary`)。

**設計のキモ**: parser は記法を **汎用的に受理** し、妥当性判定を後段(interp)に寄せている箇所が多い。
例えば `.side` や `[k]` は `parse_chain_part` でどの端点にも付けられるが、「`.side` をコンパレータ/
リピーター以外に付けた」等のエラーは `interp.rs` の `resolve_dst` で出る。**新記法の追加が parser を
触らず interp の分岐追加だけで済む** ことが多いのはこのため。

### 3.3 `ast.rs` — 構文木

構文木は **データ付き enum** にして不正状態を型で表現不能にしている(素子種別・文種別の追加時に
コンパイラが網羅性を検査できる)。

主要な型:

- **logic 側**: `LogicStmt`(`DeclWire` / `DeclReg` / `AssignChain` / `Chain` / `AssignSingle` /
  `Instance`)、`Elem`(素子: `k` 種別 char + `n` 遅延)、`Port`(`width: Option<i32>` でバスポート)、
  `RegInit`(reg 初期化子)、`Qual`(`Plain`/`Const`/`Mutable`)。
- **module/sim 側**: `SimStmt`(`DeclVar` / `Assign` / `CallBind` / `WaitTicks` / `WaitInit` /
  `Call` / `MonReg` / `If` / `While` / `For`)、`Expr`(`Num`/`Var`/`Time`/`Bin`/`Un`)、`CallData`。
- **トップ**: `Program { defines, str_defines, logics, modules, included_stdlibs }`。`defines` には数値 `#define` に加え
  `param` 定数も入る(幅・式の解決はここを引く)。`included_stdlibs` は `#include "stdlogic"` 等のバンドル
  名で 2 度目以降の取り込みをスキップする重複ガード(§3.2)。

バス関連は `width: Option<i32>` / `index: Option<Box<Expr>>` を各所に持たせて表現する(後述 §6.3)。

### 3.4 `interp.rs` — エラボレーション + sim 実行

最大のモジュール。大きく 2 つの責務:

1. **`Elaborator`** — `logic` 定義を **回路グラフ**(`circuit::Circuit`)へ展開する(§4)。
2. **`ModuleExec`** — `module` の `sim` ブロックを **インタープリト実行** する(§5)。

`run_program(prog, trace)` がエントリ。`#define` から `Config`(タイムアウト・焼き切れ閾値)を組み、
全 `module` を宣言順に `ModuleExec::new().run()` で回す。`assert`/`expect` の合否は全 module 横断で
集計し、1 件でも失敗なら末尾サマリ付きで非ゼロ終了する。

### 3.5 `circuit.rs` — 回路グラフ + シミュレーションエンジン

回路の実体と `step()`(1 tick 進める)を持つ。詳細は §7。

### 3.6 `diag.rs` — 診断

- `RvError { line, col, len, msg }`。`col == 0` は **桁不明(行レベル)**、`col > 0` は
  キャレット位置(`len` は下線バイト幅)。
- `fail(line, msg) -> RvResult<T>`: 行レベル(桁不明)の `Err` を生成。
- `fail_at(line, col, len, msg)`: **桁・下線幅つき** の `Err`。lexer / parser のように
  トークン位置が分かる箇所で使う。
- `warn(line, msg)`: stderr へ `[warning] ...`(エラーと同じキャレット描画)。
- `set_source(file, src)`: キャレット描画用にソース全行を `OnceLock` へ登録(字句解析前に 1 回)。
- `report_error(&RvError)` / 内部 `render()`: **Rust 風キャレット診断**(`--> file:line:col` +
  ソース行 + `^` 下線)を stderr に出す。`col == 0` のときは行内容(行末 `//` コメント手前まで)を下線。

**規約**: 回復不能はすべて `fail` / `fail_at`(= `Err` 伝播)、続行可能な注意は `warn`。`line == 0` は
「特定の行に紐付かない」エラー/警告(エラボレーション結果や sim 実行時など)で、キャレットなしの簡易表示に退避する。

**キャレットの精度(issue #47)**: **lexer / parser** はトークンの桁を持つので `fail_at` で
**正確な列**にキャレットを出す。**interp(エラボレーション)** の診断は AST が行のみ保持する設計のため
`fail` の **行レベル**(ソース行を表示し内容全体を下線)に留める。桁は **バイト**基準で、キャレットの
インデント/下線幅は文字数で測る(ASCII のコード片はぴったり合う。日本語コメント等の全角幅は未考慮)。

**JSON モード(`--json`, issue #49)**: `set_json_mode()` でグローバルに有効化すると、
`warn` / `report_error` はキャレット描画ではなく `{"kind":"warning|error","line":N,"msg":"..."}`
形式の JSONL を stderr に出す。interp 側も `is_json_mode()` を引いて `do_monitor` を
`{"time":N,"values":[...],"fmt":"..."}` の JSONL に切り替え、`do_assert` / `do_expect` の失敗と
末尾サマリも JSONL で出す。`json_escape_into` は文字列を JSON リテラル(両端 `"` 付き、制御文字は
`\uXXXX` で退避)に書き出す共通ヘルパ。

---

## 4. エラボレーション(logic → 回路グラフ)

`Elaborator::elaborate()` が `logic` 1 つを回路ノード群へ展開する中心。

### 4.1 回路の素材

エラボレーションが作るのは `circuit.rs` の 3 種:

- **ノード** `CNode` — reg / ポート / 素子の入出力点。種別 `NodeKind`(`Plain`/`Block`/`Const`/`Input`)。
- **エッジ** `CEdge { s, d, decay }` — ダスト減衰や直結を表す有向辺。`decay` はダスト個数(直結は 0)。
- **順序素子** `CSeq` — リピータ・トーチ・コンパレータ・オブザーバ(遅延つき)。`ZeroRep` は遅延ゼロの `r0`。

### 4.2 ノードの別名併合(union-find)

`x = y;`(エイリアス)や階層インスタンスの直結などで「2 つのノードは同じ点」と分かったとき、
`Circuit::merge(a, b)` で **union-find** により併合する。

- `find()` は経路圧縮つき。回路全走査は「`find(n) == n`(= 代表ノード)」だけを対象にする。
- 併合時に種別の **ランク**(`Const` > `Input` > `Block` > `Plain`)が高い方を残し、`has_incoming` /
  `is_out_port` / `elem_assigned` / `is_const_qual` は OR で合成する。
- **競合する const reg**(別 base 値)の併合はエラー。

### 4.3 チェーンの展開

`from -chunks- to` は `build_chain_body()` が素子列を辿り、素子ごとにノード/エッジ/順序素子を生やす:

- `d`(ダスト): `decay` を +1 するだけ(ノードを作らず次のエッジに減衰として乗せる)。
- `b`(ブロック): `Block` ノードを作り、直前から `decay` のエッジを張る。
- `r1`–`r4` / `t` / `o`(リピータ/トーチ/オブザーバ): 入力ノード `#i` と出力ノード `#o` を作り、
  `add_seq` で順序素子化。オブザーバは隣接 2 サンプルの比較なので履歴 2 段(`delay=2`)で登録する。
- `r0`(0tick リピータ): `add_zero_rep` で **不動点ループ内で評価する組合せ素子** として登録(§7.4)。
- `cc` / `cd`(インラインのコンパレータ): `add_comp(side=None)` = 横入力なし = パススルー。

中間チャンクには **wire 名** も書け、`expand_chain_tokens()` が `wire_seq` を引いて再帰展開する
(`visited` で循環検出)。内部ノード名には `#`(`foo.#ch1#i3` 等)を含め、trace 非表示にする(§9)。

### 4.4 「名前付きの点」を持つ順序素子 — 3 ノード束パターン

コンパレータ reg(`reg cmp = cd;`)・ロック付きリピーター reg(`reg m = r;`)は、後ろ(back)・
横(side)・前(out)の **3 端子** を持つ。これを **3 ノード束** として展開するのが共通パターン:

```
reg cmp = cd;  →  back ノード ─┐
                  side ノード ─┼─▶ add_comp/add_rep_lock ─▶ out ノード
                              (順序素子)
```

- `back`/`side`/`out` の 3 ノードを作り、`side_regs[name] = (back, side)`、`scope[name] = out` に登録。
- チェーン終端の解決(`resolve_dst`)で **`.side` → side ノード / 無印 → back ノード**、始端解決
  (`resolve_src` 経由)で **`name` → out ノード** に振り分ける。コンパレータもロック付きリピーターも
  この束を共有するので、新しい side 系素子を足すときはこのパターンに倣う。
- **宣言時初期化に限る**(`reg cmp; cmp = cd;` の後置代入はエラー)。これは 3 端子を宣言時に確定させる
  ための制約で、後置代入(`AssignSingle`)経路では `apply_elem` が宣言時形へ誘導するエラーを出す。

### 4.5 階層インスタンス化

`(o1, o2, ...) = callee#(P=v, ...)(args...)` は `instances` に貯め、**全ノード確定後にまとめて結線** する。
出力 1 個の `out = callee(...)` は parser で `outputs = vec![out]` に正規化する(`Instance.outputs: Vec<String>`、LANGUAGE.md §5.5)。

- サブ logic を `top_level = false` で再帰エラボレート(入力ポートは `Input` でなく `Plain` ノードになる)。
- 親の引数ノード → サブ入力ポート、各サブ出力ポート → 親の対応する出力先を `connect_ports`(減衰なし直結 = エッジ decay 0)で結ぶ。
- **再帰インスタンス化**(自己・相互の循環)は `stack` で検出してエラー。出力ポート数と target 数は厳格一致(過不足ともエラー)、同一 target の重複もエラー。未接続 output ポートはエラー。

### 4.6 ジェネリック幅 param(`#(...)`)と `param_env`

logic ごとのジェネリック幅(`logic g #(W=4)(input[W] x, ...)`、LANGUAGE.md §8.4)は
**per-instance のパラメータ環境** をエラボレーションに流すだけで実装している。`elaborate()` は
`param_env: HashMap<String, i64>` を 1 個受け取り、`Port` / `DeclReg` の `width`(= `WidthExpr`)を
`resolve_width(we, &param_env, &defines)` で解決する。`WidthExpr::Lit(n)` は parse 時に確定済みの
即値(従来挙動と一致)、`WidthExpr::Expr(e)` は logic ローカル param を含むため遅延された定数式。

`build_callee_param_env()` が「callee の宣言 param + 既定値 + 呼び出し側の `#(...)` 実引数」を
突き合わせて環境を組む。実引数の式は **呼び出し側の `param_env`** で eval するので、親 logic の
param 値が子インスタンスへそのまま流れる(`s = inner#(N=W)(...)`)。

シミュレーションエンジンと意味論は完全に非介入:`#(W=4)` と `#(W=8)` は `do_call_bind` の
インスタンスキャッシュキー(`param_env_key` を含めた `callee#(W=8)(args)`)が変わるので
別インスタンスとして展開され、各々独立したノード群を持つ。`var[N]`(module 側)はジェネリック
対象外で、`DeclVar.width` は parse 時に `i32` へ即時解決する(`WidthExpr` を持たない)。

### 4.7 エラボレーションのエラーは「呼び出されて初めて」発火する

`logic` 単体を書いただけでは展開されない。`module` 内で `v = g(...)` とインスタンス化されて初めて
`elaborate` が走り、エラーが出る。**logic だけのファイルは `[warning] no module to run` で素通り** する。
エラーケースを試すときは必ず呼び出しを添える:

```rv
module m(){ var a,y; sim{ a=0; y=g(a); #init } }
```

---

## 5. sim 実行(`ModuleExec`)

`sim` ブロックは回路シミュレーションではなく **逐次インタープリタ** で実行する。

### 5.1 状態

| フィールド | 役割 |
|---|---|
| `c: Circuit` | この module 専用の回路(module ごとに独立) |
| `vars` | sim 変数(バス var はレーンを `x[0]` 等のキーで格納) |
| `var_buses` | バス var の幅(`name -> width`) |
| `insts` | `callee(args)` をキーにしたインスタンス表(同じ呼び出しは同一インスタンス) |
| `out_bind` | 出力ノード → 束縛先 var(毎 tick 後に var へ書き戻す) |
| `mons` | ホイストした `?monitor`(sim 開始時に収集) |
| `pulses` | パルス代入(`x = v ~ w`)の残り tick |
| `sim_time` | `#init` 完了時(未使用なら開始時)を 0 とした経過 tick |
| `assert_total`/`assert_failed` | 自己検証の集計 |

### 5.2 時間の進め方

`tick_once()` が 1 tick の最小単位: **入力反映(`apply_inputs`)→ `c.step()` → 出力反映
(`apply_outputs`)→ パルス減算(`tick_pulses`)**。これを:

- `#n` / `wait(n)` … n 回。`#n` は `$time` を進めて monitor を発火、`wait(n)` は **進めず発火しない**(発振回路用)。
- `#init` … `step()` が「変化なし」を返すまで(定常状態)。`INIT_TIMEOUT` 超過はエラー(発振の検出)。

`?monitor` は **各ウェイト完了直後** に全件発火する(Verilog `$monitor` 風)。sim 内のどこに書いても全時刻が出る。

### 5.3 入出力の束縛

`(t1, t2, ...) = g(args)`(`CallBind` → `do_call_bind`)で:

- 初回はキー `g(arg1,arg2)` でインスタンスを生成・エラボレートし、引数 var を入力ノードに、各出力ポートのレーン列を
  `Instance.out_ports: Vec<Vec<usize>>` として保持する。同じキーの 2 回目以降は同じインスタンスを再利用(`targets` はキーに含めない =
  別の var 組で同じ出力を観測できる)。
- 出力ポートごとに、対応する target(スカラ var / バス var)へレーン対応で `out_bind` を登録する。
  形(スカラ/バス)と幅が一致しないとエラー。出力ポート数と target 数の厳格一致・同一 target の重複も `SimStmt::CallBind` で検査する(LANGUAGE.md §5.5)。
- 1 出力 logic は `t = callee(...)` でも `(t) = callee(...)` でも書ける(parser で `targets = vec![t]` に正規化)。
- バス var ↔ バスポートはレーン対応で束縛する。
- 入力反映時、回路へ束縛した var は **0–15 にクランプ**(範囲外は変数ごとに 1 回だけ警告)。

`scan()` も `CallBind` 経由(`do_scan`)。stdin から整数 1 個を読む(EOF・非数値はエラー)。
scan は出力を 1 個しか返さないので、`(t1, t2) = scan(...)` のタプル束縛はエラーにする。

---

## 6. データモデルの要点

### 6.1 ノード種別 `NodeKind`

- `Input` — sim 変数で駆動される入力(top-level 入力ポート)。値は毎 tick 先頭で `base` から再設定。
- `Const` — 不変の定数(`const reg`)。`base` を持つ。
- `Block` — ブロック。給電(>0)なら 15、さもなくば 0(2 値)。
- `Plain` — 通常の点。複数経路の合流は **max**。

### 6.2 順序素子 `SeqKind` と出力関数

`circuit.rs::seq_out_of(kind, back, side)` が出力の純関数を一手に持つ:

- `Rep`: `back > 0 ? 15 : 0`
- `Torch`: `back > 0 ? 0 : 15`
- `CompCmp`(比較): `back >= side ? back : 0`
- `CompSub`(減算): `max(0, back - side)`

ロック付きリピーターはこの純関数を超える(前 tick の出力を凍結する)ため、`rep_locked()` で分岐し
**出力確定(phase 1)と #init 定常判定(phase 4)の両方** に手を入れている(片方だけだと #init が収束しない。§7.5)。

**オブザーバ** `Observer` も `(back, side)` の純関数では決まらない(隣接 2 サンプル `in(T-2)` と
`in(T-1)` の比較)ので、`seq_out_of` ではなく `observer_out(prev, cur)` を **phase 1 / phase 4 の両方** で
`hist` の前後から直接呼ぶ(履歴 2 段に乗るのでロックと同じく両相を対応させる)。

### 6.3 バスは純粋な糖衣

`reg[N]` / `input[N]` / `output[N]` / `var[N]` は **N 本のスカラ点・スカラ操作への糖衣**。
エラボレーション時に N 本のスカラチェーンへ展開されるだけで、**`circuit` エンジンはバスを一切知らない**。

- logic 側: `buses: HashMap<String, Vec<usize>>`(ベース名 → レーンノード列)。`scope`(スカラ)とは別空間。
- 端点解決: `Ep::Single`(スカラ点)/ `Ep::Bus`(レーン列)。バスチェーンは両端同幅必須で element-wise に展開。
- sim 側: バス var のレーンは `vars` に `x[0]` 等のキーで格納し、`var_buses` に幅を持つ。

このため **シミュレーション意味論(合流 max・順序素子・`#init`)はスカラ点と完全に同一**。

---

## 7. シミュレーションエンジン(`circuit.rs::step()`)

意味論の確定値は LANGUAGE.md §9 が一次情報。ここでは **実装の構造** を説明する。

`step()` は 1 レッドストーンティックを **3〜4 相** で進める。順序非依存・決定的。

### 7.1 phase 1 — 順序素子の出力確定

各順序素子 `CSeq` について、`hist`(後ろ入力履歴)・`side_hist`(横入力履歴)の **front(= 最古 =
`delay` tick 前)** から `seq_out_of()` で出力 `outv` を計算する。ロック中は `prev_out` で凍結、
トーチ焼き切れクールダウン中は 0 に固定。オブザーバは `hist` の front / back(= `in(T-2)` / `in(T-1)`)を
`observer_out()` に渡して変化検出する。

### 7.2 phase 2 — 組合せ網の MAX 合流不動点

1. 全代表ノードの `value` をリセット(`Const`/`Input` は `base`、他は 0)。
2. 順序素子の出力 `outv` を出力ノードへ寄与(`contribute`)。
3. 変化が無くなるまでループ:
   - 各エッジ `s → d` で `value(s) - decay`(0 でクランプ)を `d` へ寄与。
   - 各 `ZeroRep` で `in > 0 ? 15 : 0` を出力へ寄与。

**合流は単調 MAX 更新**(`Plain` は max、`Block` は >0 で 15 にラッチ)なので不動点は決定的に収束し、
走査順に結果が依存しない(逐次キュー方式と同じ結果を順序非依存で保証する実装)。`guard` で
発散を打ち切る(エッジ/素子数 × 16 + 64 回が上限)。

### 7.3 phase 3 — トーチ焼き切れ検出 + 出力変化追跡

トーチが出力を変えた tick を `togg` に記録し、**監視窓内のトグル回数が閾値超過** なら警告して
一定期間強制 OFF(クールダウン)。`changed` フラグに「この tick で何か変わったか」を集める。

### 7.4 phase 4 — 順序素子の入力サンプリング

各順序素子が現在のノード値を `hist` / `side_hist` の back に push、front を pop(`delay` 段のシフトレジスタ)。
さらに **パイプラインが一様か**(`hist` が全段同値か)を見て、まだ過渡状態なら `changed = true` にする
(これが無いと `#init` がパイプライン充填前に止まってしまう)。

### 7.5 `#init`(定常判定)と `changed`

`step()` の戻り値 `changed` は「全ノード値が前 tick と不変、かつ全順序素子のパイプラインが一様」を
**偽** とする。`do_init()` はこれが偽になるまで回す。ロック付きリピーターのような状態保持素子を足すときは、
phase 1(出力)と phase 4(期待出力 = 据え置き)の **両方** で `rep_locked` を考慮しないと `changed` が
落ちず `#init` が収束しないので注意。

### 7.6 `r0`(0tick リピータ)が組合せ網に乗る理由

`r0` は `out = in > 0 ? 15 : 0` を **同一 tick** で確定する。入力について **単調**(in が増えれば out も
増える)なので、MAX 合流の不動点ループにそのまま参加でき、決定性・順序非依存を保てる。一方 **0tick の
トーチ/コンパレータ**(反転・減算)は非単調で組合せループ検出を伴うため、`r0` のように単純には乗らない
(別 issue 扱い)。

---

## 8. 借用検査との付き合い方

`self` を走査しながら書き換える箇所は、Rust の借用検査に通すために素朴な参照イテレートを避けて
形を変えてある。結果(逐次キュー方式)は変わらない:

- **インデックス走査**: `circuit.rs::step()` は `self.seqs` / `self.edges` / `self.nodes` を
  `for i in 0..len` の添字で回す(`CEdge` は `Copy`)。参照を保持したまま `self` を書き換える二重借用を避ける。
- **集めてから適用**: `interp.rs` で `insts` / `out_bind` を走査しつつ回路を書き換える箇所は、必要な値を
  一旦ローカル `Vec` に集めてから適用する(`apply_inputs` / `apply_outputs` / `do_call_bind`)。
- `Program` を parser の構造体に持たせず `&mut` で渡すのも、`#include` のサブパーサと共有するため(§3.2)。

---

## 9. テストと回帰の砦

`tests/golden.rs` が `cargo test` で走る。**回帰はこれで守る**(CI も `cargo test`)。

- **ゴールデンテスト**: `examples/*.rv` を実行し、stdout を `tests/expected/*.txt` と **バイト比較**。
  期待値は **LF 固定**(`.gitattributes` で `tests/expected/*.txt` を `eol=lf`)。Windows で生成すると
  CRLF が混ざるので `tr -d '\r'` を通す。
- **ゴールデンは _出力_ を比較する(ソースは見ない)。** 回路が等価なら `examples/*.rv` を新構文へ
  書き換えても期待値 `.txt` は不変で緑のまま → 記法変更・構文糖衣のリグレッション確認に使える。
- **CLI テスト**: 引数なし/不明オプション/ファイル無しの終了コード、`--time` の stderr 出力。
- **エラーケーステスト**: `run_source()` でソース文字列を一時ファイル化して叩き、(終了コード, stderr) を
  検証する。エラボレーションのエラーは module 呼び出しを添えないと発火しない(§4.7)ので注意。
- **ノード名の `#`**: 内部ノード(`foo.w#i3` 等)は `dump_trace` でスキップされる。`-t` で公開したい点は
  名前に `#` を含めない。`--vcd <file>` の VCD 波形出力(`circuit.rs::dump_vcd`、`Vcd`)も **同じ公開ノード
  基準** で信号を選び、`dump_trace` と並んで `step()` から毎 tick 駆動される。値変化のみを記録し、時刻は
  生 tick(`#init` 整定も含む)。VCD ゴールデンは生成ファイルを `tests/expected/vcd_demo.vcd` と比較する。

新機能を足したら **サンプル + ゴールデン(LF) + `tests/golden.rs` のテスト関数** を 1 つ以上追加する。

---

## 10. 横断的な設計判断・ハマりどころ

- **後方互換を設計で確保する。** 既存ゴールデンの出力が変わる変更は原則 NG。新機能は「未使用時は従来と
  同一挙動」に倒す(例: コンパレータの side 未接続 = 0 で旧パススルーに退化、`r0` 以外は従来どおり)。
- **記法追加は「従来エラーだったトークン列」に割り当てると純加算で安全。** 例: 文頭 `ident -` は従来
  構文エラー → 無名チェーン文に割り当て(既存記法と衝突しない)。`reg = r` も従来エラー → ロック付き
  リピーターに割り当て。
- **破壊的変更は Phase 分割で独立 PR に。** 「純加算の新記法」と「旧記法の廃止/再定義」は別 PR。前者
  (回帰ゼロ)を先にマージし、後者の破壊的差分を小さく保つ。`closes #N` は最終 Phase の PR に付ける。
- **妥当性判定は parser でなく interp に寄せる**(§3.2)。新記法が parser を触らず interp の分岐追加で
  済むことが多い。
- **既存の警告は追わない。** `dead_code`(未使用 `line`/`delay`/`has_sim` フィールド)や `type_complexity`
  (`elaborate` の戻り値型)は既存のもの。自分の変更が増やした警告だけ気にする(CI も `-D warnings` 無し)。
- **ゲームの固有名はコード・ドキュメントに書かない**(公式とは無関係)。必要なら「ゲーム」と表記する。
  ゲームとの差異(0tick リピータ・ロック条件の緩さ等)は LANGUAGE.md に「ゲームとの差異」として残す。

---

## 11. 用語集

| 用語 | 意味 |
|---|---|
| エラボレーション | `logic` 定義を回路グラフ(ノード/エッジ/順序素子)へ展開すること(`Elaborator`) |
| ノード(`CNode`) | 回路上の点。reg / ポート / 素子の入出力点 |
| エッジ(`CEdge`) | 有向辺。ダスト減衰(`decay`)や直結(`decay=0`)を表す |
| 順序素子(`CSeq`) | 遅延を持つ素子(リピータ/トーチ/コンパレータ/オブザーバ)。履歴 `hist` を持つ |
| `ZeroRep` | 0tick リピータ。遅延なしで組合せ網の不動点ループ内で評価する増幅器 |
| 3 ノード束 | side 入力を持つ reg(コンパレータ/ロック付きリピーター)の back/side/out 展開パターン(§4.4) |
| union-find | ノード別名併合(`merge`/`find`)。回路走査は代表ノードのみ対象 |
| 不動点(MAX 合流) | 組合せ網を単調 MAX 更新で収束させる解法(§7.2)。順序非依存・決定的 |
| ゴールデンテスト | `examples/*.rv` の stdout を `tests/expected/*.txt` とバイト比較する回帰テスト |
| 糖衣 | バス(`reg[N]` 等)のようにスカラ操作へ展開されるだけの記法(§6.3) |
