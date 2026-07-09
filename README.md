# redv (Red Verilog) — レッドストーン回路 HDL シミュレータ

[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![deps](https://img.shields.io/badge/dependencies-0-brightgreen.svg)](#)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#ライセンス)

*[English README](README.en.md)*

redv (Red Verilog) は、レッドストーン回路を Verilog 風の HDL で記述して、コマンドラインからコンパイルとティック単位のシミュレーションを実行する処理系である。
ゲートより低い素子レベル(ダスト、リピータ、トーチ、コンパレータ、オブザーバ)で、任意の 2 点を素子列でつないで回路を組み立てる。
テキストエディタとターミナルだけで回路を設計して検証できる。

```rv
logic NOT(input x, output y) {
    x-t-y;                       // x と y をトーチでつなぐ
}

module test {
    var x, y;
    sim {
        x = 0;
        y = NOT(x);
        #init                    // 定常状態まで待つ
        x = 10;  #1
        ?monitor("t=% x=% y=%\n", $time, x, y);
    }
}
```

Rust(edition 2021)実装、依存クレートゼロ、標準ライブラリのみで動く。
`cargo build` だけでビルドできる。

---

## 何ができるか

- **テキストで回路を書ける**。素子を文字列で並べ、点と点を素子列でつなぐだけで回路になる。
- **ティック正確にシミュレーションする**。リピータ遅延、トーチ反転、ダスト減衰、合流の最大値はゲーム仕様に沿う。順序非依存で決定的に収束する。
- **Verilog 風のテストベンチを書ける**。`sim` ブロックで入力を駆動し、`#init` / `#n` / `#until(cond)` / `wait()` で時間を進め、`monitor` で観測する。`assert` と `expect` を使えば合否を終了コードで判定できる。
- **バスとパラメータ定数を持つ**。`reg[N]` で複数レーンを束ねて 1 行で配線でき、スライスや連結、バスとスカラの直結にも対応する。`param W = 4;` で幅を 1 定義から複数幅に再利用でき、`logic g #(W=4)(...)` で呼び出しごとに幅を変えるジェネリック logic も書ける。
- **依存ゼロで単一バイナリ**。外部クレートが要らず、`cargo build --release` で `redv` が出来上がる。
- **Rust 風のキャレット診断**。エラーと警告は `--> file:line:col` とソース行、`^` 下線で表示する。構文エラーは正確な列を指す。

## インストールとビルド

```sh
git clone git@github.com:shore6/redv.git
cd redv
cargo build --release                       # target/release/redv を生成
```

## 使い方

```sh
./target/release/redv examples/not_gate.rv             # コンパイル + シミュレーション
./target/release/redv -t examples/or_gate.rv           # -t:毎 tick の全ノード値を stderr にトレース
./target/release/redv --vcd out.vcd examples/clock.rv  # --vcd:波形を VCD で出力
cargo run --release -- examples/clock.rv               # cargo run 経由でも実行できる
cargo test                                             # 全サンプルのゴールデンテストと CLI テスト
```

### CLI オプション

| オプション | 動作 |
|---|---|
| `redv <file.rv>` | 回路をコンパイルしてシミュレーションする(成功で終了コード 0、エラーで 1) |
| `-t`, `--trace` | 毎 tick の全ノード値を stderr にトレース出力する |
| `--vcd <file>` | 波形を VCD(Value Change Dump)形式で `<file>` に出力する(GTKWave 等で観測)。公開ノード(名前に `#` を含まない reg / ポート)を強度 0–15 の 4 bit ベクタで記録する。時刻は生 tick(`-t` と同じく `#init` 整定も含む)。module 複数時は `<file>.<module名>.vcd` に分割する |
| `--json` | monitor / assert / 警告を 1 行 1 イベントの JSON(JSONL)で出力する。monitor は stdout、assert / expect / warning と末尾 summary は stderr に出る。CI での回帰比較や別ツールへの取り込みに使う |
| `-W error` | 警告をエラー化する。完走後に警告(lint 含む)が 1 件でもあれば終了コード 1 を返す。CI で「警告ゼロ」を強制する用途([docs/LANGUAGE.md §10.4](docs/LANGUAGE.md)) |
| `-T`, `--time` | コンパイル時間とシミュレーション時間を stderr に出力する |
| `-h`, `--help` | usage を表示する(終了コード 0) |
| `-v`, `--version` | バージョンを表示する |
| 引数なし、不明オプション、ファイルなし | usage を stderr に出して終了コード 2 |

## サンプル

代表的なファイルを抜粋する。
全ファイルの一覧は [docs/LANGUAGE.md §12](docs/LANGUAGE.md) を参照。

| ファイル | 内容 |
|---|---|
| `examples/not_gate.rv` | トーチ 1 本の NOT |
| `examples/and_gate.rv` | トーチ 3 本(NOT の NOR)の AND |
| `examples/comparator_side.rv` | コンパレータのサイド入力(減算と比較) |
| `examples/repeater_lock.rv` | リピーターロック(`.side` で出力を凍結) |
| `examples/half_adder.rv` | 多出力 logic とタプル束縛 `(sum, carry) = HALF_ADDER(x1, x2);` |
| `examples/nested_call.rv` | ネスト呼び出し `y = s_or(s_and(x1,x2), s_xor(x3,x4));` と一行 MUX |
| `examples/bus_and4.rv` | バスポートとバス var で 4 ビットバスのビット単位 AND |
| `examples/bus_reg_side.rv` | バス reg への素子代入 `reg[4] m = r;` と `.side` の broadcast / レーン / スライス結線 |
| `examples/bus_lane_call.rv` | レーン / スライスを logic 呼び出しの引数とタプル束縛 target に直接書くリップルキャリー加算器 |
| `examples/generic_logic_width.rv` | logic ごとのジェネリック幅 `#(W=4)` で 1 定義を複数幅にインスタンス化 |
| `examples/slice_const_expr.rv` | スライス / レーン添字の定数式:`x[W-1:W/2]` でジェネリック幅バスを分割 |
| `examples/numeric_literals.rv` | 2 進 / 16 進整数リテラル(`0b1010` / `0xff`)を強度、幅、`#define`、sim 代入などで使う |
| `examples/define_expr.rv` | `#define N (W*2)` 等の定数式を `#define` で使う |
| `examples/monitor_format.rv` | monitor の基数書式 `%b` / `%x` / `%o` とゼロ埋め、`scan("%x")` で入力側も基数指定 |
| `examples/monitor_bus.rv` | バス var を 1 引数で monitor に渡し、各レーンを 4 bit ニブルでパッキングして表示 |
| `examples/stdlogic_demo.rv` | `#include "stdlogic"` で基本ゲート群(NOT / AND / OR / XOR / NAND / NOR / XNOR)を取り込む |
| `examples/stdmem_demo.rv` | `#include "stdmem"` でラッチ・レジスタ群(RS ラッチ / D ラッチ / D-FF / レジスタ)を取り込む |
| `examples/stdmem_generic.rv` | stdmem のジェネリック幅:`s_register#(W=4)` などでデータ経路を 4 ビット化 |
| `examples/assert_selfcheck.rv` | `assert` と `expect` で合否を終了コードに返す自己検証 |
| `examples/lint_demo.rv` | デザインルールチェック(lint)の 5 ルールをすべて発火させるデモ |
| `examples/vcd_demo.rv` | `--vcd` で波形を VCD 出力するデモ |
| `examples/vcd_generic.rv` | ジェネリック logic インスタンスのポートをトレース / VCD で観測するデモ |
| `examples/json_output.rv` | `--json` で monitor を JSONL として出力するデモ |

## ドキュメント

- **言語仕様**(`.rv` の文法、素子、シミュレーションセマンティクス):[docs/LANGUAGE.md](docs/LANGUAGE.md)
- **内部設計**(コンパイルパイプライン、エラボレーション、シミュレーションエンジン):[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **スタイルガイド**(命名、回路スタイルの選択、階層設計の推奨):[docs/STYLE.md](docs/STYLE.md)

最小例:

```rv
logic OR2(input x1, input x2, output y) {
    x1-r-y;          // x1 をリピータ経由で y へ
    x2-r-y;          // x2 をリピータ経由で y へ(y で合流 = 最大値)
}
```

`reg` / `wire` / ポートの名前は素子名(`d` / `r` / `cd` 等)と衝突できない。
チェーン内で曖昧になるためで、詳細は [docs/LANGUAGE.md §2](docs/LANGUAGE.md) を参照。

## プロジェクト構成

```
src/
  main.rs       CLI エントリポイント
  lexer.rs      字句解析
  parser.rs     構文解析(logic / module / sim / #define / #include)
  ast.rs        構文木定義(データ保持 enum)
  circuit.rs    回路グラフとティックシミュレーションエンジン
  interp.rs     エラボレーション(logic → 回路)と sim 実行系、monitor
  diag.rs       エラーと警告
  stdlib/       バンドル済み標準ライブラリ(`#include "stdlogic"` 等で取り込む)
examples/       サンプル回路
tests/
  golden.rs     ゴールデンテスト(cargo test)
  expected/     期待出力
docs/
  LANGUAGE.md       言語仕様とシミュレーションセマンティクスの詳細
  ARCHITECTURE.md   内部設計(パイプライン、各モジュール、シミュレーションエンジン)
  STYLE.md          スタイルガイド(命名、回路スタイルの選択、階層設計)
```

## ライセンス

MIT
