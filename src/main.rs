//! redv - redstone HDL simulator (CLI)
//!
//! C++ 版 `main.cpp` の移植。`-t/--trace`, `-h/--help`, `-v/--version` を受け付ける。

mod ast;
mod circuit;
mod diag;
mod interp;
mod lexer;
mod parser;

use std::process::ExitCode;

const VERSION: &str = "redv 0.1.0";

fn usage() {
    print!(
        "usage: redv [options] <file.rv>\n\
\n\
Redstone circuit HDL simulator.\n\
\n\
options:\n\
\x20 -t, --trace    dump named node values to stderr every tick\n\
\x20 -h, --help     show this help\n\
\x20 -v, --version  show version\n"
    );
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut file: Option<String> = None;
    let mut trace = false;

    for a in &argv[1..] {
        if a == "-t" || a == "--trace" {
            trace = true;
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

    let result = (|| -> diag::RvResult<()> {
        let mut prog = ast::Program::default();
        let toks = lexer::Lexer::new(src).run()?;
        let mut ps = parser::Parser::new(toks, parser::dir_of(&file));
        ps.parse_file(&mut prog)?;
        interp::run_program(&prog, trace)?;
        Ok(())
    })();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[error] {}", e);
            ExitCode::FAILURE
        }
    }
}
