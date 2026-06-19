# redv (rv) — A Redstone Circuit HDL Simulator

[![Rust](https://img.shields.io/badge/Rust-edition_2021-orange.svg)](https://www.rust-lang.org/)
[![deps](https://img.shields.io/badge/dependencies-0-brightgreen.svg)](#)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

*[日本語版/Japanese](README.md)*

A toolchain that lets you design redstone circuits **as text**, like Verilog,
then compile and simulate them from the command line. It works at the *component level*
(even lower than the gate level): you describe a circuit by **connecting any two points
with a wire made of a chain of components**.

```rv
logic not_gate(input x, output y) {
    x-t-y;              // Connect x and y with a single torch → that alone is a NOT
}

module test() {
    var x, y;
    sim {
        x = 0;
        y = not_gate(x);                 // Instantiate and bind variables
        #init                            // Wait until steady state ($time = 0)
        x = 10;  #1 #1                   // Drive the input high and advance 2 ticks
        ?monitor("t=% x=% y=%\n", $time, x, y);
    }
}
```

Written in Rust (edition 2021), with **zero dependency crates** — standard library only.
A plain `cargo build` is all you need.

---

## Features

- **Write circuits as text** — Lay out redstone components (dust, repeater, torch,
  comparator, block) as strings and just connect two points with a wire to form a circuit.
- **Tick-accurate simulation** — Deterministic tick simulation that follows the game's
  rules: repeater delay, torch inversion, dust attenuation, max-value merging, and more.
- **Verilog-style testbench** — Drive inputs in a `sim` block, advance time with
  `#init` / `#n` / `wait()`, and observe with `monitor`. `if` / `while` / `for` and pulse
  assignment `a = v ~ w;` (auto-resets to 0 after w ticks) are available.
- **Buses (`reg[N]` / `input[N]` / `output[N]` / `var[N]`)** — Declare several lanes as one
  bundle and wire them all in a single line: `in - r - buf;` (simplifies dense circuits). Pick a
  single lane with `a[k]`, and bind **multi-I/O** directly through bus ports and sim bus vars.
- **Zero dependencies, single binary** — No external crates. `cargo build --release`
  produces `redv`.
- **Strict diagnostics** — Reports out-of-range signals, unconnected outputs, invalid
  components, non-convergent oscillation, and more as errors / warnings.

## Install and Build

```sh
cargo build --release            # Produces target/release/redv
```

## Usage

```sh
./target/release/redv examples/not_gate.rv        # Compile + simulate
./target/release/redv -t examples/or_gate.rv      # -t: trace all node values every tick to stderr
cargo run --release -- examples/clock.rv          # Can also run via cargo run
cargo test                                        # Golden tests for all examples + CLI tests
```

### CLI Options

| Option | Behavior |
|---|---|
| `redv <file.rv>` | Compile and simulate the circuit (exit code 0 on success, 1 on error) |
| `-t`, `--trace` | Trace all node values every tick to stderr |
| `-T`, `--time` | Print compile / simulation timings to stderr |
| `-h`, `--help` | Show usage (exit code 0) |
| `-v`, `--version` | Show version |
| No args / unknown option / missing file | Print usage to stderr, exit code 2 |

## Examples

| File | Contents |
|---|---|
| `examples/not_gate.rv` | A NOT from a single torch |
| `examples/or_gate.rv` | An OR from 2 repeaters + dust merge |
| `examples/and_gate.rv` | An AND from 3 torches (NOR of NOTs) |
| `examples/decay.rv` | Comparison of dust attenuation / repeater re-amplification / comparator strength pass-through |
| `examples/counter_test.rv` | Automatically verifies the AND truth table with `for` / `if` |
| `examples/clock.rv` | A torch + repeater-4 clock (period 10). Example of `wait()` |
| `examples/scan_and.rv` | Reads 2 values from stdin with `scan()` and feeds them into an AND |
| `examples/hier_and.rv` | A hierarchical AND nesting `not_gate` / `or_gate` (De Morgan) |
| `examples/chain_mixed.rv` | Merging two chain paths into the same point (max) |
| `examples/comparator_side.rv` | Comparator side input (`cd` subtract / `cc` compare) |
| `examples/repeater_lock.rv` | Repeater lock (`.side` on `reg m = r;` freezes the output) |
| `examples/wire_reuse.rv` | Define a wire as a reusable component sequence used in several places |
| `examples/pulse.rv` | Pulse assignment (`a = v ~ w;`) auto-resets the var to 0 after w ticks |
| `examples/bus_or4.rv` | Bus `reg[N]`: wire all 4 lanes in one line with `in - r - buf;` |
| `examples/bus_and4.rv` | Bus ports + bus vars: bitwise AND of two 4-bit buses (multi-I/O) |

## Project Layout

```
src/
  main.rs       CLI entry point
  lexer.rs      Lexical analysis
  parser.rs     Parsing (logic / module / sim / #define / #include)
  ast.rs        AST definitions (data-holding enums)
  circuit.rs    Circuit graph + tick simulation engine
  interp.rs     Elaboration (logic→circuit) + sim runtime + monitor
  diag.rs       Errors / warnings
examples/       Sample circuits
tests/
  golden.rs     Golden tests (cargo test)
  expected/     Expected output
docs/
  LANGUAGE.md   Detailed language spec + simulation semantics
```

## Language Specification

For details on circuit definitions, components, wires, the `sim` block, directives, and
simulation semantics, see **[docs/LANGUAGE.md](docs/LANGUAGE.md)** (Japanese).

Minimal example:

```rv
logic or_gate(input a, input b2, output y) {
    a-r-y;          // a to y via a repeater
    b2-r-y;         // b to y via a repeater (merges at y = max value)
}
```

> `reg` / `wire` / port names may not collide with element names (`b` / `r` / `cd`, etc.);
> such names would be ambiguous inside a chain (see [docs/LANGUAGE.md](docs/LANGUAGE.md) §2).

## License

MIT
