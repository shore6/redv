//! redv - parser
//!
//! C++ 版 `parser.hpp` の移植。再帰下降。`#include` のサブパーサが同じ `Program` を
//! 共有するため、`Program` は各メソッドに `&mut` 引数として渡す(構造体に保持しない)。

use crate::ast::*;
use crate::diag::{fail, warn, RvResult};
use crate::lexer::{Lexer, Tk, Token};
use std::fs;

pub fn dir_of(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(p) => path[..p].to_string(),
        None => ".".to_string(),
    }
}

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    base_dir: String,
}

impl Parser {
    pub fn new(toks: Vec<Token>, base_dir: String) -> Self {
        Parser {
            toks,
            i: 0,
            base_dir,
        }
    }

    pub fn parse_file(&mut self, prog: &mut Program) -> RvResult<()> {
        while self.cur().k != Tk::End {
            if self.is_punct("#") {
                self.i += 1;
                self.parse_directive(prog)?;
            } else if self.is_ident("logic") {
                self.i += 1;
                self.parse_logic(prog)?;
            } else if self.is_ident("module") {
                self.i += 1;
                self.parse_module(prog)?;
            } else {
                return fail(
                    self.cur().line,
                    "expected 'logic', 'module', or '#' directive at top level",
                );
            }
        }
        Ok(())
    }

    // ---- token helpers -----------------------------------------------------

    fn cur(&self) -> &Token {
        &self.toks[self.i]
    }
    fn peek(&self, n: usize) -> &Token {
        let idx = (self.i + n).min(self.toks.len() - 1);
        &self.toks[idx]
    }
    fn is_punct(&self, s: &str) -> bool {
        self.cur().k == Tk::Punct && self.cur().s == s
    }
    fn is_ident(&self, s: &str) -> bool {
        self.cur().k == Tk::Ident && self.cur().s == s
    }
    fn expect_punct(&mut self, s: &str) -> RvResult<()> {
        if !self.is_punct(s) {
            return fail(self.cur().line, format!("expected '{}'", s));
        }
        self.i += 1;
        Ok(())
    }
    fn expect_ident(&mut self, what: &str) -> RvResult<String> {
        if self.cur().k != Tk::Ident {
            return fail(self.cur().line, format!("expected {}", what));
        }
        let r = self.cur().s.clone();
        self.i += 1;
        Ok(r)
    }

    /// reg / wire / ポートの宣言名が素子名(`b` / `r` / `cd` 等)と衝突しないか検査する。
    /// 衝突する名前はチェーン内で素子列と区別できず曖昧になるためエラーにする。
    fn check_decl_name(&self, name: &str, kind: &str, line: i32) -> RvResult<()> {
        if crate::interp::name_collides_with_element(name) {
            return fail(
                line,
                format!(
                    "{} name '{}' collides with an element name (e.g. 'b', 'r', 'cd'); \
                     pick a name that is not an element sequence",
                    kind, name
                ),
            );
        }
        Ok(())
    }

    // ---- directives --------------------------------------------------------

    fn parse_directive(&mut self, prog: &mut Program) -> RvResult<()> {
        let ln = self.cur().line;
        let d = self.expect_ident("directive name after '#'")?;
        if d == "define" {
            let name = self.expect_ident("define name")?;
            match self.cur().k {
                Tk::Num => {
                    let num = self.cur().num;
                    prog.defines.insert(name, num);
                    self.i += 1;
                }
                Tk::Ident => {
                    let v = self.cur().s.clone();
                    self.i += 1;
                    if name == "MODE" {
                        if v != "element" {
                            warn(
                                ln,
                                format!(
                                    "MODE '{}' is not implemented yet; using element mode",
                                    v
                                ),
                            );
                        }
                    } else {
                        prog.str_defines.insert(name, v);
                    }
                }
                _ => return fail(ln, "#define expects a value"),
            }
        } else if d == "include" {
            let fn_;
            match self.cur().k {
                Tk::Str | Tk::Ident => {
                    fn_ = self.cur().s.clone();
                    self.i += 1;
                }
                _ => return fail(ln, "#include expects a file name"),
            }
            if fn_ == "stdlogic" {
                warn(
                    ln,
                    "stdlogic (logic-level mode) is not implemented yet; ignored",
                );
                return Ok(());
            }
            self.include_file(&fn_, ln, prog)?;
        } else {
            return fail(ln, format!("unknown directive: #{}", d));
        }
        Ok(())
    }

    fn include_file(&mut self, fn_: &str, ln: i32, prog: &mut Program) -> RvResult<()> {
        let cands = [
            format!("{}/{}", self.base_dir, fn_),
            format!("{}/{}.rv", self.base_dir, fn_),
            fn_.to_string(),
            format!("{}.rv", fn_),
        ];
        for c in &cands {
            if let Ok(src) = fs::read_to_string(c) {
                let toks = Lexer::new(src).run()?;
                let mut sub = Parser::new(toks, dir_of(c));
                sub.parse_file(prog)?;
                return Ok(());
            }
        }
        fail(ln, format!("cannot open include file: {}", fn_))
    }

    // ---- logic -------------------------------------------------------------

    fn parse_logic(&mut self, prog: &mut Program) -> RvResult<()> {
        let line = self.cur().line;
        let name = self.expect_ident("logic name")?;
        if prog.logics.contains_key(&name) {
            return fail(line, format!("duplicate logic definition: {}", name));
        }
        let mut ports = Vec::new();
        self.expect_punct("(")?;
        if !self.is_punct(")") {
            loop {
                let pl = self.cur().line;
                let pk = self.expect_ident("'input' or 'output'")?;
                if pk != "input" && pk != "output" {
                    return fail(pl, "port must start with 'input' or 'output'");
                }
                // バスポート `input[N]` / `output[N]`(`[N]` は input/output と名前の間)。
                let mut width: Option<i32> = None;
                if self.is_punct("[") {
                    self.i += 1;
                    if self.cur().k != Tk::Num {
                        return fail(self.cur().line, "expected a bus width 'N' after '['");
                    }
                    let n = self.cur().num;
                    self.i += 1;
                    self.expect_punct("]")?;
                    if n < 1 {
                        return fail(pl, format!("bus width must be >= 1 (got {})", n));
                    }
                    width = Some(n as i32);
                }
                let pname = self.expect_ident("port name")?;
                self.check_decl_name(&pname, "port", pl)?;
                ports.push(Port {
                    input: pk == "input",
                    name: pname,
                    line: pl,
                    width,
                });
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
        }
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        while !self.is_punct("}") {
            self.parse_logic_stmt(&mut stmts)?;
        }
        self.expect_punct("}")?;
        prog.logics.insert(
            name.clone(),
            LogicDef {
                name,
                line,
                ports,
                stmts,
            },
        );
        Ok(())
    }

    fn parse_logic_stmt(&mut self, stmts: &mut Vec<LogicStmt>) -> RvResult<()> {
        let ln = self.cur().line;
        if self.is_ident("wire") {
            self.i += 1;
            let mut names = vec![self.expect_ident("wire name")?];
            while self.is_punct(",") {
                self.i += 1;
                names.push(self.expect_ident("wire name")?);
            }
            for n in &names {
                self.check_decl_name(n, "wire", ln)?;
            }
            self.expect_punct(";")?;
            stmts.push(LogicStmt::DeclWire { line: ln, names });
            return Ok(());
        }

        let mut qual = Qual::Plain;
        if self.is_ident("const") {
            qual = Qual::Const;
            self.i += 1;
        } else if self.is_ident("mutable") {
            qual = Qual::Mutable;
            self.i += 1;
        }

        if self.is_ident("reg") {
            self.i += 1;
            // バス幅 `reg[N]`(`[N]` は reg と名前の間に置き、宣言列全体に適用)。
            let mut width: Option<i32> = None;
            if self.is_punct("[") {
                self.i += 1;
                if self.cur().k != Tk::Num {
                    return fail(self.cur().line, "expected a bus width 'N' after 'reg['");
                }
                let n = self.cur().num;
                self.i += 1;
                self.expect_punct("]")?;
                if n < 1 {
                    return fail(ln, format!("bus width must be >= 1 (got {})", n));
                }
                width = Some(n as i32);
            }
            loop {
                let name = self.expect_ident("reg name")?;
                self.check_decl_name(&name, "reg", ln)?;
                let mut init = None;
                if self.is_punct("=") {
                    if width.is_some() {
                        return fail(
                            ln,
                            "a bus reg cannot have an initializer (drive each lane via a chain)",
                        );
                    }
                    self.i += 1;
                    let mut strength = -1;
                    if self.cur().k == Tk::Num {
                        strength = self.cur().num as i32;
                        self.i += 1;
                    }
                    let tok = self.expect_ident("element")?;
                    init = Some(RegInit { strength, tok });
                }
                if width.is_some() && qual != Qual::Plain {
                    return fail(ln, "a bus reg must be plain (const/mutable not supported yet)");
                }
                stmts.push(LogicStmt::DeclReg {
                    line: ln,
                    name,
                    qual,
                    init,
                    width,
                });
                if self.is_punct(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
            self.expect_punct(";")?;
            return Ok(());
        }

        if qual != Qual::Plain {
            return fail(ln, "'const'/'mutable' must be followed by 'reg'");
        }

        // 先頭の識別子(端点には `.side` が付き得る)を読む。
        // 次が '-' ならチェーン接続文、'=' なら代入/インスタンス。
        let head = self.parse_chain_part()?;
        if self.is_punct("-") {
            // チェーン接続文:  from -chunks...- to
            let mut parts: Vec<(String, bool, Option<i32>)> = vec![head];
            while self.is_punct("-") {
                self.i += 1;
                parts.push(self.parse_chain_part()?);
            }
            self.expect_punct(";")?;
            // 中間チャンク(素子列 / wire 名)に `.side` / `[k]` は付けられない
            for (tok, side, idx) in &parts[1..parts.len() - 1] {
                if *side {
                    return fail(
                        ln,
                        format!("'.side' cannot appear on a mid-chain chunk '{}'", tok),
                    );
                }
                if idx.is_some() {
                    return fail(
                        ln,
                        format!("a lane index '[..]' cannot appear on a mid-chain chunk '{}'", tok),
                    );
                }
            }
            let (from, from_side, from_idx) = parts.first().unwrap().clone();
            let (to, to_side, to_idx) = parts.last().unwrap().clone();
            let chunks: Vec<String> = parts[1..parts.len() - 1]
                .iter()
                .map(|(t, _, _)| t.clone())
                .collect();
            stmts.push(LogicStmt::Chain {
                line: ln,
                from,
                from_side,
                from_idx,
                to,
                to_side,
                to_idx,
                chunks,
            });
            return Ok(());
        }
        let (target, target_side, target_idx) = head;
        if target_side {
            return fail(ln, "'.side' is only valid as a chain endpoint");
        }
        if target_idx.is_some() {
            return fail(
                ln,
                "cannot assign to a bus lane '[..]' yet; drive it with a chain (e.g. 'src - a[0];')",
            );
        }
        self.expect_punct("=")?;
        // 階層インスタンス化:  output = callee(args...)
        if self.cur().k == Tk::Ident && self.peek(1).k == Tk::Punct && self.peek(1).s == "(" {
            let callee = self.cur().s.clone();
            self.i += 2; // ident '('
            let mut args = Vec::new();
            if !self.is_punct(")") {
                loop {
                    let an = self.expect_ident("logic input (reg/port name)")?;
                    if self.is_punct("[") {
                        return fail(
                            self.cur().line,
                            format!(
                                "cannot pass a bus lane '{}[..]' as a logic argument; \
                                 pass the whole bus reg/port",
                                an
                            ),
                        );
                    }
                    args.push(an);
                    if self.is_punct(",") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
            }
            self.expect_punct(")")?;
            self.expect_punct(";")?;
            stmts.push(LogicStmt::Instance {
                line: ln,
                output: target,
                callee,
                args,
            });
            return Ok(());
        }
        if self.cur().k == Tk::Num {
            let strength = self.cur().num as i32;
            self.i += 1;
            let rhs = self.expect_ident("element after signal strength")?;
            stmts.push(LogicStmt::AssignSingle {
                line: ln,
                target,
                strength,
                rhs,
            });
        } else {
            // 各パートは「素子チャンク or 端点」。端点には `.side` が付き得る。
            let mut parts: Vec<(String, bool, Option<i32>)> = vec![self.parse_chain_part()?];
            while self.is_punct("-") {
                self.i += 1;
                parts.push(self.parse_chain_part()?);
            }
            // 添字は代入(wire 素子列定義 / エイリアス)では使えない(Phase 1a)。
            for (tok, _side, idx) in &parts {
                if idx.is_some() {
                    return fail(
                        ln,
                        format!(
                            "a lane index '[..]' is not allowed on the right-hand side of '='; \
                             use a chain to wire a bus lane (e.g. '{}[..] - dst;')",
                            tok
                        ),
                    );
                }
            }
            if parts.len() == 1 {
                let (rhs, side, _idx) = parts.into_iter().next().unwrap();
                if side {
                    return fail(ln, "'.side' is only valid as a wire endpoint");
                }
                stmts.push(LogicStmt::AssignSingle {
                    line: ln,
                    target,
                    strength: -1,
                    rhs,
                });
            } else {
                // 中間チャンク(素子列)に `.side` は付けられない
                for (tok, side, _idx) in &parts[1..parts.len() - 1] {
                    if *side {
                        return fail(
                            ln,
                            format!("'.side' cannot appear on a mid-wire element chunk '{}'", tok),
                        );
                    }
                }
                let (from, from_side, _) = parts.first().unwrap().clone();
                let (to, to_side, _) = parts.last().unwrap().clone();
                let chunks: Vec<String> = parts[1..parts.len() - 1]
                    .iter()
                    .map(|(t, _, _)| t.clone())
                    .collect();
                stmts.push(LogicStmt::AssignChain {
                    line: ln,
                    target,
                    from,
                    from_side,
                    to,
                    to_side,
                    chunks,
                });
            }
        }
        self.expect_punct(";")?;
        Ok(())
    }

    /// ワイヤーチェーンの 1 パート(素子チャンク or 端点)を読む。
    /// 端点には `[k]`(バスのレーン添字)と `.side`(コンパレータの横入力端子)が付き得る。
    /// 返り値は (名前, side か, 添字 Option)。素子チャンクには添字・side は付かない。
    fn parse_chain_part(&mut self) -> RvResult<(String, bool, Option<i32>)> {
        let name = self.expect_ident("element chunk or endpoint")?;
        let mut idx = None;
        if self.is_punct("[") {
            let ln = self.cur().line;
            self.i += 1;
            if self.cur().k != Tk::Num {
                return fail(ln, "expected an integer lane index after '['");
            }
            idx = Some(self.cur().num as i32);
            self.i += 1;
            self.expect_punct("]")?;
        }
        let mut side = false;
        if self.is_punct(".") {
            let ln = self.cur().line;
            self.i += 1;
            let suf = self.expect_ident("terminal name after '.'")?;
            if suf != "side" {
                return fail(
                    ln,
                    format!("unknown terminal '.{}' (only '.side' is supported)", suf),
                );
            }
            side = true;
        }
        Ok((name, side, idx))
    }

    // ---- module ------------------------------------------------------------

    fn parse_module(&mut self, prog: &mut Program) -> RvResult<()> {
        let line = self.cur().line;
        let name = self.expect_ident("module name")?;
        self.expect_punct("(")?;
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut pre = Vec::new();
        let mut sim = Vec::new();
        let mut has_sim = false;
        while !self.is_punct("}") {
            if self.is_ident("var") {
                pre.push(self.parse_var_decl()?);
            } else if self.is_ident("sim") {
                self.i += 1;
                self.expect_punct("{")?;
                while !self.is_punct("}") {
                    sim.push(self.parse_sim_stmt()?);
                }
                self.expect_punct("}")?;
                has_sim = true;
            } else {
                return fail(self.cur().line, "expected 'var' or 'sim' in module body");
            }
        }
        self.expect_punct("}")?;
        prog.modules.push(ModuleDef {
            name,
            line,
            pre,
            sim,
            has_sim,
        });
        Ok(())
    }

    fn parse_var_decl(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;
        self.i += 1; // 'var'
        // バス var `var[N]`(`[N]` は var と名前の間。宣言列全体に適用)。
        let mut width: Option<i32> = None;
        if self.is_punct("[") {
            self.i += 1;
            if self.cur().k != Tk::Num {
                return fail(self.cur().line, "expected a bus width 'N' after 'var['");
            }
            let n = self.cur().num;
            self.i += 1;
            self.expect_punct("]")?;
            if n < 1 {
                return fail(ln, format!("bus width must be >= 1 (got {})", n));
            }
            width = Some(n as i32);
        }
        let mut decls = Vec::new();
        loop {
            let n = self.expect_ident("variable name")?;
            let e = if self.is_punct("=") {
                self.i += 1;
                Some(self.parse_expr()?)
            } else {
                None
            };
            decls.push((n, e, width));
            if self.is_punct(",") {
                self.i += 1;
                continue;
            }
            break;
        }
        self.expect_punct(";")?;
        Ok(SimStmt::DeclVar { line: ln, decls })
    }

    fn parse_block_or_single(&mut self) -> RvResult<Vec<SimStmt>> {
        let mut out = Vec::new();
        if self.is_punct("{") {
            self.i += 1;
            while !self.is_punct("}") {
                out.push(self.parse_sim_stmt()?);
            }
            self.expect_punct("}")?;
        } else {
            out.push(self.parse_sim_stmt()?);
        }
        Ok(out)
    }

    fn parse_assign_no_semi(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;
        let target = self.expect_ident("assignment target")?;
        let index = if self.is_punct("[") {
            self.i += 1;
            let e = self.parse_expr()?;
            self.expect_punct("]")?;
            Some(Box::new(e))
        } else {
            None
        };
        self.expect_punct("=")?;
        let value = self.parse_expr()?;
        Ok(SimStmt::Assign {
            line: ln,
            target,
            index,
            value,
            pulse: None,
        })
    }

    fn parse_sim_stmt(&mut self) -> RvResult<SimStmt> {
        let ln = self.cur().line;

        if self.is_punct("#") {
            self.i += 1;
            let s = if self.cur().k == Tk::Num {
                let ticks = self.cur().num;
                self.i += 1;
                SimStmt::WaitTicks { line: ln, ticks }
            } else if self.is_ident("init") {
                self.i += 1;
                SimStmt::WaitInit { line: ln }
            } else {
                return fail(ln, "expected '#<ticks>' or '#init'");
            };
            if self.is_punct(";") {
                self.i += 1; // optional ';'
            }
            return Ok(s);
        }

        if self.is_punct("?") {
            self.i += 1;
            let call = self.parse_call()?;
            if call.callee != "monitor" {
                return fail(ln, "'?' prefix is only supported for monitor()");
            }
            return Ok(SimStmt::MonReg { line: ln, call });
        }

        if self.is_ident("var") {
            return self.parse_var_decl();
        }
        if self.is_ident("sim") {
            return fail(ln, "nested sim block is not allowed");
        }

        if self.is_ident("if") {
            self.i += 1;
            self.expect_punct("(")?;
            let cond = self.parse_expr()?;
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            let else_body = if self.is_ident("else") {
                self.i += 1;
                self.parse_block_or_single()?
            } else {
                Vec::new()
            };
            return Ok(SimStmt::If {
                line: ln,
                cond,
                body,
                else_body,
            });
        }

        if self.is_ident("while") {
            self.i += 1;
            self.expect_punct("(")?;
            let cond = self.parse_expr()?;
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            return Ok(SimStmt::While {
                line: ln,
                cond,
                body,
            });
        }

        if self.is_ident("for") {
            self.i += 1;
            self.expect_punct("(")?;
            let init = if !self.is_punct(";") {
                Some(Box::new(self.parse_assign_no_semi()?))
            } else {
                None
            };
            self.expect_punct(";")?;
            let cond = if !self.is_punct(";") {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.expect_punct(";")?;
            let post = if !self.is_punct(")") {
                Some(Box::new(self.parse_assign_no_semi()?))
            } else {
                None
            };
            self.expect_punct(")")?;
            let body = self.parse_block_or_single()?;
            return Ok(SimStmt::For {
                line: ln,
                init,
                cond,
                post,
                body,
            });
        }

        if self.cur().k == Tk::Ident {
            let name = self.cur().s.clone();
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "(" {
                let call = self.parse_call()?;
                return Ok(SimStmt::Call { line: ln, call });
            }
            // バス var のレーン代入:  name[idx] = value [~ pulse];
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "[" {
                self.i += 1; // name
                self.expect_punct("[")?;
                let idx = self.parse_expr()?;
                self.expect_punct("]")?;
                self.expect_punct("=")?;
                let value = self.parse_expr()?;
                let pulse = if self.is_punct("~") {
                    self.i += 1;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_punct(";")?;
                return Ok(SimStmt::Assign {
                    line: ln,
                    target: name,
                    index: Some(Box::new(idx)),
                    value,
                    pulse,
                });
            }
            if self.peek(1).k == Tk::Punct && self.peek(1).s == "=" {
                self.i += 2;
                // CallBind?  target = callee(args...)
                if self.cur().k == Tk::Ident
                    && self.peek(1).k == Tk::Punct
                    && self.peek(1).s == "("
                {
                    let callee = self.cur().s.clone();
                    self.i += 2; // ident '('
                    let mut bind_args = Vec::new();
                    if !self.is_punct(")") {
                        loop {
                            let an =
                                self.expect_ident("variable name (logic inputs must be vars)")?;
                            if self.is_punct("[") {
                                return fail(
                                    self.cur().line,
                                    format!(
                                        "cannot pass a bus lane '{}[..]' as a logic argument; \
                                         pass the whole bus var (or copy the lane to a scalar var first)",
                                        an
                                    ),
                                );
                            }
                            bind_args.push(an);
                            if self.is_punct(",") {
                                self.i += 1;
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_punct(")")?;
                    self.expect_punct(";")?;
                    return Ok(SimStmt::CallBind {
                        line: ln,
                        target: name,
                        callee,
                        bind_args,
                    });
                }
                // plain Assign(`~ width` を付けるとパルス代入)
                let value = self.parse_expr()?;
                let pulse = if self.is_punct("~") {
                    self.i += 1;
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.expect_punct(";")?;
                return Ok(SimStmt::Assign {
                    line: ln,
                    target: name,
                    index: None,
                    value,
                    pulse,
                });
            }
        }

        fail(ln, "unexpected token in sim block")
    }

    /// callee が現在位置にある呼び出し `name ( ... ) ;` を解析。末尾 ';' まで消費。
    fn parse_call(&mut self) -> RvResult<CallData> {
        let callee = self.expect_ident("function name")?;
        let mut has_fmt = false;
        let mut fmt = String::new();
        let mut args = Vec::new();
        self.expect_punct("(")?;
        if !self.is_punct(")") {
            if self.cur().k == Tk::Str {
                has_fmt = true;
                fmt = self.cur().s.clone();
                self.i += 1;
                if self.is_punct(",") {
                    self.i += 1;
                } else if !self.is_punct(")") {
                    return fail(
                        self.cur().line,
                        "expected ',' or ')' after format string",
                    );
                }
            }
            if !self.is_punct(")") {
                loop {
                    args.push(self.parse_expr()?);
                    if self.is_punct(",") {
                        self.i += 1;
                        continue;
                    }
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        self.expect_punct(";")?;
        Ok(CallData {
            callee,
            has_fmt,
            fmt,
            args,
        })
    }

    // ---- expressions -------------------------------------------------------

    fn mk_bin(op: &str, a: Expr, b: Expr, ln: i32) -> Expr {
        Expr::Bin {
            line: ln,
            op: op.to_string(),
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn parse_expr(&mut self) -> RvResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_and()?;
        while self.is_punct("||") {
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin("||", a, self.parse_and()?, ln);
        }
        Ok(a)
    }
    fn parse_and(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_eq()?;
        while self.is_punct("&&") {
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin("&&", a, self.parse_eq()?, ln);
        }
        Ok(a)
    }
    fn parse_eq(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_rel()?;
        while self.is_punct("==") || self.is_punct("!=") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_rel()?, ln);
        }
        Ok(a)
    }
    fn parse_rel(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_add()?;
        while self.is_punct("<") || self.is_punct("<=") || self.is_punct(">") || self.is_punct(">=")
        {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_add()?, ln);
        }
        Ok(a)
    }
    fn parse_add(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_mul()?;
        while self.is_punct("+") || self.is_punct("-") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_mul()?, ln);
        }
        Ok(a)
    }
    fn parse_mul(&mut self) -> RvResult<Expr> {
        let mut a = self.parse_unary()?;
        while self.is_punct("*") || self.is_punct("/") || self.is_punct("%") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            a = Self::mk_bin(&op, a, self.parse_unary()?, ln);
        }
        Ok(a)
    }
    fn parse_unary(&mut self) -> RvResult<Expr> {
        if self.is_punct("-") || self.is_punct("!") {
            let op = self.cur().s.clone();
            let ln = self.cur().line;
            self.i += 1;
            return Ok(Expr::Un {
                line: ln,
                op,
                a: Box::new(self.parse_unary()?),
            });
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> RvResult<Expr> {
        let ln = self.cur().line;
        if self.cur().k == Tk::Num {
            let num = self.cur().num;
            self.i += 1;
            return Ok(Expr::Num { line: ln, num });
        }
        if self.is_punct("$") {
            self.i += 1;
            let n = self.expect_ident("system variable name after '$'")?;
            if n != "time" {
                return fail(ln, format!("unknown system variable: ${}", n));
            }
            return Ok(Expr::Time { line: ln });
        }
        if self.cur().k == Tk::Ident {
            let name = self.cur().s.clone();
            self.i += 1;
            // バス var のレーン参照 `name[expr]`(添字は実行時に評価)。
            let index = if self.is_punct("[") {
                self.i += 1;
                let e = self.parse_expr()?;
                self.expect_punct("]")?;
                Some(Box::new(e))
            } else {
                None
            };
            return Ok(Expr::Var {
                line: ln,
                name,
                index,
            });
        }
        if self.is_punct("(") {
            self.i += 1;
            let e = self.parse_expr()?;
            self.expect_punct(")")?;
            return Ok(e);
        }
        fail(ln, "expected expression")
    }
}
