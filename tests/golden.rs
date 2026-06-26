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

/// バスのスライス `a[hi:lo]`(ビット反転)と連結 `{a, b}`(左ローテート)(issue #43)。
#[test]
fn bus_slice_concat() {
    run_golden("bus_slice_concat");
}

/// 2 進 / 16 進整数リテラル `0b1010` / `0xff`(issue #49):
/// 強度・バス幅・param・#define・sim 代入・tick 数など、従来 10 進が書けた
/// 場所すべてで使えること。
#[test]
fn numeric_literals() {
    run_golden("numeric_literals");
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
#[test]
fn vcd_demo() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rv = format!("{manifest}/examples/vcd_demo.rv");
    let expected_path = format!("{manifest}/tests/expected/vcd_demo.vcd");
    let expected = std::fs::read(&expected_path)
        .unwrap_or_else(|e| panic!("read {expected_path}: {e}"));

    // 並行実行で衝突しないよう PID 入りの一意な一時パスに書く。
    let out_path = std::env::temp_dir().join(format!("redv_vcd_{}.vcd", std::process::id()));
    let out = Command::new(bin())
        .arg("--vcd")
        .arg(&out_path)
        .arg(&rv)
        .output()
        .expect("spawn redv");
    assert!(
        out.status.success(),
        "vcd_demo: exit {:?}\nstderr:\n{}",
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
            "vcd_demo: VCD mismatch\n--- expected ---\n{}\n--- got ---\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&got)
        );
    }
}

/// 与えたソースを一時ファイルに書いて redv に渡し、(終了コード, stderr) を返す。
fn run_source(tag: &str, src: &str) -> (Option<i32>, String) {
    let path = std::env::temp_dir().join(format!("redv_test_{tag}.rv"));
    std::fs::write(&path, src).expect("write temp rv");
    let out = Command::new(bin()).arg(&path).output().expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
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
               module m(){ var a,b; sim{ a=0; b=g(a); #init } }";
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
        ("port_b", "logic g(input a, input b, output y){ a-t-y; }"),
        ("wire_r", "logic g(input a, output y){ wire r; a-t-y; }"),
        ("reg_tb", "logic g(input a, output y){ reg tb; a-t-y; }"),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(
            stderr.contains("collides with an element name"),
            "{tag}: unexpected stderr:\n{stderr}"
        );
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
             module t(){ var u,v; sim{ u=0; v=g(u); #init } }",
            "must be initialized at its declaration",
        ),
        (
            "post_comp",
            "logic g(input a, output y){ reg cmp; a-cmp; cmp=cd; cmp-y; }\n\
             module t(){ var u,v; sim{ u=0; v=g(u); #init } }",
            "must be initialized at its declaration",
        ),
        (
            "post_torch",
            "logic g(input a, output y){ reg z; a-z; z=t; z-y; }\n\
             module t(){ var u,v; sim{ u=0; v=g(u); #init } }",
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
               module t(){ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("decl_rep_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 0tick リピータ(`r0`)は inline チェーン専用。ロック付き reg(`reg m = r0;`)は
/// 保持する状態が無いのでエラーになり、inline 利用へ誘導する(issue #37)。
#[test]
fn zero_tick_repeater_as_reg_is_error() {
    let src = "logic g(input a, output y){ reg m = r0; a-m; m-y; }\n\
               module t(){ var u,v; sim{ u=0; v=g(u); #init } }";
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
               module t(){ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("r0_inline_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// オブザーバ(`o`)は横端子を持たずインラインチェーン専用。reg(`reg p = o;`)に
/// 置こうとするとエラーになり、inline 利用へ誘導する(issue #45)。
#[test]
fn observer_as_reg_is_error() {
    let src = "logic g(input a, output y){ reg p = o; a-p; p-y; }\n\
               module t(){ var u,v; sim{ u=0; v=g(u); #init } }";
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
               module t(){ var u,v; sim{ u=0; v=g(u); #init } }";
    let (code, stderr) = run_source("obs_inline_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 全 assert / expect が真なら exit 0 で「all passed」サマリを出す(issue #40)。
#[test]
fn assert_all_passed_exits_zero() {
    let src = "module m(){ var a; sim{ a=0; assert(a==0); expect(a, 0); } }";
    let (code, stderr) = run_source("assert_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(stderr.contains("all passed"), "unexpected stderr:\n{stderr}");
}

/// 偽の assert は失敗を記録し、末尾サマリ付きで非ゼロ終了する(issue #40)。
#[test]
fn assert_failure_exits_nonzero() {
    let src = "module m(){ var a; sim{ a=0; assert(a > 0); } }";
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
    let src = "module m(){ var a; sim{ a=7; expect(a, 3); } }";
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
    let src = "module m(){ var a; sim{ a=0; assert(a > 0); expect(a, 9); assert(a == 0); } }";
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
            "module m(){ var a; sim{ a=0; assert(a, 1); } }",
            "assert(cond) takes exactly one",
        ),
        (
            "expect_one",
            "module m(){ var a; sim{ a=0; expect(a); } }",
            "expect(actual, expected) takes exactly two",
        ),
    ] {
        let (code, stderr) = run_source(tag, src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// 素子名でない宣言名(`b2` / `cmp` / `x` / `c` 等)は受理される。
#[test]
fn non_element_names_are_accepted() {
    let src = "logic g(input a, input b2, output y){ reg cmp, x, c; a-t-y; }\n\
               module m(){ var u, v; sim{ u=0; v=g(u,u); #init } }";
    let (code, stderr) = run_source("non_elem_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// バス reg(`reg[N]`)+ 添字 `a[k]` + 全体チェーン `p - r - q;` は受理される(issue #11)。
#[test]
fn bus_basic_is_accepted() {
    let src = "logic g(input a, output y){ reg[2] p; reg[2] q; a-p[0]; a-p[1]; p-r-q; q[0]-y; }\n\
               module m(){ var u,v; sim{ u=15; v=g(u); #init } }";
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
                           module m(){{ var u,v; sim{{ u=15; v=g(u); #init }} }}");
        let (code, stderr) = run_source(&format!("bus_bc_{tag}"), &src);
        assert_eq!(code, Some(0), "{tag}: expected success, stderr:\n{stderr}");
    }
}

/// バスチェーンの幅不一致・スカラ混在・範囲外添字・非バス添字は **エラー**(issue #11)。
#[test]
fn bus_misuse_is_error() {
    let call = "module m(){ var u,v; sim{ u=0; v=g(u); #init } }";
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

/// バスポート + バス var + バス束縛 + 添字 + ブロードキャストは受理される(Phase 1b)。
#[test]
fn bus_ports_basic_is_accepted() {
    // 4 ビット NOT を バスポートで定義し、バス var を束縛・添字・ブロードキャストする。
    let src = "logic not4(input[4] a, output[4] y){ a-t-y; }\n\
               module m(){ var[4] x; var[4] y; var i; sim{ x=0; y=not4(x); #init \
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
             module m(){ var[2] x; var y; sim{ x=0; y=g(x); #init } }",
            "does not match",
        ),
        // スカラ var をバス入力ポートへ
        (
            "scalar_to_bus_port",
            "logic g(input[4] a, output y){ a[0]-t-y; }\n\
             module m(){ var x; var y; sim{ x=0; y=g(x); #init } }",
            "is a scalar var but",
        ),
        // バス出力ポートをスカラ var へ束縛
        (
            "bus_out_to_scalar",
            "logic g(input a, output[4] y){ a-t-y[0]; a-y[1]; a-y[2]; a-y[3]; }\n\
             module m(){ var x; var y; sim{ x=0; y=g(x); #init } }",
            "bus output",
        ),
        // バス var を添字なしでスカラ式に使う(monitor 引数は別扱いで合成可、
        // それ以外の文脈では従来どおりエラー)
        (
            "bus_in_scalar_expr",
            "module m(){ var[4] x; sim{ x=0; assert(x); } }",
            "is a bus var",
        ),
        // バス var の範囲外添字
        (
            "bus_var_index_oor",
            "module m(){ var[2] x; sim{ x=0; x[5]=1; } }",
            "out of range",
        ),
        // バスレーンを logic 引数に渡す
        (
            "pass_bus_lane_arg",
            "logic g(input a, output y){ a-t-y; }\n\
             module m(){ var[2] x; var y; sim{ x=0; y=g(x[0]); #init } }",
            "cannot pass a bus lane",
        ),
        // scan() をバス var へ
        (
            "scan_to_bus",
            "module m(){ var[2] x; sim{ x=scan(); } }",
            "cannot target a whole bus",
        ),
        // 全バスへのパルス代入
        (
            "pulse_on_bus",
            "module m(){ var[2] x; sim{ x = 5 ~ 2; } }",
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
            "param W=2;\nmodule m(){ var[W] x; sim{ x=0; monitor(\"%\", x[0]); } }",
        ),
        // 定数式を幅に(W+1 -> 幅 3)
        (
            "param_expr_width",
            "param W=2;\nmodule m(){ var[W+1] x; sim{ x=0; monitor(\"%\", x[2]); } }",
        ),
        // param から param を導出
        (
            "param_from_param",
            "param W=4;\nparam H=W*2;\nmodule m(){ var x; sim{ x=H; monitor(\"%\", x); } }",
        ),
        // 数値 #define を幅として流用
        (
            "define_as_width",
            "#define W 3\nmodule m(){ var[W] x; sim{ x=0; monitor(\"%\", x[2]); } }",
        ),
        // sim 式での param 参照(for 上限)
        (
            "param_in_sim_expr",
            "param W=3;\nmodule m(){ var[W] x; var i; sim{ x=0; for(i=0;i<W;i=i+1){ x[i]=15; } } }",
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
            "module m(){ var[NOPE] x; sim{ x=0; } }",
            "unknown constant",
        ),
        // param から幅 0
        (
            "param_width_zero",
            "param Z=0;\nmodule m(){ var[Z] x; sim{ x=0; } }",
            "bus width must be >= 1",
        ),
        // 前方参照(まだ未定義の param)
        (
            "forward_ref",
            "param A=B;\nmodule m(){ var u; sim{ u=A; } }",
            "unknown constant",
        ),
        // 定数式での添字は不可
        (
            "index_in_const",
            "param W=4;\nmodule m(){ var[W[0]] x; sim{ x=0; } }",
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
             module m(){ var[4] x,y; sim{ x=0; y=g(x); #init } }",
        ),
        // 実引数で別幅を指定
        (
            "explicit_arg",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m(){ var[8] x,y; sim{ x=0; y=g#(W=8)(x); #init } }",
        ),
        // 複数 param
        (
            "multi_param",
            "logic g #(W=4, K=2)(input[W] x, output[W] y){ reg[W] s; x-s; s-t-y; }\n\
             module m(){ var[8] x,y; sim{ x=0; y=g#(W=8, K=4)(x); #init } }",
        ),
        // logic 内の `reg[W+1]` などの派生幅
        (
            "derived_reg_width",
            "logic g #(W=4)(input[W] x, output[W] y){ reg[W] s; x-s; s-t-y; }\n\
             module m(){ var[4] x,y; sim{ x=0; y=g(x); #init } }",
        ),
        // 階層: 外側 param を内側 param に渡す
        (
            "passthrough",
            "logic inner #(N=2)(input[N] x, output[N] y){ x-t-y; }\n\
             logic outer #(W=4)(input[W] a, output[W] z){ z = inner#(N=W)(a); }\n\
             module m(){ var[8] a,z; sim{ a=0; z = outer#(W=8)(a); #init } }",
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
             module m(){ var[4] x,y; sim{ x=0; y=g#(X=2)(x); #init } }",
            "has no parameter 'X'",
        ),
        // 既定値なし、呼び出し側でも未指定
        (
            "missing_required_param",
            "logic g #(W)(input[W] x, output[W] y){ x-t-y; }\n\
             module m(){ var[4] x,y; sim{ x=0; y=g(x); #init } }",
            "requires parameter 'W'",
        ),
        // 呼び出し側の `#(...)` で param 重複
        (
            "dup_param_at_call",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m(){ var[4] x,y; sim{ x=0; y=g#(W=4, W=8)(x); #init } }",
            "duplicate logic parameter 'W'",
        ),
        // 宣言側の `#(...)` で param 重複
        (
            "dup_param_at_decl",
            "logic g #(W=4, W=8)(input[W] x, output[W] y){ x-t-y; }\n\
             module m(){ var[4] x,y; sim{ x=0; y=g(x); #init } }",
            "duplicate logic parameter 'W'",
        ),
        // 実引数が 0 幅
        (
            "zero_width",
            "logic g #(W=4)(input[W] x, output[W] y){ x-t-y; }\n\
             module m(){ var[4] x,y; sim{ x=0; y=g#(W=0)(x); #init } }",
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
        let src = format!("module m(){{ var a; sim{{ a=0; monitor(\"{fmt}\", a); }} }}");
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
    let src = "module m(){ var a; sim{ a=0; monitor(\"x=% y=%2\\n\", a, a); } }";
    let (code, stderr) = run_source("monfmt_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// `scan(fmt)` の書式が `%` / `%b` / `%x` / `%o` 以外ならエラー(issue #77)。
#[test]
fn scan_fmt_invalid_is_error() {
    let src = "module m(){ var a; sim{ a = scan(\"%4b\"); } }";
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
    let src = "#define BAD UNKNOWN\nmodule m(){ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("define_unknown", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(stderr.contains("unknown constant"), "unexpected stderr:\n{stderr}");
}

/// `#define MODE` は引き続き ident のみ受理する(将来のモード切替え用予約名, issue #49)。
#[test]
fn define_mode_keeps_ident_path() {
    // 'element' は受理、'logic' は警告だが続行
    let src = "#define MODE element\nmodule m(){ var a; sim{ a=0; } }";
    let (code, stderr) = run_source("define_mode_ok", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
    assert!(stderr.is_empty(), "no warning expected, stderr:\n{stderr}");

    let src = "#define MODE logic\nmodule m(){ var a; sim{ a=0; } }";
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
    let src = "module m(){ var a; sim{ a=0; assert(a>0); expect(a, 7); } }";
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
               module m(){ var a,b; sim{ a=999; b=g(a); #init } }";
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

/// 多出力 logic のタプル束縛(issue #79): sim・logic 本体ともに過不足・重複・空・
/// バスレーン直指定はエラー。1 出力 logic の `(v) = ...` も統一性のため許容する。
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
            "module m(){ var x,p; sim{ x=0; (p) = g(x); #init } }",
            "g has 2 output port(s) but the binding tuple has 1 target(s)",
        ),
        // 過剰: 3 ターゲットで 1 出力 logic を受ける
        (
            "too_many",
            "module m(){ var x,p,q,r2; sim{ x=0; (p,q,r2) = g0(x); #init } }",
            "g0 has 1 output port(s) but the binding tuple has 3 target(s)",
        ),
        // 重複 target
        (
            "dup",
            "module m(){ var x,p; sim{ x=0; (p,p) = g(x); #init } }",
            "duplicate target 'p' in logic-instance binding tuple",
        ),
        // 空タプル
        (
            "empty",
            "module m(){ var x; sim{ x=0; () = g0(x); #init } }",
            "empty target tuple '()' is not allowed",
        ),
        // scan のタプル束縛
        (
            "scan_tuple",
            "module m(){ var a,b2; sim{ (a,b2) = scan(); } }",
            "scan() returns a single value",
        ),
        // バスレーンを target にできない
        (
            "bus_lane",
            "module m(){ var x; var[2] bs; sim{ x=0; (bs[0], bs[1]) = g(x); #init } }",
            "cannot bind a logic output to a bus lane",
        ),
    ] {
        let src = format!("{header}{body}");
        let (code, stderr) = run_source(tag, &src);
        assert_eq!(code, Some(1), "{tag}: expected failure, stderr:\n{stderr}");
        assert!(stderr.contains(want), "{tag}: unexpected stderr:\n{stderr}");
    }
}

/// 1 出力 logic を `(v) = callee(...)` のタプル形でも受けられる(統一性。issue #79)。
/// 従来形 `v = callee(...)` も引き続き有効(回帰なし)。
#[test]
fn multi_output_single_target_tuple_is_accepted() {
    let src = "logic g(input x, output y) { x - r - y; }\n\
               module m(){ var x, p, q; sim{\n\
               \x20\x20\x20\x20x = 0;\n\
               \x20\x20\x20\x20p = g(x);\n\
               \x20\x20\x20\x20(q) = g(x);\n\
               \x20\x20\x20\x20#init\n\
               } }";
    let (code, stderr) = run_source("multi_out_single_tuple", src);
    assert_eq!(code, Some(0), "expected success, stderr:\n{stderr}");
}

/// 旧 `dx`(十字ダスト)は廃止(issue #66)。`d` と挙動が同一で `reg` の繋ぎ方で
/// 表現できるため不要。素子列中では `d` のあとの `x` が未知素子として弾かれる。
#[test]
fn dust_cross_dx_is_error() {
    let src =
        "logic g(input x, output y){\n    wire seg;\n    seg = dx;\n    x-seg-y;\n}\nmodule m(){ var a,y; sim{ a=15; y=g(a); #init } }";
    let (code, stderr) = run_source("dust_dx", src);
    assert_eq!(code, Some(1), "expected failure, stderr:\n{stderr}");
    assert!(
        stderr.contains("unknown element 'x'"),
        "unexpected stderr:\n{stderr}"
    );
}
