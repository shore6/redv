# redv (rv) — レッドストーン回路 HDL シミュレータ

[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![deps](https://img.shields.io/badge/dependencies-0-brightgreen.svg)](#)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#ライセンス)

*[English README](README.en.md)*

Verilog のようにレッドストーン回路を **文字で** 設計し、コマンドラインで
コンパイル + シミュレーションできる処理系です。素子レベル(ゲートレベルよりさらに低レイヤー)で、
**「任意の2点を素子列のワイヤーでつなぐ」** ことで回路を記述します。

```rv
logic not_gate(input x, output y) {
    x-t-y;              // x と y をトーチ 1 本でつなぐ → これだけで NOT
}

module test() {
    var x, y;
    sim {
        x = 0;
        y = not_gate(x);                 // インスタンス化して変数を束縛
        #init                            // 定常状態まで待つ($time = 0)
        x = 10;  #1 #1                   // 入力を立てて 2 tick 進める
        ?monitor("t=% x=% y=%\n", $time, x, y);
    }
}
```

Rust(edition 2021)実装・**依存クレートゼロ**・標準ライブラリのみ。`cargo build` だけでビルドできます。

---

## 特長

- **テキストで回路を書く** — レッドストーンの素子(ダスト・リピータ・トーチ・コンパレータ・ブロック・
  オブザーバ)を文字列で並べ、2 点間をワイヤーでつなぐだけで回路になります。
- **tick 正確なシミュレーション** — リピータ遅延・トーチ反転・ダスト減衰・合流の最大値など、
  ゲーム仕様に沿った決定的なティックシミュレーションを行います。
- **Verilog 風テストベンチ** — `sim` ブロックで入力を駆動し、`#init` / `#n` / `#until(cond)` / `wait()` で時間を進め、
  `monitor` で観測します。`if` / `while` / `for`、パルス代入 `a = v ~ w;`(w tick 後に自動で 0)も使えます。
  `assert(cond)` / `expect(actual, expected)` で自己検証も書けます — 失敗は stderr に出て **非ゼロ終了**
  するので、合否は終了コードで分かります(ゴールデン文字列に依存しない)。
- **バス(`reg[N]` / `input[N]` / `output[N]` / `var[N]`)** — 複数レーンを束ねて宣言し、
  `in - r - buf;` の 1 行で全レーンをまとめて配線できます(高密度回路の表記簡素化)。レーンは
  `a[k]` で個別に取り出せ、バスポートと sim のバス var を介して **多入出力** をそのまま束縛できます。
- **パラメータ定数(`param W = 4;`)** — 整数定数を宣言し、バス幅(`input[W]` / `reg[W+1]`)や
  sim 式から参照できます。幅をリテラル固定せず **1 定義を複数幅で再利用** できます。
- **Rust 風のキャレット診断** — エラー / 警告は `--> file:line:col` とソース行・`^` 下線で表示します。
  構文エラーは正確な列を指します。
- **依存ゼロ・単一バイナリ** — 外部クレート不要。`cargo build --release` で `redv` が出来上がります。
- **厳しめの診断** — 範囲外信号・未接続出力・不正素子・発振の非収束などをエラー / 警告で報告します。

## インストールとビルド

```sh
cargo build --release            # target/release/redv を生成
```

## 使い方

```sh
./target/release/redv examples/not_gate.rv        # コンパイル + シミュレーション
./target/release/redv -t examples/or_gate.rv      # -t: 毎 tick の全ノード値を stderr にトレース
./target/release/redv --vcd out.vcd examples/clock.rv  # --vcd: 波形を VCD で出力(GTKWave 等で観測)
cargo run --release -- examples/clock.rv          # cargo run 経由でも実行可
cargo test                                        # 全サンプルのゴールデンテスト + CLI テスト
```

### CLI オプション

| オプション | 動作 |
|---|---|
| `redv <file.rv>` | 回路をコンパイルしてシミュレーション(成功で終了コード 0、エラー時 1) |
| `-t`, `--trace` | 毎 tick の全ノード値を stderr にトレース出力 |
| `--vcd <file>` | 波形を VCD(Value Change Dump)形式で `<file>` に出力(GTKWave 等で観測)。公開ノード(名前に `#` を含まない reg / ポート)を強度 0–15 の 4bit ベクタで記録。時刻は生 tick(`-t` と同じく `#init` 整定も含む)。module 複数時は `<file>.<module名>.vcd` に分割 |
| `-T`, `--time` | コンパイル時間 / シミュレーション時間を stderr に出力 |
| `-h`, `--help` | usage を表示(終了コード 0) |
| `-v`, `--version` | バージョンを表示 |
| 引数なし / 不明オプション / ファイルなし | usage を stderr、終了コード 2 |

## サンプル

| ファイル | 内容 |
|---|---|
| `examples/not_gate.rv` | トーチ 1 本の NOT |
| `examples/or_gate.rv` | リピータ 2 本 + ダスト合流の OR |
| `examples/and_gate.rv` | トーチ 3 本(NOT の NOR)の AND |
| `examples/decay.rv` | ダスト減衰 / リピータ再増幅 / コンパレータの強度パススルーの比較 |
| `examples/counter_test.rv` | `for` / `if` で AND の真理値表を自動検証 |
| `examples/assert_selfcheck.rv` | `assert` / `expect` で合否を終了コードに返す自己検証テストベンチ |
| `examples/clock.rv` | トーチ + リピータ 4 のクロック(周期 10)。`wait()` の使用例 |
| `examples/scan_and.rv` | `scan()` で stdin から 2 値を読んで AND に通す |
| `examples/hier_and.rv` | `not_gate` / `or_gate` を入れ子にした階層化 AND(ド・モルガン) |
| `examples/chain_mixed.rv` | チェーン文で 2 経路を同じ点に合流(max) |
| `examples/comparator_side.rv` | コンパレータのサイド入力(`cd` 減算 / `cc` 比較) |
| `examples/repeater_lock.rv` | リピーターロック(`reg m = r;` の `.side` で出力を凍結) |
| `examples/repeater_0tick.rv` | 0tick リピータ(`r0`)と通常リピータ(`r1`)の反応タイミング比較 |
| `examples/observer.rv` | オブザーバ(`o`): 入力の変化を検出して 1tick パルス(立ち上がり/立ち下がり/強度変化) |
| `examples/until_wait.rv` | `#until(cond)`: 条件成立まで tick を進めるイベント駆動待機(出力が立つまで遅延数を知らずに待つ) |
| `examples/wire_reuse.rv` | wire を再利用可能な素子列として定義し複数箇所で使い回す |
| `examples/pulse.rv` | パルス代入(`a = v ~ w;`)で w tick 後に var を自動で 0 に戻す |
| `examples/bus_or4.rv` | バス `reg[N]`: 4 レーンを `in - r - buf;` の 1 行でまとめて配線 |
| `examples/bus_and4.rv` | バスポート + バス var で 2 本の 4 ビットバスのビット単位 AND(多入出力) |
| `examples/param_notN.rv` | param 定数で幅をパラメータ化した N ビット NOT |
| `examples/vcd_demo.rv` | `--vcd` で波形を VCD 出力するデモ(トーチ反転 + リピータ遅延) |

## プロジェクト構成

```
src/
  main.rs       CLI エントリポイント
  lexer.rs      字句解析
  parser.rs     構文解析 (logic / module / sim / #define / #include)
  ast.rs        構文木定義(データ保持 enum)
  circuit.rs    回路グラフ + ティックシミュレーションエンジン
  interp.rs     エラボレーション(logic→回路) + sim 実行系 + monitor
  diag.rs       エラー / 警告
examples/       サンプル回路
tests/
  golden.rs     ゴールデンテスト (cargo test)
  expected/     期待出力
docs/
  LANGUAGE.md       言語仕様・シミュレーションセマンティクスの詳細
  ARCHITECTURE.md   内部設計(パイプライン・各モジュール・シミュレーションエンジン)
```

開発者向けの内部設計(コンパイルパイプライン・エラボレーション・シミュレーションエンジンの
仕組み・横断的な設計判断)は **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** を参照してください。

## 言語仕様

回路定義・素子・ワイヤー・`sim` ブロック・ディレクティブ・シミュレーションセマンティクスの
詳細は **[docs/LANGUAGE.md](docs/LANGUAGE.md)** を参照してください。

最小例:

```rv
logic or_gate(input a, input b2, output y) {
    a-r-y;          // a をリピータ経由で y へ
    b2-r-y;         // b をリピータ経由で y へ(y で合流 = 最大値)
}
```

> `reg` / `wire` / ポート名に素子名(`b` / `r` / `cd` 等)と衝突する名前は使えない
> (チェーン内で曖昧になるため。詳細は [docs/LANGUAGE.md](docs/LANGUAGE.md) §2)。

## ライセンス

MIT
