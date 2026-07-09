//! ゴールデンテスト: examples/*.rv を実行し、tests/expected/*.txt と
//! 標準出力がバイト一致することを検証する(オリジナル tests/run.sh 相当)。

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// リリース/デバッグどちらでも cargo がビルドした redv バイナリのパス。
/// CARGO_BIN_EXE_<name> は統合テスト実行時に cargo が自動設定する。
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_redv")
}

fn run_golden(name: &str) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/{name}.rv");
    let expected_path = format!("{manifest}/tests/expected/{name}.txt");

    assert!(Path::new(&rv).exists(), "missing example: {rv}");
    let expected =
        std::fs::read(&expected_path).unwrap_or_else(|e| panic!("read {expected_path}: {e}"));

    let out = Command::new(bin())
        .arg(&rv)
        .output()
        .unwrap_or_else(|e| panic!("spawn redv: {e}"));

    assert!(
        out.status.success(),
        "{name}: exit {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    if out.stdout != expected {
        panic!(
            "{name}: stdout mismatch\n--- expected ---\n{}\n--- got ---\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn not_gate() {
    run_golden("not_gate");
}

#[test]
fn or_gate() {
    run_golden("or_gate");
}

#[test]
fn and_gate() {
    run_golden("and_gate");
}

#[test]
fn decay() {
    run_golden("decay");
}

#[test]
fn counter_test() {
    run_golden("counter_test");
}

#[test]
fn comparator_side() {
    run_golden("comparator_side");
}

#[test]
fn clock() {
    run_golden("clock");
}

/// リピーターロック: 横入力 > 0 の間、出力を直前値で凍結する。
#[test]
fn repeater_lock() {
    run_golden("repeater_lock");
}

/// パルス代入: `a = v ~ w;` は w tick 後に var を自動で 0 に戻す。
#[test]
fn pulse() {
    run_golden("pulse");
}

/// 0tick リピータ(`r0`): 遅延ゼロの組合せ増幅器。r1 と反応タイミングが 1 tick ずれる(issue #37)。
#[test]
fn repeater_0tick() {
    run_golden("repeater_0tick");
}

/// オブザーバ素子(`o`): 入力の変化を検出して 1tick パルス(立ち上がり/立ち下がり/強度変化, issue #45)。
#[test]
fn observer() {
    run_golden("observer");
}

/// オブザーバのエッジ亜種(`op` 立ち上がり / `on` 立ち下がり / `oe` 2値エッジ):
/// 判定式だけが異なり、強度変化(15→7)は `o` だけが拾う(issue #58)。
#[test]
fn observer_edge() {
    run_golden("observer_edge");
}

/// イベント駆動待機(`#until(cond)`): 条件成立まで tick を進める($time は #n 同様に進む, issue #42)。
#[test]
fn until_wait() {
    run_golden("until_wait");
}

/// クロック生成シュガー `clock(var, N)`: var を各レベル N tick 保持で 0/15 に自動トグル(issue #44)。
#[test]
fn clock_sugar() {
    run_golden("clock_sugar");
}

/// assert / expect による自己検証テストベンチ(全 assert が通れば exit 0)(issue #40)。
#[test]
fn assert_selfcheck() {
    run_golden("assert_selfcheck");
}

/// stdin を流し込んで stdout がゴールデンと一致するか検証する。
fn run_golden_stdin(name: &str, input: &str) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/{name}.rv");
    let expected_path = format!("{manifest}/tests/expected/{name}.txt");

    assert!(Path::new(&rv).exists(), "missing example: {rv}");
    let expected =
        std::fs::read(&expected_path).unwrap_or_else(|e| panic!("read {expected_path}: {e}"));

    let mut child = Command::new(bin())
        .arg(&rv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn redv: {e}"));
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait redv");

    assert!(
        out.status.success(),
        "{name}: exit {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    if out.stdout != expected {
        panic!(
            "{name}: stdout mismatch\n--- expected ---\n{}\n--- got ---\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn scan_and() {
    run_golden_stdin("scan_and", "15 0\n");
}

/// monitor / scan の基数書式(issue #77):
/// `%b` / `%x` / `%o` と `%Nb` / `%0Nb`(ゼロ埋め)・負値は `-` 接頭 + 絶対値で
/// 基数表記。`scan("%b")` 等で入力側も基数指定できる。
#[test]
fn monitor_format() {
    run_golden_stdin("monitor_format", "10 1010 ff 17\n");
}

/// バス var を 1 引数で monitor に渡す(issue #49 残項目):
/// 各レーン強度 0-15 を 4 bit のニブルとしてパッキングし(`lane[0]` が最下位)、
/// `%x` は既定 N 桁・`%b` は 4N bit にゼロ埋め。ユーザー指定幅は下限として効く。
#[test]
fn monitor_bus() {
    run_golden("monitor_bus");
}

#[test]
fn hier_and() {
    run_golden("hier_and");
}

/// 多出力 logic のタプル束縛 `(sum, carry) = half_adder(x1, x2);`(issue #79)。
/// 半加算器の 2 出力 (sum = XOR, carry = AND) を同時に受け取る。
#[test]
fn half_adder() {
    run_golden("half_adder");
}

/// ネスト呼び出し `y = s_or(s_and(x1,x2), s_xor(x3,x4));`(issue #97)。
/// sim 側と logic 本体(MUX2)の両方で、中間 reg / var なしに呼び出しを直接ネストする。
#[test]
fn nested_call() {
    run_golden("nested_call");
}

/// `#include "stdlogic"` でバンドル済みの基本ゲート群(NOT/AND/OR/XOR/NAND/NOR/XNOR)を取り込む
/// (issue #55)。7 ゲートを 4 通りの入力で sweep して真理値表が一致するか検証する。
#[test]
fn stdlogic_demo() {
    run_golden("stdlogic_demo");
}

/// stdlogic のジェネリック幅 `#(W=4)`(issue #121): 全ゲートを 4 レーン化し、
/// ビット単位の element-wise 演算(NOT/AND/OR/XOR/NAND/NOR/XNOR)を検証する。
#[test]
fn stdlogic_generic() {
    run_golden("stdlogic_generic");
}

/// stdlogic を同じソース内で複数回 include しても重複定義エラーにならない(issue #55)。
/// 2 度目以降の `#include "stdlogic"` は no-op になる。
#[test]
fn stdlogic_double_include_is_noop() {
    let src = "#include \"stdlogic\"\n#include \"stdlogic\"\n\
               module m{ var a,y; sim{ a=0; y=s_not(a); #init } }";
    let (code, stderr) = run_source("stdlogic_dup", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// `#include "stdmem"` でバンドル済みのラッチ・レジスタ 4 種
/// (s_rslatch / s_dlatch / s_dff / s_register)を取り込む(issue #87)。
#[test]
fn stdmem_demo() {
    run_golden("stdmem_demo");
}

/// stdmem のジェネリック幅 `#(W=4)`(issue #95): s_dlatch / s_dff / s_register の
/// データ経路(x / q)を 4 レーン化し、制御線(en / ld / clk)を全レーン共有する。
#[test]
fn stdmem_generic() {
    run_golden("stdmem_generic");
}

/// stdmem は内部で stdlogic をネスト include する(issue #87)。利用側が先に
/// `#include "stdlogic"` を書いていても、stdmem 側のネスト include は no-op になり
/// 重複定義エラーにならない。両バンドルの logic が同時に使える。
#[test]
fn stdmem_nested_stdlogic_include_is_noop() {
    let src = "#include \"stdlogic\"\n#include \"stdmem\"\n\
               module m{ var a,en,y,z; sim{ a=0; en=15; y=s_not(a); z=s_dlatch(a,en); #init } }";
    let (code, stderr) = run_source("stdmem_dup", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 未知の stdlib 名はバンドル一覧にマッチしないため file include へフォールスルーし、
/// 存在しないファイルとして通常の include エラーになる(issue #55)。
#[test]
fn unknown_stdlib_is_file_error() {
    let src = "#include \"stdfoo\"\nmodule m{ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("stdfoo", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("cannot open include file: stdfoo"),
        "unexpected stderr:\n{stderr}"
    );
}

/// チェーン文で 2 経路を同じ点に合流(max)。
#[test]
fn chain_mixed() {
    run_golden("chain_mixed");
}

/// wire を再利用可能な素子列として定義し、複数箇所で使い回す(各箇所で独立展開)。
#[test]
fn wire_reuse() {
    run_golden("wire_reuse");
}

/// バス reg(`reg[N] a;`): 4 レーンを `in - r - buf;` の 1 行でまとめて配線(issue #11)。
#[test]
fn bus_or4() {
    run_golden("bus_or4");
}

/// バスポート + バス var + バス束縛: 2 本の 4 ビットバスのビット単位 AND(issue #11, Phase 1b)。
#[test]
fn bus_and4() {
    run_golden("bus_and4");
}

/// param 定数: 幅 `input[W]`/`var[W]` と sim 式での W 参照(issue #41 Phase 1)。
#[test]
fn param_not_n() {
    run_golden("param_notN");
}

/// logic ごとのジェネリック幅 `#(W=4)`(issue #41 Phase 2):
/// 同じ logic 定義を呼び出しごとに異なる幅で別インスタンスへ展開する。
#[test]
fn generic_logic_width() {
    run_golden("generic_logic_width");
}

/// バスのレーン / スライスを logic 呼び出しの引数とタプル束縛 target に直接書く
/// (issue #118): リップルキャリー加算器のレーン引数・レーン target、スライス引数、
/// sim 側のレーン引数 / target を 16x16 全ケースの参照モデルで検証する。
#[test]
fn bus_lane_call() {
    run_golden("bus_lane_call");
}

/// バスのスライス `a[hi:lo]`(ビット反転)と連結 `{a, b}`(左ローテート)(issue #43)。
#[test]
fn bus_slice_concat() {
    run_golden("bus_slice_concat");
}

/// バス reg への素子代入(issue #95): `reg[4] m = r;` / `reg[4] c = cd;` をレーンごとの
/// named point に展開し、`.side` を全体(broadcast / element-wise)・レーン `m[k].side`・
/// スライス `m[hi:lo].side` の各粒度で結線する。
#[test]
fn bus_reg_side() {
    run_golden("bus_reg_side");
}

/// スライス / レーン添字の定数式化(issue #89):
/// `x[W-1:W/2]` のようなジェネリック param 式(インスタンス化時に評価)と、
/// `a[N+1]` のような param 参照(パース時に即時解決)の両方。
#[test]
fn slice_const_expr() {
    run_golden("slice_const_expr");
}

/// 2 進 / 16 進整数リテラル `0b1010` / `0xff`(issue #49):
/// 強度・バス幅・param・#define・sim 代入・tick 数など、従来 10 進が書けた
/// 場所すべてで使えること。
#[test]
fn numeric_literals() {
    run_golden("numeric_literals");
}

/// const reg の裸数値初期化 `const reg n = 15;`(issue #75):
/// 素子トークンを伴わない数値だけの初期化。10 進 / 16 進とも使える。
#[test]
fn const_reg() {
    run_golden("const_reg");
}

#[test]
fn bus_scalar() {
    run_golden("bus_scalar");
}

/// CLI 動作: 引数なしは usage を出して終了コード 2。
#[test]
fn no_args_exits_2() {
    let out = Command::new(bin()).output().expect("spawn redv");
    assert_eq!(out.status.code(), Some(2));
}

/// CLI 動作: 不明なオプションは終了コード 2。
#[test]
fn unknown_option_exits_2() {
    let out = Command::new(bin()).arg("-x").output().expect("spawn redv");
    assert_eq!(out.status.code(), Some(2));
}

/// CLI 動作: 存在しないファイルは終了コード 2。
#[test]
fn missing_file_exits_2() {
    let out = Command::new(bin())
        .arg("does_not_exist.rv")
        .output()
        .expect("spawn redv");
    assert_eq!(out.status.code(), Some(2));
}

/// CLI 動作: `--time` は compile/sim/total の計測値を stderr に出し、
/// stdout はフラグ無しと完全一致する(issue #25)。値は非決定的なので
/// ゴールデン比較せず「行が出ること」「stdout が不変なこと」だけ検証する。
#[test]
fn time_flag_reports_to_stderr() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/clock.rv");

    let plain = Command::new(bin()).arg(&rv).output().expect("spawn redv");
    let timed = Command::new(bin())
        .arg("--time")
        .arg(&rv)
        .output()
        .expect("spawn redv");

    assert!(timed.status.success(), "--time should exit 0");
    // stdout は計測フラグの有無で変わらない。
    assert_eq!(
        timed.stdout, plain.stdout,
        "--time must not alter stdout"
    );

    let stderr = String::from_utf8_lossy(&timed.stderr);
    for needle in ["[time] compile:", "[time] sim:", "[time] total:"] {
        assert!(
            stderr.contains(needle),
            "missing {needle:?} in stderr:\n{stderr}"
        );
    }
    // フラグ無しでは計測行を出さない。
    let plain_stderr = String::from_utf8_lossy(&plain.stderr);
    assert!(
        !plain_stderr.contains("[time]"),
        "timing should be opt-in, stderr:\n{plain_stderr}"
    );
}

/// `--vcd <file>` で生成した VCD が `tests/expected/<name>.vcd` とバイト一致するか検証する
/// (issue #46)。VCD はファイルへ出るので stdout でなく生成ファイルを比較する。
fn run_vcd_golden(name: &str) {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/{name}.rv");
    let expected_path = format!("{manifest}/tests/expected/{name}.vcd");
    let expected = std::fs::read(&expected_path)
        .unwrap_or_else(|e| panic!("read {expected_path}: {e}"));

    // 並行実行で衝突しないよう PID 入りの一意な一時パスに書く。
    let out_path = std::env::temp_dir().join(format!("redv_vcd_{name}_{}.vcd", std::process::id()));
    let out = Command::new(bin())
        .arg("--vcd")
        .arg(&out_path)
        .arg(&rv)
        .output()
        .expect("spawn redv");
    assert!(
        out.status.success(),
        "{name}: exit {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let got = std::fs::read(&out_path).expect("read generated VCD");
    let _ = std::fs::remove_file(&out_path);

    // VCD 生成は Windows で CRLF が混ざりうるので CR を落として LF 比較する。
    let strip_cr = |v: &[u8]| -> Vec<u8> { v.iter().copied().filter(|&b| b != b'\r').collect() };
    let got = strip_cr(&got);
    let expected = strip_cr(&expected);
    if got != expected {
        panic!(
            "{name}: VCD mismatch\n--- expected ---\n{}\n--- got ---\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&got)
        );
    }
}

/// 入力ポートの max 合流(issue #99): トップレベルの入力ポートも回路内配線から駆動でき、
/// var 駆動との max になる。自己保持回路がトップレベルと階層経由で同じに動く。
#[test]
fn input_feedback() {
    run_golden("input_feedback");
}

#[test]
fn vcd_demo() {
    run_vcd_golden("vcd_demo");
}

/// ジェネリック logic のインスタンス(キーに `#(...)` を含む)のポートが、`#` を除いた
/// ノード名 `inv(W=2)(a).x[0]` でトレース / VCD に現れることを検証する(issue #101)。
/// dump_trace と dump_vcd は公開判定が別実装なので両方を確認する。
#[test]
fn vcd_generic() {
    run_golden("vcd_generic");
    run_vcd_golden("vcd_generic");

    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/vcd_generic.rv");
    let out = Command::new(bin())
        .arg("-t")
        .arg(&rv)
        .output()
        .expect("spawn redv");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("inv(W=2)(a).x[0]="),
        "generic instance nodes missing from -t trace:\n{stderr}"
    );
}

/// 与えたソースを一時ファイルに書いて redv に渡し、(終了コード, stderr) を返す。
fn run_source(tag: &str, src: &str) -> (Option<i32>, String) {
    let path = std::env::temp_dir().join(format!("redv_test_{tag}.rv"));
    std::fs::write(&path, src).expect("write temp rv");
    let out = Command::new(bin()).arg(&path).output().expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// module 名直後の `()` は旧記法であり、新記法 `module name { ... }` へ誘導する
/// エラーになる(issue #96 Phase 2 で廃止)。
#[test]
fn module_parens_is_error() {
    let src = "module m(){ var a; sim{ a=0; assert(a==0); } }";
    let (code, stderr) = run_source("module_parens_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("module takes no arguments"),
        "unexpected stderr:\n{stderr}"
    );
}

/// 診断のキャレット表示(issue #47): 構文エラーは `--> file:line:col` + ソース行 +
/// 正確な列の `^` を出す。`=` は 2 行目 12 桁目なので `:2:12` とキャレットが出る。
#[test]
fn caret_diagnostic_points_at_exact_column() {
    let src = "logic g(input x, output y){\n    x - r2 = y;\n}\n";
    let (code, stderr) = run_source("caret_col", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(stderr.contains(":2:12"), "missing line:col, stderr:\n{stderr}");
    assert!(stderr.contains("-->"), "missing source pointer, stderr:\n{stderr}");
    assert!(stderr.contains("x - r2 = y;"), "missing source line, stderr:\n{stderr}");
    assert!(stderr.contains('^'), "missing caret, stderr:\n{stderr}");
}

/// 意味(エラボレーション)エラーは行レベル: 正確な列は持たないが、ソース行と
/// `-->` ・行内容の下線を出す(issue #47 の見出し例 `unknown element`)。
#[test]
fn caret_diagnostic_line_level_for_elaboration_error() {
    let src = "logic g(input x, output y){\n    x - d2q - y;\n}\n\
               module m{ var a,b; sim{ a=0; b=g(a); #init } }";
    let (code, stderr) = run_source("caret_line", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(stderr.contains("unknown element 'q'"), "missing message, stderr:\n{stderr}");
    assert!(stderr.contains("-->") && stderr.contains(":2"), "missing pointer, stderr:\n{stderr}");
    assert!(stderr.contains("x - d2q - y;"), "missing source line, stderr:\n{stderr}");
    assert!(stderr.contains('^'), "missing caret, stderr:\n{stderr}");
}

/// 素子名と衝突する宣言名(reg / wire / ポート)はパース時にエラーになる。
/// インスタンス化を待たず発火するので module 呼び出しは不要。
#[test]
fn element_name_collision_is_error() {
    // 単体素子・コンパレータ・連結素子列、reg / wire / port の各サイト。
    for (tag, src) in [
        ("reg_cd", "logic g(input a, output y){ reg cd; a-t-y; }"),
        ("port_d", "logic g(input a, input d, output y){ a-t-y; }"),
        ("wire_r", "logic g(input a, output y){ wire r; a-t-y; }"),
        ("reg_td", "logic g(input a, output y){ reg td; a-t-y; }"),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains("collides with an element name"),
            "{tag}: unexpected stderr:\n{stderr}"
        );
    }
}

/// ブロック素子 `b` と素子接尾字つき強度リテラルは廃止(issue #75)。
/// チェーン / reg の `b` は unknown element、旧 `15b` / `3d` は裸数値への
/// 案内エラー、数字を伴わない `0b` は字句エラーになる。
#[test]
fn block_element_is_removed() {
    for (tag, body, want) in [
        ("chain_b", "a - b - y;", "unknown element 'b'"),
        ("reg_b", "reg p = b; a-p; p-y;", "unknown element 'b'"),
        ("const_15b", "const reg n = 15b; n-y;", "write a bare number instead"),
        ("const_3d", "const reg n = 3d; n-y;", "write a bare number instead"),
        ("const_elem_only", "const reg n = d; n-y;", "bare signal strength"),
        ("digitless_0b", "const reg n = 0b; n-y;", "expected binary digits after '0b'"),
    ] {
        let src = format!(
            "logic g(input a, output y){{ {} }}\n\
             module t{{ var u,v; sim{{ u=0; v=g(u); #init }} }}",
            body
        );
        let (code, stderr) = run_source(&format!("rmblk_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// リピータ / コンパレータ reg は **宣言時初期化に限る**(`reg m = r;`)。
/// 後置代入(`reg m; m = r;`)は宣言時形へ誘導するエラーになる(issue #21)。
#[test]
fn seq_reg_post_assignment_is_error() {
    for (tag, src, want) in [
        (
            "post_rep",
            "logic g(input a, output y){ reg m; a-m; m=r; m-y; }\n\
             module t{ var u,v; sim{ u=0; v=g(u); #init } }",
            "must be initialized at its declaration",
        ),
        (
            "post_comp",
            "logic g(input a, output y){ reg cmp; a-cmp; cmp=cd; cmp-y; }\n\
             module t{ var u,v; sim{ u=0; v=g(u); #init } }",
            "must be initialized at its declaration",
        ),
        (
            "post_torch",
            "logic g(input a, output y){ reg z; a-z; z=t; z-y; }\n\
             module t{ var u,v; sim{ u=0; v=g(u); #init } }",
            "a torch belongs inside a wire/chain",
        ),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains(want),
            "{tag}: unexpected stderr:\n{stderr}"
        );
    }
}

/// 宣言時初期化(`reg m = r;` / `reg cmp = cd;`)は従来どおり受理される(issue #21)。
#[test]
fn seq_reg_declaration_init_is_accepted() {
    let src = "logic g(input a, output y){ reg m = r; a-m; m-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("decl_rep_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 裸数値初期化は const 専用(issue #75, Phase 1)。plain / mutable reg に
/// 数値だけを与えると素子接尾字形と同じメッセージで停止する。
#[test]
fn bare_strength_on_non_const_is_error() {
    for (tag, src) in [
        (
            "bare_plain",
            "logic g(input a, output y){ reg n = 15; a-t-y; }\n\
             module t{ var u,v; sim{ u=0; v=g(u); #init } }",
        ),
        (
            "bare_mutable",
            "logic g(input a, output y){ mutable reg n = 15; a-t-y; }\n\
             module t{ var u,v; sim{ u=0; v=g(u); #init } }",
        ),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains("signal-strength literals are only allowed on const reg"),
            "{tag}: unexpected stderr:\n{stderr}"
        );
    }
}

/// 裸数値初期化も強度の範囲(0-15)を検査する(issue #75, Phase 1)。
#[test]
fn bare_strength_out_of_range_is_error() {
    let src = "logic g(input a, output y){ const reg n = 16; a-t-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("bare_range_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("const signal strength out of range 0-15: 16"),
        "unexpected stderr:\n{stderr}"
    );
}

/// 0tick リピータ(`r0`)は inline チェーン専用。ロック付き reg(`reg m = r0;`)は
/// 保持する状態が無いのでエラーになり、inline 利用へ誘導する(issue #37)。
#[test]
fn zero_tick_repeater_as_reg_is_error() {
    let src = "logic g(input a, output y){ reg m = r0; a-m; m-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("r0_reg_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("0-tick repeater") && stderr.contains("inline"),
        "unexpected stderr:\n{stderr}"
    );
}

/// 0tick リピータは inline チェーン素子(`x - r0 - y;`)として受理される(issue #37)。
#[test]
fn zero_tick_repeater_inline_is_accepted() {
    let src = "logic g(input a, output y){ a-r0-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("r0_inline_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// オブザーバ(`o`)は横端子を持たずインラインチェーン専用。reg(`reg p = o;`)に
/// 置こうとするとエラーになり、inline 利用へ誘導する(issue #45)。
#[test]
fn observer_as_reg_is_error() {
    let src = "logic g(input a, output y){ reg p = o; a-p; p-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("obs_reg_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("observer belongs inline"),
        "unexpected stderr:\n{stderr}"
    );
}

/// オブザーバは inline チェーン素子(`x - o - y;`)として受理される(issue #45)。
#[test]
fn observer_inline_is_accepted() {
    let src = "logic g(input a, output y){ a-o-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("obs_inline_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// エッジ亜種(`op`/`on`/`oe`)も本体 `o` と同じく reg には置けない(issue #58)。
#[test]
fn observer_variant_as_reg_is_error() {
    let src = "logic g(input a, output y){ reg p = oe; a-p; p-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("obs_variant_reg_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("observer belongs inline"),
        "unexpected stderr:\n{stderr}"
    );
}

/// エッジ亜種の綴りは素子名の名前衝突ルール(§2.2)に自動で乗る:
/// `on` を reg 名にしようとするとエラーになる(issue #58)。
#[test]
fn observer_variant_name_collides() {
    let src = "logic g(input a, output y){ reg on; a-on; on-y; }\n\
               module t{ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("obs_variant_name_err", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("collides with an element name"),
        "unexpected stderr:\n{stderr}"
    );
}

/// 全 assert / expect が真なら exit 0 で「all passed」サマリを出す(issue #40)。
#[test]
fn assert_all_passed_exits_zero() {
    let src = "module m{ var a; sim{ a=0; assert(a==0); expect(a, 0); } }";
    let (code, stderr) = run_source("assert_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(stderr.contains("all passed"), "unexpected stderr:\n{stderr}");
}

/// 偽の assert は失敗を記録し、末尾サマリ付きで非ゼロ終了する(issue #40)。
#[test]
fn assert_failure_exits_nonzero() {
    let src = "module m{ var a; sim{ a=0; assert(a > 0); } }";
    let (code, stderr) = run_source("assert_fail", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("assertion failed") && stderr.contains("1 of 1 failed"),
        "unexpected stderr:\n{stderr}"
    );
}

/// expect の不一致は「実際の値 / 期待値」を出力して非ゼロ終了する(issue #40)。
#[test]
fn expect_mismatch_reports_values() {
    let src = "module m{ var a; sim{ a=7; expect(a, 3); } }";
    let (code, stderr) = run_source("expect_fail", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("expect failed") && stderr.contains("= 7, expected 3"),
        "unexpected stderr:\n{stderr}"
    );
}

/// 失敗しても sim は継続し、全チェックを集計する(2 件失敗を 1 度に把握できる)(issue #40)。
#[test]
fn assert_collects_all_failures() {
    let src = "module m{ var a; sim{ a=0; assert(a > 0); expect(a, 9); assert(a == 0); } }";
    let (code, stderr) = run_source("assert_collect", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(stderr.contains("2 of 3 failed"), "unexpected stderr:\n{stderr}");
}

/// assert / expect の引数個数が違えばエラー(issue #40)。
#[test]
fn assert_expect_arity_is_error() {
    for (tag, src, want) in [
        (
            "assert_two",
            "module m{ var a; sim{ a=0; assert(a, 1); } }",
            "assert(cond) takes exactly one",
        ),
        (
            "expect_one",
            "module m{ var a; sim{ a=0; expect(a); } }",
            "expect(actual, expected) takes exactly two",
        ),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// 素子名でない宣言名(`b2` / `cmp` / `x` / `c` 等)は受理される。
/// `b` / `tb` はブロック素子の廃止(issue #75)で素子列でなくなったので使える。
#[test]
fn non_element_names_are_accepted() {
    let src = "logic g(input a, input b2, output y){ reg cmp, x, c, b, tb; a-t-y; }\n\
               module m{ var u, v; sim{ u=0; v=g(u,u); #init } }";
    let (code, stderr) = run_source("non_elem_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// バス reg(`reg[N]`)+ 添字 `a[k]` + 全体チェーン `p - r - q;` は受理される(issue #11)。
#[test]
fn bus_basic_is_accepted() {
    let src = "logic g(input a, output y){ reg[2] p; reg[2] q; a-p[0]; a-p[1]; p-r-q; q[0]-y; }\n\
               module m{ var u,v; sim{ u=15; v=g(u); #init } }";
    let (code, stderr) = run_source("bus_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// バス↔スカラの直結はブロードキャストとして受理される(issue #63)。
/// fan-in(bus[N]-scalar = レーンの MAX 合流)・fan-out(scalar-bus[N] = 全レーン駆動)、
/// 素子列を挟む形・スライス/連結の幅 1 相手も同様。
#[test]
fn bus_scalar_broadcast_is_accepted() {
    for (tag, body) in [
        ("fanin", "reg[4] p; reg z; a-p[0]; a-p[1]; a-p[2]; a-p[3]; p-z; z-y;"),
        ("fanin_elem", "reg[4] p; reg z; a-p[0]; a-p[1]; a-p[2]; a-p[3]; p-r-z; z-y;"),
        ("fanout", "reg[4] p; a-p; p[0]-y;"),
        ("fanout_elem", "reg[4] p; a-r-p; p[0]-y;"),
        ("slice_fanin", "reg[4] p; a-p[0]; a-p[1]; a-p[2]; a-p[3]; reg z; p[3:0]-z; z-y;"),
        ("concat_fanin", "reg[2] p; reg z; a-p[0]; a-p[1]; {a, p} - z; z-y;"),
    ] {
        let src = format!("logic g(input a, output y){{ {body} }}\n\
                           module m{{ var u,v; sim{{ u=15; v=g(u); #init }} }}");
        let (code, stderr) = run_source(&format!("bus_bc_{tag}"), &src);
        assert_eq!(code, Some(0), "{tag}: expected success, stderr:\n{stderr}");
    }
}

/// バスチェーンの幅不一致・スカラ混在・範囲外添字・非バス添字は **エラー**(issue #11)。
#[test]
fn bus_misuse_is_error() {
    let call = "module m{ var u,v; sim{ u=0; v=g(u); #init } }";
    for (tag, body, want) in [
        (
            "width_mismatch",
            "reg[2] p; reg[3] q; a-p[0]; p-r-q; q[0]-y;",
            "bus width mismatch",
        ),
        (
            "index_out_of_range",
            "reg[2] p; a-p[5]; p[0]-y;",
            "bus index out of range",
        ),
        (
            "index_on_non_bus",
            "reg z; a-z[0]; z-y;",
            "is not a bus",
        ),
        (
            "bus_width_zero",
            "reg[0] p; a-y;",
            "bus width must be >= 1",
        ),
        (
            "assign_whole_bus",
            "reg[2] p; p = d; a-y;",
            "cannot assign to a whole bus",
        ),
        (
            "bus_as_mid_chunk",
            "reg[2] p; reg z; a-p-z; z-y;",
            "cannot appear inside a chain",
        ),
        // スライス / 連結(issue #43)
        (
            "slice_on_non_bus",
            "reg z; a-z[1:0]; z-y;",
            "is not a bus",
        ),
        (
            "slice_out_of_range",
            "reg[2] p; a-p[0]; p[3:0]-y;",
            "bus slice index out of range",
        ),
        (
            "empty_concat",
            "{} - y; a-y;",
            "empty concatenation",
        ),
        (
            "nested_concat",
            "reg[2] p; a-p[0]; {a, {p}} - y;",
            "nested concatenation",
        ),
        (
            "side_in_concat",
            "reg cmp; {a, cmp.side} - y;",
            "cannot appear inside a concatenation",
        ),
        (
            "concat_as_mid_chunk",
            "reg[2] p; a-p[0]; a - {p} - y;",
            "on a mid-chain chunk",
        ),
    ] {
        let src = format!("logic g(input a, output y){{ {body} }}\n{call}");
        let (code, stderr) = run_source(&format!("bus_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains(want),
            "{tag}: unexpected stderr:\n{stderr}"
        );
    }
}

/// バス reg への素子代入(issue #95)の誤用は **エラー**。初期化子の制約はスカラ named
/// point と同じ(コンパレータ/リピーターのみ、r0 不可、強度不可、plain のみ)で、
/// `.side` の粒度解決(レーン/スライス/非バス/範囲外/幅不一致)も検査する。
#[test]
fn bus_side_reg_misuse_is_error() {
    let call = "module m{ var u,v; sim{ u=0; v=g(u); #init } }";
    for (tag, body, want) in [
        (
            "r0_bus_reg",
            "reg[4] m = r0; a-m; m[0]-y;",
            "a 0-tick repeater (r0) cannot be a lockable reg",
        ),
        (
            "strength_init",
            "reg[4] m = 15; a-m[0]; m[0]-y;",
            "a bus reg cannot take a signal strength",
        ),
        (
            "non_seq_element_init",
            "reg[4] m = t; a-m[0]; m[0]-y;",
            "a bus reg initializer must be a comparator or repeater element",
        ),
        (
            "const_bus_reg",
            "const reg[4] m = r; a-m; m[0]-y;",
            "a bus reg must be plain",
        ),
        (
            "side_on_plain_bus",
            "reg[4] p; a-p[0].side; p[0]-y;",
            "'.side' is only valid on a comparator/repeater reg",
        ),
        (
            "side_as_source",
            "reg[4] m = r; a-m; m.side-y;",
            "cannot be a wire source",
        ),
        (
            "indexed_side_on_scalar_reg",
            "reg c = cd; a-c[0].side; c-y;",
            "is a scalar comparator/repeater reg and cannot be indexed",
        ),
        (
            "side_lane_out_of_range",
            "reg[4] m = r; a-m; a-m[5].side; m[0]-y;",
            "bus index out of range",
        ),
        (
            "side_width_mismatch",
            "reg[2] m = r; reg[4] s; a-s[0]; a-m; s-m.side; m[0]-y;",
            "bus width mismatch",
        ),
    ] {
        let src = format!("logic g(input a, output y){{ {body} }}\n{call}");
        let (code, stderr) = run_source(&format!("busreg_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// スライス / レーン添字の定数式の誤用は **エラー**(issue #89)。
/// ジェネリック param を含む式の範囲外はインスタンス化時、それ以外はパース時に検出する。
#[test]
fn slice_const_expr_misuse_is_error() {
    let call = "module m{ var[4] u; var y; sim{ u=0; y=g(u); #init } }";
    for (tag, body, want) in [
        // ジェネリック式の評価結果が範囲外(インスタンス化時に検出)
        (
            "generic_lane_out_of_range",
            "logic g #(W=4)(input[W] a, output y){ a[W] - y; }",
            "bus index out of range",
        ),
        (
            "generic_slice_out_of_range",
            "logic g #(W=4)(input[W] a, output y){ a[W:0] - y; }",
            "bus slice index out of range",
        ),
        // param でも #define でもない識別子(パース時に検出)
        (
            "unknown_const_in_index",
            "logic g(input[4] a, output y){ a[Q] - y; }",
            "unknown constant 'Q'",
        ),
        // $time は定数式に置けない(パース時に検出)
        (
            "time_in_index",
            "logic g(input[4] a, output y){ a[$time] - y; }",
            "$time is not allowed in a constant expression",
        ),
    ] {
        let src = format!("{body}\n{call}");
        let (code, stderr) = run_source(&format!("selx_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// バスポート + バス var + バス束縛 + 添字 + ブロードキャストは受理される(Phase 1b)。
#[test]
fn bus_ports_basic_is_accepted() {
    // 4 ビット NOT を バスポートで定義し、バス var を束縛・添字・ブロードキャストする。
    let src = "logic not4(input[4] a, output[4] y){ a-t-y; }\n\
               module m{ var[4] x; var[4] y; var i; sim{ x=0; y=not4(x); #init \
               for(i=0;i<4;i=i+1){ x[i]=15; } #2 } }";
    let (code, stderr) = run_source("bus_ports_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// バスポート/バス var の不整合(幅・スカラ混在・出力先・添字・引数・scan・pulse)は
/// **エラー**(Phase 1b)。
#[test]
fn bus_ports_misuse_is_error() {
    for (tag, src, want) in [
        // 入力ポート幅と引数バス幅の不一致
        (
            "arg_width_mismatch",
            "logic g(input[4] a, output y){ a[0]-t-y; }\n\
             module m{ var[2] x; var y; sim{ x=0; y=g(x); #init } }",
            "does not match",
        ),
        // スカラ var をバス入力ポートへ
        (
            "scalar_to_bus_port",
            "logic g(input[4] a, output y){ a[0]-t-y; }\n\
             module m{ var x; var y; sim{ x=0; y=g(x); #init } }",
            "is a scalar var but",
        ),
        // バス出力ポートをスカラ var へ束縛
        (
            "bus_out_to_scalar",
            "logic g(input a, output[4] y){ a-t-y[0]; a-y[1]; a-y[2]; a-y[3]; }\n\
             module m{ var x; var y; sim{ x=0; y=g(x); #init } }",
            "bus output",
        ),
        // バス var を添字なしでスカラ式に使う(monitor 引数は別扱いで合成可、
        // それ以外の文脈では従来どおりエラー)
        (
            "bus_in_scalar_expr",
            "module m{ var[4] x; sim{ x=0; assert(x); } }",
            "is a bus var",
        ),
        // バス var の範囲外添字
        (
            "bus_var_index_oor",
            "module m{ var[2] x; sim{ x=0; x[5]=1; } }",
            "out of range",
        ),
        // 範囲外レーンを logic 引数に渡す(レーン引数自体は issue #118 で合法化)
        (
            "pass_bus_lane_arg_oor",
            "logic g(input a, output y){ a-t-y; }\n\
             module m{ var[2] x; var y; sim{ x=0; y=g(x[5]); #init } }",
            "bus index out of range",
        ),
        // スカラ var に添字を付けて logic 引数に渡す
        (
            "index_scalar_arg",
            "logic g(input a, output y){ a-t-y; }\n\
             module m{ var x; var y; sim{ x=0; y=g(x[0]); #init } }",
            "is not a bus var; cannot index it",
        ),
        // scan() をバス var へ
        (
            "scan_to_bus",
            "module m{ var[2] x; sim{ x=scan(); } }",
            "cannot target a whole bus",
        ),
        // 全バスへのパルス代入
        (
            "pulse_on_bus",
            "module m{ var[2] x; sim{ x = 5 ~ 2; } }",
            "pulse assignment is not supported on a whole bus",
        ),
    ] {
        let (code, stderr) = run_source(&format!("busport_{tag}"), src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// param 定数: 幅(リテラル/param/定数式)・#define 流用・sim 式参照は受理される(issue #41)。
#[test]
fn param_basic_is_accepted() {
    for (tag, src) in [
        // param をバス幅に
        (
            "param_width",
            "param W=2;\nmodule m{ var[W] x; sim{ x=0; monitor(\"%\", x[0]); } }",
        ),
        // 定数式を幅に(W+1 -> 幅 3)
        (
            "param_expr_width",
            "param W=2;\nmodule m{ var[W+1] x; sim{ x=0; monitor(\"%\", x[2]); } }",
        ),
        // param から param を導出
        (
            "param_from_param",
            "param W=4;\nparam H=W*2;\nmodule m{ var x; sim{ x=H; monitor(\"%\", x); } }",
        ),
        // 数値 #define を幅として流用
        (
            "define_as_width",
            "#define W 3\nmodule m{ var[W] x; sim{ x=0; monitor(\"%\", x[2]); } }",
        ),
        // sim 式での param 参照(for 上限)
        (
            "param_in_sim_expr",
            "param W=3;\nmodule m{ var[W] x; var i; sim{ x=0; for(i=0;i<W;i=i+1){ x[i]=15; } } }",
        ),
    ] {
        let (code, stderr) = run_source(&format!("param_{tag}"), src);
        assert_eq!(code, Some(0), "{tag}: expected success, stderr:\n{stderr}");
    }
}

/// param 定数の誤用(未定義参照・幅 < 1・前方参照・式中の禁止構文)は **エラー**(issue #41)。
#[test]
fn param_misuse_is_error() {
    for (tag, src, want) in [
        // 未定義の定数を幅に
        (
            "unknown_in_width",
            "module m{ var[NOPE] x; sim{ x=0; } }",
            "unknown constant",
        ),
        // param から幅 0
        (
            "param_width_zero",
            "param Z=0;\nmodule m{ var[Z] x; sim{ x=0; } }",
            "bus width must be >= 1",
        ),
        // 前方参照(まだ未定義の param)
        (
            "forward_ref",
            "param A=B;\nmodule m{ var u; sim{ u=A; } }",
            "unknown constant",
        ),
        // 定数式での添字は不可
        (
            "index_in_const",
            "param W=4;\nmodule m{ var[W[0]] x; sim{ x=0; } }",
            "constant expression",
        ),
    ] {
        let (code, stderr) = run_source(&format!("parambad_{tag}"), src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// logic ごとのジェネリック幅 `#(W=4)` の正常系(issue #41 Phase 2):
/// 既定値のみ・実引数あり・複数 param・logic 内 `reg[W]`・階層パススルーの 5 ケース。
#[test]
fn generic_logic_width_is_accepted() {
    for (tag, src) in [
        // 既定値のみで呼び出し(`#(...)` 省略 = `#(W=4)`)
        (
            "default_only",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g(x); #init } }",
        ),
        // 実引数で別幅を指定
        (
            "explicit_arg",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[8] x,y; sim{ x=0; y=g#(W=8)(x); #init } }",
        ),
        // 複数 param
        (
            "multi_param",
            "logic g #(W=4, K=2)(input[W] x, output[W] y){ reg[W] s; x-s; s-t-y; }\n\
             module m{ var[8] x,y; sim{ x=0; y=g#(W=8, K=4)(x); #init } }",
        ),
        // logic 内の `reg[W+1]` などの派生幅
        (
            "derived_reg_width",
            "logic g #(W=4)(input[W] x, output[W] y){ reg[W] s; x-s; s-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g(x); #init } }",
        ),
        // 階層: 外側 param を内側 param に渡す
        (
            "passthrough",
            "logic inner #(N=2)(input[N] x, output[N] y){ x-t-y; }\n\
             logic outer #(W=4)(input[W] a, output[W] z){ z = inner#(N=W)(a); }\n\
             module m{ var[8] a,z; sim{ a=0; z = outer#(W=8)(a); #init } }",
        ),
    ] {
        let (code, stderr) = run_source(&format!("genericw_{tag}"), src);
        assert_eq!(code, Some(0), "{tag}: expected success, stderr:\n{stderr}");
    }
}

/// logic ごとのジェネリック幅の誤用は **エラー**(issue #41 Phase 2):
/// 未知 param・既定値なしの未指定・重複・幅 0 / 負の各ケースを検査する。
#[test]
fn generic_logic_width_is_error() {
    for (tag, src, want) in [
        // 呼び出し側に logic に無い param 名
        (
            "unknown_param",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g#(X=2)(x); #init } }",
            "has no parameter 'X'",
        ),
        // 既定値なし、呼び出し側でも未指定
        (
            "missing_required_param",
            "logic g #(W)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g(x); #init } }",
            "requires parameter 'W'",
        ),
        // 呼び出し側の `#(...)` で param 重複
        (
            "dup_param_at_call",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g#(W=4, W=8)(x); #init } }",
            "duplicate logic parameter 'W'",
        ),
        // 宣言側の `#(...)` で param 重複
        (
            "dup_param_at_decl",
            "logic g #(W=4, W=8)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g(x); #init } }",
            "duplicate logic parameter 'W'",
        ),
        // 実引数が 0 幅
        (
            "zero_width",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m{ var[4] x,y; sim{ x=0; y=g#(W=0)(x); #init } }",
            "bus width must be >= 1",
        ),
    ] {
        let (code, stderr) = run_source(&format!("genericwbad_{tag}"), src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// monitor フォーマットの型サフィックス `%t` / `%d`(`%2d` 等の幅付きも)は **エラー**(issue #17)。
/// `%` / `%N` に統一されたため、旧サフィックスは受理しない。
#[test]
fn monitor_fmt_type_suffix_is_error() {
    for (tag, fmt) in [("pct_t", "%t"), ("pct_d", "%d"), ("pct_2d", "%2d")] {
        let src = format!("module m{{ var a; sim{{ a=0; monitor(\"{fmt}\", a); }} }}");
        let (code, stderr) = run_source(&format!("monfmt_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains("type suffix") && stderr.contains("not supported"),
            "{tag}: unexpected stderr:\n{stderr}"
        );
    }
}

/// `%` / `%N`(幅指定)は引き続き受理される(issue #17)。
#[test]
fn monitor_fmt_bare_and_width_is_accepted() {
    let src = "module m{ var a; sim{ a=0; monitor(\"x=% y=%2\\n\", a, a); } }";
    let (code, stderr) = run_source("monfmt_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// `scan(fmt)` の書式が `%` / `%b` / `%x` / `%o` 以外ならエラー(issue #77)。
#[test]
fn scan_fmt_invalid_is_error() {
    let src = "module m{ var a; sim{ a = scan(\"%4b\"); } }";
    let (code, stderr) = run_source("scanfmt_bad", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("scan() format must be one of"),
        "unexpected stderr:\n{stderr}"
    );
}

/// `#define NAME <定数式>` は param と同じ式を受理する(issue #49):
/// 算術 `(W*2)` / ネスト `(N/2)` / 0b・0x リテラルとの混在ができる。
#[test]
fn define_expr() {
    run_golden("define_expr");
}

/// `#define BAD UNKNOWN_NAME` は未定義参照でエラー(issue #49)。
#[test]
fn define_expr_unknown_const_is_error() {
    let src = "#define BAD UNKNOWN\nmodule m{ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("define_unknown", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(stderr.contains("unknown constant"), "unexpected stderr:\n{stderr}");
}

/// `#define MODE` は引き続き ident のみ受理する(将来のモード切替え用予約名, issue #49)。
#[test]
fn define_mode_keeps_ident_path() {
    // 'element' は受理、'logic' は警告だが続行
    let src = "#define MODE element\nmodule m{ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("define_mode_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(stderr.is_empty(), "no warning expected, stderr:\n{stderr}");

    let src = "#define MODE logic\nmodule m{ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("define_mode_warn", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(stderr.contains("MODE 'logic'"), "unexpected stderr:\n{stderr}");
}

/// `--json` モード(issue #49): monitor を JSONL `{"time":N,"values":[...],"fmt":"..."}`
/// で stdout に出す。同一ソースの通常モード出力との対称比較で挙動を固定する。
#[test]
fn json_output_text_mode() {
    // ?monitor + expect の通常モード(整形済み文字列)
    run_golden("json_output");
}

#[test]
fn json_output_json_mode() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/json_output.rv");
    let expected_path = format!("{manifest}/tests/expected/json_output.jsonl");
    let expected =
        std::fs::read(&expected_path).unwrap_or_else(|e| panic!("read {expected_path}: {e}"));
    let out = Command::new(bin())
        .arg("--json")
        .arg(&rv)
        .output()
        .expect("spawn redv");
    assert!(
        out.status.success(),
        "json_output --json: exit {:?}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    // CRLF を取り除いて LF で比較(Windows でも安定)。
    let strip_cr = |v: &[u8]| -> Vec<u8> { v.iter().copied().filter(|&b| b != b'\r').collect() };
    let got = strip_cr(&out.stdout);
    let expected = strip_cr(&expected);
    if got != expected {
        panic!(
            "json_output --json: stdout mismatch\n--- expected ---\n{}\n--- got ---\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&got)
        );
    }
}

/// `--json` モードで assert/expect の失敗を JSONL で stderr に出す(issue #49)。
#[test]
fn json_mode_emits_assert_failure_jsonl() {
    let src = "module m{ var a; sim{ a=0; assert(a>0); expect(a, 7); } }";
    let path = std::env::temp_dir().join("redv_test_json_assert.rv");
    std::fs::write(&path, src).expect("write tmp rv");
    let out = Command::new(bin())
        .arg("--json")
        .arg(&path)
        .output()
        .expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.status.code(), Some(1), "expected failure");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("\"kind\":\"assert\"") && stderr.contains("\"expr\":\"(a > 0)\""),
        "missing assert JSON, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("\"kind\":\"expect\"")
            && stderr.contains("\"actual\":0")
            && stderr.contains("\"expected\":7"),
        "missing expect JSON, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("\"kind\":\"summary\"") && stderr.contains("\"failed\":2"),
        "missing summary JSON, stderr:\n{stderr}"
    );
}

/// `--json` モードで warning を JSONL で出す(issue #49)。
#[test]
fn json_mode_emits_warning_jsonl() {
    let src = "logic g(input x, output y){ x-t-y; }\n\
               module m{ var a,b; sim{ a=999; b=g(a); #init } }";
    let path = std::env::temp_dir().join("redv_test_json_warn.rv");
    std::fs::write(&path, src).expect("write tmp rv");
    let out = Command::new(bin())
        .arg("--json")
        .arg(&path)
        .output()
        .expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.status.code(), Some(0), "expected success");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("\"kind\":\"warning\"") && stderr.contains("\"msg\":"),
        "missing warning JSON, stderr:\n{stderr}"
    );
}

/// 多出力 logic のタプル束縛(issue #79): sim・logic 本体ともに過不足・重複・空は
/// エラー。1 出力 logic の `(v) = ...` も統一性のため許容する。
/// バスレーン target は issue #118 で合法化されたため、重複はレーン単位で検査する。
#[test]
fn multi_output_binding_is_error() {
    // 共通ヘッダ: g は 2 出力 logic、g0 は 1 出力 logic。
    let header = "logic g0(input x, output y) { x - r - y; }\n\
                  logic g(input x, output a1, output a2) {\n\
                  \x20\x20\x20\x20a1 = g0(x);\n\
                  \x20\x20\x20\x20a2 = g0(x);\n\
                  }\n";
    for (tag, body, want) in [
        // 不足: 1 ターゲットで 2 出力 logic を受ける
        (
            "too_few",
            "module m{ var x,p; sim{ x=0; (p) = g(x); #init } }",
            "g has 2 output port(s) but the binding tuple has 1 target(s)",
        ),
        // 過剰: 3 ターゲットで 1 出力 logic を受ける
        (
            "too_many",
            "module m{ var x,p,q,r2; sim{ x=0; (p,q,r2) = g0(x); #init } }",
            "g0 has 1 output port(s) but the binding tuple has 3 target(s)",
        ),
        // 重複 target
        (
            "dup",
            "module m{ var x,p; sim{ x=0; (p,p) = g(x); #init } }",
            "duplicate target 'p' in logic-instance binding tuple",
        ),
        // 空タプル
        (
            "empty",
            "module m{ var x; sim{ x=0; () = g0(x); #init } }",
            "empty target tuple '()' is not allowed",
        ),
        // scan のタプル束縛
        (
            "scan_tuple",
            "module m{ var a,b2; sim{ (a,b2) = scan(); } }",
            "scan() returns a single value",
        ),
        // 同一レーンの字面重複(issue #118: レーン target は合法、重複はエラー)
        (
            "dup_lane",
            "module m{ var x; var[2] bs; sim{ x=0; (bs[0], bs[0]) = g(x); #init } }",
            "duplicate target 'bs[0]' in logic-instance binding tuple",
        ),
        // 解決後レーンの部分重複(バス全体とレーンが同じ点を束縛)
        (
            "overlap_lane",
            "module m{ var x; var[2] bs; sim{ x=0; (bs, bs[0]) = g(x); #init } }",
            "targets 'bs' and 'bs[0]' in logic-instance binding tuple overlap",
        ),
    ] {
        let src = format!("{header}{body}");
        let (code, stderr) = run_source(tag, &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// ネスト呼び出し(issue #97)の正常系: sim・logic 本体・3 段の深いネスト・
/// ジェネリック幅 `#(...)` 付き・バス出力のネストがすべて受理される。
#[test]
fn nested_call_is_accepted() {
    for (tag, src) in [
        // sim 側の基本形(issue の例)
        (
            "sim_basic",
            "#include \"stdlogic\"\n\
             module m{ var x1,x2,x3,x4,y; sim{ x1=0;x2=0;x3=0;x4=0;\n\
             \x20\x20\x20\x20y = s_or(s_and(x1,x2), s_xor(x3,x4)); #init } }"
                .to_string(),
        ),
        // logic 本体
        (
            "logic_body",
            "#include \"stdlogic\"\n\
             logic G(input a, output y) { y = s_or(s_and(a, a), a); }\n\
             module m{ var a,y; sim{ a=0; y=G(a); #init } }"
                .to_string(),
        ),
        // 3 段の深いネスト
        (
            "deep",
            "#include \"stdlogic\"\n\
             module m{ var a,y; sim{ a=0; y = s_not(s_not(s_not(a))); #init } }"
                .to_string(),
        ),
        // ジェネリック幅付き + バス出力のネスト
        (
            "generic_bus",
            "logic inv #(W=4)(input[W] x, output[W] y){ x - t - y; }\n\
             module m{ var[8] a,y; sim{ a=0; y = inv#(W=8)(inv#(W=8)(a)); #init } }"
                .to_string(),
        ),
    ] {
        let (code, stderr) = run_source(&format!("nested_{tag}"), &src);
        assert_eq!(code, Some(0), "{tag}: expected success, stderr:\n{stderr}");
    }
}

/// ネスト呼び出し(issue #97)の誤用はエラー: 多出力 logic のネスト(sim / logic 本体)、
/// 未知 logic のネスト、scan のネスト、ネスト経由の再帰、出力幅の不一致。
#[test]
fn nested_call_is_error() {
    let header = "logic HA(input x1, input x2, output s2, output c2) { x1-t-s2; x2-t-c2; }\n\
                  logic OR2(input x1, input x2, output y) { x1-d-y; x2-d-y; }\n";
    for (tag, body, want) in [
        // 多出力 logic はネストできない(sim)
        (
            "multi_output_sim",
            "module m{ var a,b,q,y; sim{ a=0;b=0;q=0; y = OR2(HA(a,b), q); #init } }",
            "nested call to HA must have exactly 1 output port (it has 2)",
        ),
        // 多出力 logic はネストできない(logic 本体)
        (
            "multi_output_logic",
            "logic G(input x1, input x2, output y) { y = OR2(HA(x1, x2), x1); }\n\
             module m{ var a,b,y; sim{ a=0;b=0; y = G(a,b); #init } }",
            "nested call to HA must have exactly 1 output port (it has 2)",
        ),
        // 未知 logic のネスト
        (
            "unknown_nested",
            "module m{ var a,b,y; sim{ a=0;b=0; y = OR2(NOPE(a), b); #init } }",
            "unknown logic: NOPE",
        ),
        // scan はネストできない
        (
            "nested_scan",
            "module m{ var a,y; sim{ a=0; y = OR2(scan(), a); #init } }",
            "scan() cannot be nested in a logic call argument",
        ),
        // ネスト経由の相互再帰
        (
            "nested_recursion",
            "logic RA(input x, output y) { y = RB(x); }\n\
             logic RB(input x, output y) { y = OR2(RA(x), x); }\n\
             module m{ var a,y; sim{ a=0; y = RA(a); #init } }",
            "recursive logic instantiation",
        ),
        // ネスト出力の幅不一致(バス出力 -> スカラ入力)。幅検査は logic 本体側と
        // 共通の connect_ports に一本化されている(issue #100)。
        (
            "width_mismatch",
            "logic inv #(W=4)(input[W] x, output[W] y){ x - t - y; }\n\
             module m{ var[4] a; var y; sim{ a=0; y = OR2(inv(a), a); #init } }",
            "OR2 input port 'x1': port width mismatch (4 vs 1 lane(s)",
        ),
    ] {
        let src = format!("{header}{body}");
        let (code, stderr) = run_source(&format!("nestedbad_{tag}"), &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// ネスト呼び出しの部分式は standalone 呼び出しと同一インスタンスを共有する(issue #97)。
/// トレース(-t)のノード名に `s_and(W=1)(x1,x2)` のインスタンスが 1 組だけ現れることで確認する
/// (stdlogic のジェネリック幅化(issue #121)以降、キーに既定 param `(W=1)` が付く)。
#[test]
fn nested_call_shares_subexpression_instance() {
    let src = "#include \"stdlogic\"\n\
               module m{ var x1,x2,x3,t,y; sim{ x1=15;x2=15;x3=0;\n\
               \x20\x20\x20\x20t = s_and(x1, x2);\n\
               \x20\x20\x20\x20y = s_or(s_and(x1, x2), x3);\n\
               \x20\x20\x20\x20#init ?monitor(\"t=%2 y=%2\\n\", t, y); #1 } }";
    let (code, stderr) = run_source_args("nested_share", src, &["-t"]);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    // 共有されていれば、ネスト側の s_and は standalone と同じキー `s_and(W=1)(x1,x2)` の
    // インスタンスを使うので、別インスタンス(例: `s_or(...)` 内部の s_and)は現れない。
    assert!(
        stderr.contains("s_and(W=1)(x1,x2).y"),
        "missing shared instance node, stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("/s_and#"),
        "nested s_and must reuse the standalone instance, stderr:\n{stderr}"
    );
}

/// 1 出力 logic を `(v) = callee(...)` のタプル形でも受けられる(統一性。issue #79)。
/// 従来形 `v = callee(...)` も引き続き有効(回帰なし)。
#[test]
fn multi_output_single_target_tuple_is_accepted() {
    let src = "logic g(input x, output y) { x - r - y; }\n\
               module m{ var x, p, q; sim{\n\
               \x20\x20\x20\x20x = 0;\n\
               \x20\x20\x20\x20p = g(x);\n\
               \x20\x20\x20\x20(q) = g(x);\n\
               \x20\x20\x20\x20#init\n\
               } }";
    let (code, stderr) = run_source("multi_out_single_tuple", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 与えたソースを一時ファイルに書き、追加 CLI 引数付きで redv に渡して
/// (終了コード, stderr) を返す(`-W error` 等のフラグ検証用)。
fn run_source_args(tag: &str, src: &str, args: &[&str]) -> (Option<i32>, String) {
    let path = std::env::temp_dir().join(format!("redv_test_{tag}.rv"));
    std::fs::write(&path, src).expect("write temp rv");
    let out = Command::new(bin())
        .args(args)
        .arg(&path)
        .output()
        .expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// lint デモ(issue #48): 警告は stderr へ出るので stdout のゴールデンは不変。
/// sim 自体は完走して exit 0 になる。
#[test]
fn lint_demo() {
    run_golden("lint_demo");
}

/// lint パス(issue #48): デモの 5 ルールがすべて `[lint]` 種別で stderr に出る。
#[test]
fn lint_demo_emits_all_rules() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/lint_demo.rv");
    let out = Command::new(bin()).arg(&rv).output().expect("spawn redv");
    assert_eq!(out.status.code(), Some(0), "lint warnings must not fail the run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for rule in [
        "floating-reg",
        "unused-wire",
        "unused-input",
        "always-on-torch",
        "unreachable-output",
    ] {
        assert!(
            stderr.contains(&format!("[lint] {rule}:")),
            "missing rule {rule}, stderr:\n{stderr}"
        );
    }
}

/// lint パス(issue #48): すべて結線済みのソースには `[lint]` を 1 件も出さない
/// (誤検出ゼロの検証)。
#[test]
fn lint_clean_source_emits_no_lint() {
    let src = "logic g(input x, output y){ reg z; x - t - z; z - r - y; }\n\
               module m{ var a,y; sim{ a=0; y=g(a); #init } }";
    let (code, stderr) = run_source("lint_clean", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(!stderr.contains("[lint]"), "unexpected lint, stderr:\n{stderr}");
}

/// 静的 lint ルール(floating-reg 等)は logic 名につき 1 回。同じ logic を
/// 複数回インスタンス化しても宣言由来の警告は重複しない(issue #48)。
#[test]
fn lint_static_rules_fire_once_per_logic() {
    let src = "logic g(input x, output y){ reg orphan; x - r - y; }\n\
               module m{ var a1,a2,y1,y2; sim{ a1=0; a2=0;\n\
               \x20\x20\x20\x20y1=g(a1); y2=g(a2); #init } }";
    let (code, stderr) = run_source("lint_dedup", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert_eq!(
        stderr.matches("floating-reg").count(),
        1,
        "static lint must fire once per logic, stderr:\n{stderr}"
    );
}

/// `-W error`(issue #48): 完走後に警告(lint 含む)が 1 件でもあれば exit 1、
/// 警告ゼロなら exit 0。
#[test]
fn werror_flag_promotes_warnings() {
    let dirty = "logic g(input x, output y){ reg orphan; x - r - y; }\n\
                 module m{ var a,y; sim{ a=0; y=g(a); #init } }";
    let (code, stderr) = run_source_args("werror_dirty", dirty, &["-W", "error"]);
    assert_eq!(code, Some(1), "expected exit 1, stderr:\n{stderr}");
    assert!(
        stderr.contains("treated as errors"),
        "missing -W error message, stderr:\n{stderr}"
    );

    let clean = "logic g(input x, output y){ x - r - y; }\n\
                 module m{ var a,y; sim{ a=0; y=g(a); #init } }";
    let (code, stderr) = run_source_args("werror_clean", clean, &["-W", "error"]);
    assert_eq!(code, Some(0), "expected exit 0, stderr:\n{stderr}");
}

/// `-W` の不正モード / モード欠落は CLI エラー(exit 2)(issue #48)。
#[test]
fn werror_flag_misuse_exits_2() {
    let src = "module m{ var a; sim{ a=0; } }";
    let (code, _) = run_source_args("werror_bad", src, &["-W", "bogus"]);
    assert_eq!(code, Some(2), "unknown -W mode must exit 2");

    // `-W` が最後の引数(モード欠落)。ファイルは先に渡す。
    let path = std::env::temp_dir().join("redv_test_werror_noarg.rv");
    std::fs::write(&path, src).expect("write temp rv");
    let out = Command::new(bin())
        .arg(&path)
        .arg("-W")
        .output()
        .expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.status.code(), Some(2), "-W without a mode must exit 2");
}

/// `--json` モードで lint は `{"kind":"lint","rule":...}` の JSONL になる(issue #48)。
#[test]
fn json_mode_emits_lint_jsonl() {
    let src = "logic g(input x, output y){ reg orphan; x - r - y; }\n\
               module m{ var a,y; sim{ a=0; y=g(a); #init } }";
    let (code, stderr) = run_source_args("json_lint", src, &["--json"]);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(
        stderr.contains("\"kind\":\"lint\"") && stderr.contains("\"rule\":\"floating-reg\""),
        "missing lint JSON, stderr:\n{stderr}"
    );
}

/// 旧 `dx`(十字ダスト)は廃止(issue #66)。`d` と挙動が同一で `reg` の繋ぎ方で
/// 表現できるため不要。素子列中では `d` のあとの `x` が未知素子として弾かれる。
#[test]
fn dust_cross_dx_is_error() {
    let src =
        "logic g(input x, output y){\n    wire seg;\n    seg = dx;\n    x-seg-y;\n}\nmodule m{ var a,y; sim{ a=15; y=g(a); #init } }";
    let (code, stderr) = run_source("dust_dx", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("unknown element 'x'"),
        "unexpected stderr:\n{stderr}"
    );
}
