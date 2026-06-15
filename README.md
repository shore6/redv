# redv (rv) — レッドストーン回路 HDL シミュレータ

[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![deps](https://img.shields.io/badge/dependencies-0-brightgreen.svg)](#)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#ライセンス)

*[English README here](README.en.md)*

Verilog のようにレッドストーン回路を **文字で** 設計し、コマンドラインで
コンパイル + シミュレーションできる処理系です。素子レベル(ゲートレベルよりさらに低レイヤー)で、
**「任意の2点を素子列のワイヤーでつなぐ」** ことで回路を記述します。

```rv
logic not_gate(input a, output y) {
    a-t-y;              // a と y をトーチ 1 本でつなぐ → これだけで NOT
}

module test() {
    var a, y;
    sim {
        a = 0;
        y = not_gate(a);                 // インスタンス化して変数を束縛
        #init                            // 定常状態まで待つ($time = 0)
        a = 10;  #1 #1                   // 入力を立てて 2 tick 進める
        ?monitor("t=%t a=% y=%\n", $time, a, y);
    }
}
```

Rust(edition 2021)実装・**依存クレートゼロ**・標準ライブラリのみ。`cargo build` だけでビルドできます。

---

## 特長

- **テキストで回路を書く** — レッドストーンの素子(ダスト・リピータ・トーチ・コンパレータ・ブロック)を
  文字列で並べ、2 点間をワイヤーでつなぐだけで回路になります。
- **tick 正確なシミュレーション** — リピータ遅延・トーチ反転・ダスト減衰・合流の最大値など、
  ゲーム仕様に沿った決定的なティックシミュレーションを行います。
- **Verilog 風テストベンチ** — `sim` ブロックで入力を駆動し、`#init` / `#n` / `wait()` で時間を進め、
  `monitor` で観測します。`if` / `while` / `for` も使えます。
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
cargo run --release -- examples/clock.rv          # cargo run 経由でも実行可
cargo test                                        # 全サンプルのゴールデンテスト + CLI テスト
```

### CLI オプション

| オプション | 動作 |
|---|---|
| `redv <file.rv>` | 回路をコンパイルしてシミュレーション(成功で終了コード 0、エラー時 1) |
| `-t`, `--trace` | 毎 tick の全ノード値を stderr にトレース出力 |
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
| `examples/clock.rv` | トーチ + リピータ 4 のクロック(周期 10)。`wait()` の使用例 |
| `examples/scan_and.rv` | `scan()` で stdin から 2 値を読んで AND に通す |
| `examples/hier_and.rv` | `not_gate` / `or_gate` を入れ子にした階層化 AND(ド・モルガン) |
| `examples/chain_mixed.rv` | 無名チェーンと named wire の併用 + 合流(max) |

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
  LANGUAGE.md   言語仕様・シミュレーションセマンティクスの詳細
```

## 言語仕様

回路定義・素子・ワイヤー・`sim` ブロック・ディレクティブ・シミュレーションセマンティクスの
詳細は **[docs/LANGUAGE.md](docs/LANGUAGE.md)** を参照してください。

最小例:

```rv
logic or_gate(input a, input b, output y) {
    a-r-y;          // a をリピータ経由で y へ
    b-r-y;          // b をリピータ経由で y へ(y で合流 = 最大値)
}
```

## ライセンス

MIT
