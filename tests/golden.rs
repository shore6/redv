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

#[test]
fn hier_and() {
    run_golden("hier_and");
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

/// 与えたソースを一時ファイルに書いて redv に渡し、(終了コード, stderr) を返す。
fn run_source(tag: &str, src: &str) -> (Option<i32>, String) {
    let path = std::env::temp_dir().join(format!("redv_test_{tag}.rv"));
    std::fs::write(&path, src).expect("write temp rv");
    let out = Command::new(bin()).arg(&path).output().expect("spawn redv");
    let _ = std::fs::remove_file(&path);
    (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
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
            "bus_scalar_mismatch",
            "reg[2] p; reg z; a-p[0]; p-r-z; z-y;",
            "bus/scalar width mismatch",
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
        // バス var を添字なしでスカラ式に使う
        (
            "bus_in_scalar_expr",
            "module m(){ var[4] x; sim{ x=0; monitor(\"%d\", x); } }",
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
