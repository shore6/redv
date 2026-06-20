//! redv - redstone HDL simulator (CLI)
//!
//! CLI エントリ。`-t/--trace`, `-T/--time`, `-h/--help`, `-v/--version` を受け付ける。

mod ast;
mod circuit;
mod diag;
mod interp;
mod lexer;
mod parser;

use std::process::ExitCode;
use std::time::{Duration, Instant};

/// バージョンは `Cargo.toml` を唯一の正とし、`env!` でビルド時に埋め込む。
const VERSION: &str = concat!("redv ", env!("CARGO_PKG_VERSION"));

fn usage() {
    print!(
        "usage: redv [options] <file.rv>\n\
\n\
Redstone circuit HDL simulator.\n\
\n\
options:\n\
\x20 -t, --trace    dump named node values to stderr every tick\n\
\x20 -T, --time     print compile/sim timings to stderr\n\
\x20     --vcd FILE write a VCD waveform dump to FILE\n\
\x20 -h, --help     show this help\n\
\x20 -v, --version  show version\n"
    );
}

/// `--time` のフェーズ別所要時間を stderr へ出力する。
///
/// - `compile`: 字句解析 + 構文解析 + 回路エラボレーション(tick 実行以外のすべて)。
/// - `sim`:     sim tick 実行(`step()` 不動点エンジン)。複数 module は合算。
/// - `total`:   `compile + sim`。
fn report_time(compile: Duration, sim: Duration, modules: usize) {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let suffix = if modules > 1 {
        format!(" ({} modules)", modules)
    } else {
        String::new()
    };
    eprintln!("[time] {:<8} {:.3} ms", "compile:", ms(compile));
    eprintln!("[time] {:<8} {:.3} ms{}", "sim:", ms(sim), suffix);
    eprintln!("[time] {:<8} {:.3} ms", "total:", ms(compile + sim));
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut file: Option<String> = None;
    let mut trace = false;
    let mut time = false;
    let mut vcd: Option<String> = None;
    // `--vcd FILE` の FILE を次トークンとして待っている状態。
    let mut expect_vcd = false;

    for a in &argv[1..] {
        if expect_vcd {
            vcd = Some(a.clone());
            expect_vcd = false;
        } else if a == "-t" || a == "--trace" {
            trace = true;
        } else if a == "-T" || a == "--time" {
            time = true;
        } else if a == "--vcd" {
            expect_vcd = true;
        } else if let Some(p) = a.strip_prefix("--vcd=") {
            vcd = Some(p.to_string());
        } else if a == "-h" || a == "--help" {
            usage();
            return ExitCode::SUCCESS;
        } else if a == "-v" || a == "--version" {
            println!("{}", VERSION);
            return ExitCode::SUCCESS;
        } else if !a.is_empty() && a.starts_with('-') {
            eprintln!("[error] unknown option: {}", a);
            return ExitCode::from(2);
        } else if file.is_none() {
            file = Some(a.clone());
        } else {
            eprintln!("[error] multiple input files given");
            return ExitCode::from(2);
        }
    }

    if expect_vcd {
        eprintln!("[error] --vcd requires a file path");
        return ExitCode::from(2);
    }

    let file = match file {
        Some(f) => f,
        None => {
            usage();
            return ExitCode::from(2);
        }
    };

    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("[error] cannot open file: {}", file);
            return ExitCode::from(2);
        }
    };

    let result = (|| -> diag::RvResult<(Duration, Duration, interp::RunTimings)> {
        let t0 = Instant::now();
        let mut prog = ast::Program::default();
        let toks = lexer::Lexer::new(src).run()?;
        let mut ps = parser::Parser::new(toks, parser::dir_of(&file));
        ps.parse_file(&mut prog)?;
        let parse_dur = t0.elapsed();
        let t1 = Instant::now();
        let timings = interp::run_program(&prog, trace, vcd.as_deref())?;
        let run_dur = t1.elapsed();
        Ok((parse_dur, run_dur, timings))
    })();

    match result {
        Ok((parse_dur, run_dur, timings)) => {
            if time {
                // run_dur のうち tick 実行以外(エラボレーション + monitor 出力等)を
                // compile に合算する。sim は tick 実行のみ。
                let elaborate = run_dur.saturating_sub(timings.sim);
                report_time(parse_dur + elaborate, timings.sim, timings.modules);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {}", e);
            ExitCode::FAILURE
        }
    }
}
