# redv (Red Verilog) — A Redstone Circuit HDL Simulator

[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![deps](https://img.shields.io/badge/dependencies-0-brightgreen.svg)](#)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

*[日本語版/Japanese](README.md)*

redv (Red Verilog) is a toolchain that compiles and simulates redstone circuits described as Verilog-style HDL text from the command line.
You describe circuits at the *component level* (below the gate level) by connecting any two points with a chain of components (dust, repeater, torch, comparator, observer).
You can design and verify circuits with nothing but a text editor and a terminal.

```rv
logic NOT(input x, output y) {
    x-t-y;                       // Connect x and y with a single torch
}

module test {
    var x, y;
    sim {
        x = 0;
        y = NOT(x);
        #init                    // Wait until steady state
        x = 10;  #1
        ?monitor("t=% x=% y=%\n", $time, x, y);
    }
}
```

Written in Rust (edition 2021), with **zero dependency crates** — standard library only.
A plain `cargo build` is all you need.

---

## What it does

- **Write circuits as text.** Lay out components as strings and connect two points with a chain.
- **Tick-accurate simulation.** Repeater delay, torch inversion, dust attenuation, and max-value merging follow the game's rules. The fixed-point solver converges deterministically and is order-independent.
- **Verilog-style testbench.** Drive inputs in a `sim` block, advance time with `#init` / `#n` / `#until(cond)` / `wait()`, and observe with `monitor`. `assert` and `expect` let the exit code carry the pass/fail.
- **Buses and parameter constants.** `reg[N]` bundles lanes and wires them in one line; slicing, concatenation, and bus-to-scalar broadcasting are supported. `param W = 4;` lets one definition serve many widths, and `logic g #(W=4)(...)` makes the width itself a per-call generic parameter.
- **Zero dependencies, single binary.** No external crates. `cargo build --release` produces `redv`.
- **Rust-style caret diagnostics.** Errors and warnings print `--> file:line:col` with the source line and a `^` underline. Syntax errors point at the exact column.

## Install and Build

```sh
git clone git@github.com:shore6/redv.git
cd redv
cargo build --release                       # Produces target/release/redv
```

## Usage

```sh
./target/release/redv examples/not_gate.rv             # Compile + simulate
./target/release/redv -t examples/or_gate.rv           # -t: trace all node values every tick to stderr
./target/release/redv --vcd out.vcd examples/clock.rv  # --vcd: dump waveform as VCD
cargo run --release -- examples/clock.rv               # Can run via cargo run too
cargo test                                             # Golden tests for all examples + CLI tests
```

### CLI Options

| Option | Behavior |
|---|---|
| `redv <file.rv>` | Compiles and simulates the circuit (exit code 0 on success, 1 on error) |
| `-t`, `--trace` | Traces all node values every tick to stderr |
| `--vcd <file>` | Dumps the waveform in VCD (Value Change Dump) format to `<file>` (view in GTKWave, etc.). Records public nodes (regs / ports whose name has no `#`) as 4-bit vectors of strength 0–15. Time is the raw tick (includes `#init` settling, same as `-t`). With multiple modules the output is split into `<file>.<module>.vcd` |
| `--json` | Emit monitor / assert / warning as one JSON object per line (JSONL). monitor events go to stdout; assert / expect / warning and the final summary go to stderr. Useful for CI regression diffs and for piping into other tools |
| `-W error` | Treat warnings as errors: after the run completes, exit with code 1 if any warning (including lint) was emitted. Useful for enforcing zero warnings in CI ([docs/LANGUAGE.md §10.4](docs/LANGUAGE.md)) |
| `-T`, `--time` | Prints compile and simulation timings to stderr |
| `-h`, `--help` | Prints usage (exit code 0) |
| `-v`, `--version` | Prints version |
| No args, unknown option, missing file | Prints usage to stderr, exit code 2 |

## Examples

A small selection.
For the full list, see [docs/LANGUAGE.en.md §12](docs/LANGUAGE.en.md).

| File | Contents |
|---|---|
| `examples/not_gate.rv` | A NOT from a single torch |
| `examples/and_gate.rv` | An AND from 3 torches (NOR of NOTs) |
| `examples/comparator_side.rv` | Comparator side input (subtract and compare) |
| `examples/repeater_lock.rv` | Repeater lock (`.side` freezes the output) |
| `examples/half_adder.rv` | Multi-output logic with tuple binding `(sum, carry) = HALF_ADDER(x1, x2);` |
| `examples/nested_call.rv` | Nested calls `y = s_or(s_and(x1,x2), s_xor(x3,x4));` and a one-line MUX |
| `examples/bus_and4.rv` | Bus ports + bus vars: bitwise AND of two 4-bit buses |
| `examples/bus_reg_side.rv` | Bus regs with component assignments `reg[4] m = r;`, wiring `.side` via broadcast / lane / slice |
| `examples/generic_logic_width.rv` | Per-logic generic widths `#(W=4)`: instantiating one definition at multiple widths |
| `examples/slice_const_expr.rv` | Constant expressions in slice / lane indices: splitting a generic-width bus with `x[W-1:W/2]` |
| `examples/numeric_literals.rv` | Binary / hex integer literals (`0b1010` / `0xff`) for strengths, widths, `#define`, sim assignments, and more |
| `examples/define_expr.rv` | Constant expressions in `#define` (e.g. `#define N (W*2)`) |
| `examples/monitor_format.rv` | monitor base formats `%b` / `%x` / `%o` with zero-padding, plus `scan("%x")` for matching input |
| `examples/monitor_bus.rv` | Pass a bus var directly to monitor; each lane is packed as a 4-bit nibble (lane[0] is the lowest) |
| `examples/stdlogic_demo.rv` | `#include "stdlogic"` pulls in the basic gate library (NOT / AND / OR / XOR / NAND / NOR / XNOR) |
| `examples/stdmem_demo.rv` | `#include "stdmem"` pulls in the latch/register library (RS latch / D latch / D-FF / register) |
| `examples/stdmem_generic.rv` | Generic widths in stdmem: `s_register#(W=4)` and friends widen the data path to 4 bits |
| `examples/assert_selfcheck.rv` | Self-checking testbench: `assert` / `expect` return the result via exit code |
| `examples/lint_demo.rv` | Demo that fires all 5 design-rule-check (lint) warnings |
| `examples/vcd_demo.rv` | Demo of dumping the waveform as VCD via `--vcd` |
| `examples/vcd_generic.rv` | Demo of observing generic logic instance ports in the trace / VCD |
| `examples/json_output.rv` | Demo of emitting monitor output as JSONL via `--json` |

## Documentation

- **Language specification** (.rv grammar, components, simulation semantics): [docs/LANGUAGE.en.md](docs/LANGUAGE.en.md) / [docs/LANGUAGE.md](docs/LANGUAGE.md) (Japanese)
- **Internal design** (compile pipeline, elaboration, simulation engine): [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) (Japanese)
- **Style guide** (naming, choosing a circuit style, hierarchical design): [docs/STYLE.md](docs/STYLE.md) (Japanese)

Minimal example:

```rv
logic OR2(input x1, input x2, output y) {
    x1-r-y;          // x1 to y via a repeater
    x2-r-y;          // x2 to y via a repeater (merges at y = max value)
}
```

`reg` / `wire` / port names cannot collide with element names (`d` / `r` / `cd`, etc.).
Such names would be ambiguous inside a chain.
See [docs/LANGUAGE.en.md §2](docs/LANGUAGE.en.md) for details.

## Project Layout

```
src/
  main.rs       CLI entry point
  lexer.rs      Lexical analysis
  parser.rs     Parsing (logic / module / sim / #define / #include)
  ast.rs        AST definitions (data-holding enums)
  circuit.rs    Circuit graph + tick simulation engine
  interp.rs     Elaboration (logic→circuit) + sim runtime + monitor
  diag.rs       Errors and warnings
  stdlib/       Bundled standard library (pulled in by `#include "stdlogic"`, etc.)
examples/       Sample circuits
tests/
  golden.rs     Golden tests (cargo test)
  expected/     Expected outputs
docs/
  LANGUAGE.md       Language spec and simulation semantics (Japanese)
  LANGUAGE.en.md    Language spec and simulation semantics (English)
  ARCHITECTURE.md   Internals (pipeline, modules, simulation engine; Japanese)
  STYLE.md          Style guide (naming, circuit style, hierarchical design; Japanese)
```

## License

MIT
