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
