# redv Language Specification

This document defines the grammar of `.rv` files and the behavior (semantics) of the simulation engine.
For an overview and build instructions, see [../README.en.md](../README.en.md).

This document is the primary reference for *what* redv does.
For *how* the implementation realizes it, see [ARCHITECTURE.md](ARCHITECTURE.md) (Japanese).

## Table of Contents

- §1 Overview
- §2 Names and Declarators
- §3 Qualifiers and Initialization
- §4 Components
- §5 Wires and Connections
- §6 Buses
- §7 The `sim` Block
- §8 Directives
- §9 Simulation Semantics
- §10 Errors and Warnings
- §11 Differences from the Game
- §12 Sample List

---

## 1. Overview

### 1.1 Top-level Structure

An `.rv` file is built from four kinds of top-level elements.

- **`logic name(input x, output y) { ... }`**: defines a redstone circuit
- **`module name() { ... }`**: a testbench; contains `var` declarations and a `sim { ... }` block
- **`param NAME = <const-expr>;`**: a top-level integer constant (§8.3)
- **`#define` / `#include`** and other directives (§8.1, §8.2)

All `module` definitions in a file run in declaration order.
Each circuit is independent per module; a circuit built in one module is not visible from another.

Statements end with `;`, blocks use `{}`, and comments are `//` (line) and `/* */` (range).

### 1.2 Minimal Example

The smallest possible round trip: build a NOT from a single torch, drive its input, and observe.

```rv
logic NOT(input x, output y) {
    x-t-y;                       // Connect x and y with a torch
}

module test() {
    var x, y;
    sim {
        x = 0;
        y = NOT(x);         // Instantiate and bind variables to I/O
        #init                    // Wait until steady state ($time = 0)
        x = 10;  #1
        ?monitor("t=% x=% y=%\n", $time, x, y);
    }
}
```

Connecting components with `-` (the chain statement) is the basis of circuit description (§5.1).
The `sim` block syntax is covered in §7.

### 1.3 Numeric Literals

Integer literals can be written in decimal, binary, or hexadecimal.

| Form | Examples | Value |
|---|---|---|
| Decimal | `10`, `255` | as written |
| Binary | `0b1010`, `0b1111` | 10, 15 |
| Hexadecimal | `0xf`, `0xff`, `0x10` | 15, 255, 16 |

All three forms produce the same integer token from the lexer.
They are interchangeable anywhere an integer is accepted: strengths, bus widths, `param`, `#define`, sim expressions, tick counts, and so on.

When the `0b` / `0x` prefix is not followed by a valid digit (`0b` alone, `0x` alone, or `0xg` where the following byte is not a digit), the literal interpretation is rejected and the input is split into `0` and the rest as separate tokens.
This split preserves existing forms such as the strength-0 block declaration `const reg n = 0b;`.

To write a strength-15 block in hex, separate `0xf` from `b` with whitespace (`const reg hi = 0xf b;`).
`0xfb` is lexed as `251` and exceeds the strength range (0–15), producing an error.
A binary literal followed by a `2`–`9` decimal digit (e.g. `0b12`) is rejected as a typo.

---

## 2. Names and Declarators

This section defines how to declare the *points* that appear in a circuit and the *variables* used inside a testbench.

### 2.1 List of Declarators

The table summarizes each declarator.
For details, see §3 (qualifiers and initialization), §4 (component notation), and §6 (buses).

| Declarator | Meaning |
|---|---|
| `input` / `output` | Port. Treated as a point (same nature as `reg`) |
| `wire` | A reusable component sequence (a template with no endpoints, §5.2) |
| `reg` | A point. A single component can be assigned to it (§3) |
| `reg[N] a;` | A bus (N parallel lanes, §6) |
| `const reg n = 15b;` | A constant whose component and value are both fixed; the strength is required (§3) |
| `mutable reg n = d;` | The component is fixed; the value changes as the circuit runs (§3) |
| `var` | An integer variable for testbenches; usable only in `sim`, not embeddable in a circuit |
| `param NAME = <const-expr>;` | A top-level integer constant (§8.3) |

### 2.2 Name Constraints

The name of a `reg`, `wire`, or port (`input` / `output`) must not collide with a component name.
Specifically, any name that as a whole can be interpreted as a component sequence is forbidden.

Examples of colliding names:

- Single components: `b` / `d` / `o` / `r` / `t`
- Comparators: `cc` / `cd`
- Observer edge variants: `op` / `on` / `oe`
- Notations with a count: `r2` / `d3`
- Concatenations of the above: `tb` (= torch + block), and similar

If such names were allowed, a chain could not tell a named point apart from a component sequence.
Use names that cannot be parsed as component sequences, like `b2` / `cmp` / `in_b`.
Single letters `a` / `x` / `c` are not component names and may be used.

A `var` lives in a separate namespace from chains, so this constraint does not apply to it.

---

## 3. Qualifiers and Initialization

A `reg` declaration accepts a qualifier and an initializer.
The three combinations differ in how much of "the component and the value" is fixed.

### 3.1 plain (no qualifier)

```rv
reg a;                 // A bare point; no component decided yet
reg c = r2;            // A single component (a 2-tick repeater) assigned as the element
reg m = r;             // A named point that is a lockable repeater (§5.3)
reg cmp = cd;          // A named point that is a subtract comparator (§5.3)
```

Both the component and the value are decided by chain statements (§5.1).
This is the most flexible form of declaration.

### 3.2 const

```rv
const reg n = 15b;     // A single block, locked at strength 15
const reg z = 3d;      // A constant strength equivalent to 3 dusts (8 = 15 attenuated 3 steps)
```

A `const reg` is immutable in both component and value.
The strength must be given, and values outside 0–15 are errors (§10).

### 3.3 mutable

```rv
mutable reg n = d;     // Component is fixed (dust), value follows the circuit
```

A `mutable reg` fixes the component while still letting the value follow the circuit's drive.

### 3.4 Constraints on Comparator and Repeater `reg`

`reg cmp = cd;` and `reg m = r;` are special declarations that create a *named point* for a comparator or a lockable repeater (§5.3).
This form carries the following restrictions.

- Only the plain qualifier is allowed (`const` / `mutable` are not)
- A strength literal cannot be attached
- It must be initialized at declaration time (`reg cmp; cmp = cd;` after-the-fact assignment is not allowed)

Restricting the form to initialization-at-declaration is what fixes the three endpoints (back, side, out; §5.3) at declaration time.

---

## 4. Components

This section lists the minimal building blocks of a circuit: the components.
Each component is used inside a chain statement (§5.1).
Behavioral differences from the actual game are collected in §11.

### 4.1 Summary Table

| Notation | Component | Behavior | Detail |
|---|---|---|---|
| `d` | Dust | Immediate propagation, −1 strength per piece | §4.2 |
| `dn` (e.g. `d3`) | n dusts | Same as `ddd...` | §4.2 |
| `r`, `r1`–`r4` | Repeater | n-tick delay; output re-amplified to 15 | §4.3 |
| `r0` | 0-tick repeater | Zero-delay re-amplification, `out = in > 0 ? 15 : 0` | §4.3 |
| `t` | Torch | Inverts with a 1-tick delay | §4.4 |
| `cc` / `cd` | Comparator (compare / subtract) | 1-tick delay; behavior depends on the side input | §4.5 |
| `b` | Block | Immediate; strength is only 0 or 15 | §4.6 |
| `o` | Observer | Detects input changes and emits a 1-tick pulse | §4.7 |
| `op` / `on` / `oe` | Observer (edge variants) | Detects only rising / falling / binary edges | §4.7.1 |

`r` is the same as `r1` (delay 1).
`r5` and higher are errors, and `c` alone (no mode given) is also an error.

### 4.2 Dust (`d`)

Dust is an immediately propagating attenuator.
It lowers the strength by 1 per piece, and merges by maximum.

A notation like `d3` is equivalent to `ddd`.
In the game, the placement gives dust shapes like "point" or "cross", but this system does not carry such shapes (§11.1).

### 4.3 Repeater (`r`, `r1`–`r4`, `r0`)

A repeater re-amplifies the input to 15.
It can be used in two ways: placed inline in a chain, or made into a *named point* via `reg m = r;` so that a lock input can be attached (§5.3).

The normal repeaters `r1`–`r4` update the output with an n-tick delay.
They also hold 1-tick pulses (an input that vanishes next tick still appears at the output).

`r0` is a zero-delay combinational amplifier.
Where `r1`–`r4` delay output by n ticks, `r0` propagates input changes to the output within the same tick.
The output rule is `out = in > 0 ? 15 : 0`; the re-amplification to 15 is the same as a normal repeater.

```rv
x - d4 - r0 - d4 - y;          // 4-step decay → same-tick 15 re-amp → 4-step decay = 11
```

`r0` has the following restrictions.

- Inline-only; it cannot become a lockable reg (`reg m = r0;` is rejected)
- Locking freezes the previous tick's output, and `r0` has no state to keep

`r0` is monotone in its input, so it rides the combinational network's MAX fixed-point loop directly, converging deterministically without depending on traversal order.
A 0-tick torch or comparator is non-monotone and is handled separately (§11.4). At this time it is not implemented.

### 4.4 Torch (`t`)

A torch inverts its input with a 1-tick delay.

- Output is 0 when the input is `> 0`
- Output is 15 when the input is `= 0`

Toggling many times in a short window triggers a burnout warning and a forced-OFF period (`BURNOUT_*` in §8.1, §10).

### 4.5 Comparator (`cc`, `cd`)

A comparator is a 1-tick-delay component with two modes.

- `cc` (compare): `out = back >= side ? back : 0`
- `cd` (subtract): `out = max(0, back − side)`

When the side input is not connected, `side = 0` is assumed.
In that case both modes degenerate into a pass-through (`out = back`), so the comparator can be used safely as a relay in a chain.

```rv
x - d4 - cc - d4 - y;          // Inline cc and cd act as relays (side = 0)
```

To connect a side input, turn the comparator into a *named point* (§5.3).

### 4.6 Block (`b`)

A block is an immediately propagating 2-value component.
It outputs 15 when the input is `> 0`, and 0 when `= 0` (no intermediate strength).

### 4.7 Observer (`o`)

An observer is a sequential element that emits a 1-tick pulse when its input changes.
If the current sample differs from the previous tick, it outputs strength 15 for one tick; otherwise it holds 0.

Just like the game, every change — rising, falling, or strength change (e.g. 5 → 10) — is treated as a "change".

```rv
x - o - y;                     // The tick after x changes, y is 15 for exactly one tick
```

The semantics are `out(T) = in(T-2) != in(T-1) ? 15 : 0`.
The output sits on a 2-tick history: one tick for the input to reach the circuit, and another for the neighbor-sample comparison.
So a 1-tick-wide pulse appears one tick after an input change.

The observer is inline-only.
Like the torch, it cannot be put on a reg (`reg p = o;` is rejected).
If the input is steady (no change), the output settles to 0, so `#init` (§7.2) terminates (no oscillation).

#### 4.7.1 Edge Variants (`op`, `on`, `oe`)

Three variants replace only the edge condition via a suffix letter.
Everything else — the 1-tick pulse, strength 15, inline-only placement, no reg form — is shared with the base `o`.

| Notation | Detects | Rule |
|---|---|---|
| `op` | Rising edges only | `out(T) = (in(T-2) == 0 && in(T-1) > 0) ? 15 : 0` |
| `on` | Falling edges only | `out(T) = (in(T-2) > 0 && in(T-1) == 0) ? 15 : 0` |
| `oe` | Binary edges | `out(T) = ((in(T-2) > 0) != (in(T-1) > 0)) ? 15 : 0` |

The suffixes correspond to Verilog's posedge / negedge / edge.
`oe` differs from the base `o` in that it picks up only 0↔positive toggles and ignores strength changes (e.g. 5 → 10).
`op` suits one-shot startup (one pulse on power-on); `on` suits power-off detection.

```rv
x - op - y;                    // The tick after x rises, y gets a 1-tick pulse
```

The suffixes are chosen from letters outside the existing component set `b`/`c`/`d`/`o`/`r`/`t`.
For example `od` already means "observer + dust" as a component sequence, so it cannot be the falling-edge suffix.
The variant spellings are component sequences themselves, so the name-collision rule (§2.2) forbids `on` and the like as reg / wire / port names.
The edge variants are an extension not present in the game's observer (§11.5).

---

## 5. Wires and Connections

This section covers the syntax used to wire two points together with a chain of components.
The chapter breaks into four layers: the basic chain (§5.1), the reusable `wire` template (§5.2), named points with side inputs (§5.3), and circuit nesting (§5.4).

### 5.1 Chain Statements

To wire two regs or ports together with a component sequence, write a chain statement directly.
The form is `from -components- to ;`.
No wire declaration is needed; the component sequence is placed inline.

```rv
x-ddr2brdccbr4d3-y;            // Connect x and y with a sequence (chunks may be concatenated)
x-d-r2-t-y;                    // Or separated by '-'
x-y;                           // A zero-length chain (direct connection)
```

The first and last elements of a chain must be declared regs or ports.
Intermediate chunks must be components or wire names.
If multiple chains merge at one reg, the merge takes the maximum strength (matching the game).

Attaching `.side` to an endpoint wires it to the side input of a comparator or repeater reg (§5.3).
`.side` cannot be attached to intermediate chunks or to the source end.

### 5.2 `wire` (a reusable component sequence)

A `wire` is a template for a component sequence with no endpoints.
It can be expanded as an intermediate chunk of a chain.
Using the same wire in several places expands into an *independent* sequence at each site (they are physically separate).

```rv
wire seg;
seg = d2ccd2;                  // Define: dust 2 + comparator + dust 2
x - seg - m;                   // Wire x to m via seg
y - seg - m;                   // Reuse seg for y → m (a separate instance)
```

A wire assignment (`w = ...`) is a *definition* of a component sequence, not a connection.
For that reason, endpoints (regs or port names) and `.side` cannot appear on the right-hand side.

Examples of usable sequences:

- A single component (`w = r4;`)
- Multiple components (`w = d4-cc-d4;` or `w = d4ccd4;`)
- A composition with other wires (`wrap = base-r-base;`)

Cyclic references are errors (e.g. `w1 = w2;` together with `w2 = w1;`).
Using an undefined wire (declared but not assigned) in a chain is also an error.
Reassigning a wire produces a warning, and the last assignment is taken.

A wire itself cannot be an endpoint of a chain (endpoints must be regs or ports).

### 5.3 Named Points

Components with a side input (comparators and lockable repeaters) get 3-endpoint wiring by being assigned to a `reg` to form a *named point*.
The three connection points — *back*, *side*, and *out* — are each driven by a separate chain.

#### 5.3.1 Comparator Side Input

```rv
reg cmp = cd;                  // A subtract comparator (cc = compare)
x - d3 - cmp;                  // cmp at the chain's end → back input
s - cmp.side;                  // cmp.side → side input
cmp - y;                       // cmp at the chain's start → out output
```

| Endpoint | Notation | Role |
|---|---|---|
| back | `cmp` (end of chain) | Back input |
| side | `cmp.side` (end of chain only) | Side input |
| out | `cmp` (start of chain) | Forward output |

The output appears with a 1-tick delay, computed by the mode:

- `cd` (subtract): `out = max(0, back − side)`
- `cc` (compare): `out = back >= side ? back : 0`

If multiple wires merge into back or side, the merge takes the maximum strength (matching the game's left/right side max).

#### 5.3.2 Repeater Lock

If a repeater is assigned to a `reg` to form a named point, it gains the same three endpoints as a comparator.
While the side input is `> 0`, the output is frozen at its value just before the lock; the back input is no longer followed.
When the side input returns to 0, the repeater resumes normal operation (the lock responds within 1 tick).

```rv
reg m = r;                     // A lockable repeater (r / r1-r4; same delay rules)
x - m;                         // m at the chain's end → back input
lk - m.side;                   // m.side → lock input
m - y;                         // m at the chain's start → forward output
```

Notation and merge rules for the three endpoints match the comparator case.

Inline repeaters in a chain (e.g. `x-r-y`) cannot be locked, because no side input can be drawn out.

#### 5.3.3 Constraints Common to Side Inputs

Side inputs of comparator and repeater regs share the following constraints.

- Using `.side` at the chain's start (as a signal source) is an error (side is input-only)
- Attaching `.side` to a reg that is neither a comparator nor a repeater is an error
- Only the plain qualifier is allowed; strength literals, `const`, and `mutable` are not (§3.4)
- Initialization at declaration is required (`reg cmp; cmp = cd;` after-the-fact assignment is not allowed, §3.4)

### 5.4 Hierarchical Instantiation

A `logic` body can call another `logic` to nest circuits.

```rv
logic AND2(input x1, input x2, output y) {
    reg na, nb, s;
    na = NOT(x1);         // out = callee(args...)
    nb = NOT(x2);
    s  = OR2(na, nb);
    y  = NOT(s);          // x1 & x2 = !( !x1 | !x2 )
}
```

The form is `out = callee(args...);`.
`out` refers to a reg or output port of the current logic; `args` refers to regs or input port names (wires, components, and numeric literals cannot be passed).

The connection rules are:

- The callee's output port is directly wired (no attenuation) to `out`
- Each node of `args` drives the callee's input port
- The child instance is independent of `sim` variables; it is purely a circuit expansion

With one output port, use this form (`out = callee(...)`).
For a logic with multiple output ports, use the tuple binding in §5.5.
The single port may itself be a scalar or a bus (`output[N]`); a bus port is bound lane-by-lane as a homogeneous multi-output (§6).

The number of arguments must match the callee's input port count.
Each argument is a reg, a port, or a whole bus.

Recursive instantiation (self or mutual cycles) is an error.

### 5.5 Multi-Output Logic and Tuple Binding

A `logic` can declare **multiple output ports**.
When the same inputs produce two distinct results — like `sum` and `carry` of a half adder — list them as separate ports.

```rv
logic HALF_ADDER(input x1, input x2, output sum, output carry) {
    sum   = XOR2(x1, x2);
    carry = AND2(x1, x2);
}
```

To instantiate a multi-output logic, receive the outputs as a tuple of bind targets.
Each target is wired (no attenuation) to the corresponding output port in declaration order.

```rv
module m() {
    var x1, x2, sum, carry;
    sim {
        x1 = 0; x2 = 0;
        (sum, carry) = HALF_ADDER(x1, x2);    // tuple binding
        #init
    }
}
```

Tuple binding works both inside a logic body and inside a sim block.
Rules and constraints:

- Each target is a plain reg / port / bus reg / bus port / var / bus var. Lane indices (`a[0]`), slices, concatenations, and `.side` are not allowed
- The number of targets must match the callee's output port count exactly (both shortage and excess are errors)
- The same target cannot appear twice (`(p, p) = g(x);` is an error)
- Each target and its corresponding output port must agree in shape (scalar vs. bus) and width
- A 1-output logic may also be received as a **1-target tuple** `(t) = callee(args...)` — equivalent to the conventional `t = callee(args...)`

The argument list, recursion ban, and binding semantics are identical to §5.4.

### 5.6 Nested Calls in Arguments

An argument of a logic call may itself be **another logic call** instead of a name.

```rv
logic MUX2(input in0, input in1, input sel, output y) {
    y = s_or(s_and(in0, s_not(sel)), s_and(in1, sel));
}
```

The nested call's output port is wired (no attenuation) straight into the outer call's input port.
This is the same circuit as declaring intermediate regs / vars by hand; the example above expands to:

```rv
reg ns, t0, t1;
ns = s_not(sel);
t0 = s_and(in0, ns);
t1 = s_and(in1, sel);
y  = s_or(t0, t1);
```

Rules:

- Works both inside a logic body (§5.4) and inside sim (§7.3)
- Only a logic with **exactly one output port** can be nested (receive a multi-output logic with a tuple binding first, then pass the target)
- Nesting depth is unlimited. The recursion ban (§5.4) is enforced through nested calls as well
- Generic arguments `#(...)` (§8.4) are allowed on nested calls (`g(h#(W=8)(x))`)
- The output port and the input port must agree in shape (scalar vs. bus) and width
- `scan()` (§7.8) is not a logic and cannot be nested

The intermediate point has no name, so it cannot be observed with the trace (`-t`) or `monitor`.
When intermediate values matter, declare intermediate regs / vars as before.

---

## 6. Buses

A bus is sugar for wiring several lanes with a single line.
Buses are expanded into N scalar chains at elaboration time, so the simulation engine itself knows nothing about buses.
Consequently the semantics for merging (max), sequential elements, and `#init` are identical to scalar points.

### 6.1 Bus Declaration and Indexing

`reg[N] name;` declares N parallel lanes.
Each lane, `name[0]` through `name[N-1]`, can be used as a regular scalar point.

```rv
reg[4] in;                     // Bus declaration: in[0]..in[3]
reg[4] buf;
x - in[0];                     // Address one lane by index [k]
in - r - buf;                  // Whole bus: one line expands to 4 lanes (each gets its own repeater)
buf[0] - y;                    // Pick a lane and wire it to the output port
```

| Notation | Meaning |
|---|---|
| `reg[N] a;` | A bus declaration of width N. `[N]` sits between `reg` and the name. `N` is a literal or a constant expression (§8.3) |
| `a[k]` (k is a constant expression, §6.3.1) | Lane k. Usable like a scalar point (0 ≤ k < N) |
| `a` (chain endpoint, no index) | The whole bus. Element-wise wiring with a same-width counterpart |

Bus names (the base name) also obey the name constraints in §2.2.
A bus may only appear at the endpoint of a chain, not as an intermediate chunk.

When several declarations share a line (`reg[N] a, b;`), `[N]` applies to the entire declaration list.

### 6.2 Bus Chains and Bus Ports

A bus chain is sugar for element-wise wiring between two same-width buses.
When the widths match, the intermediate component sequence is expanded independently per lane.
Intermediate wire names are also expanded into a separate instance per lane.

Ports too can be buses.
`input[N] x` and `output[N] y` are N-lane parallel ports; inside the body you can use them with indexing `x[k]` or as a whole bus `x`, just like internal bus regs.

```rv
logic AND2(input[4] x1, input[4] x2, output[4] y) {
    reg[4] nx1;  reg[4] nx2;  reg[4] s;
    x1 - t - nx1;              // nx1 = NOT x1 (4 lanes at once)
    x2 - t - nx2;
    nx1 - s;  nx2 - s;         // s = nx1 OR nx2 (max merge per lane)
    s - t - y;                 // y = NOT s = x1 AND x2 (bitwise AND)
}
```

A bus output port is a homogeneous multi-output (N lanes).
In `out = g(args...)`, if `out` is a bus var, the binding is lane-correspondent (§7.3).
Arguments and output destinations must match the port in shape (scalar vs bus) and width; otherwise it is an error.

### 6.3 Slice `x[hi:lo]` and Concatenation `{x, y}`

The endpoints of a chain accept lane ranges (slices) and concatenations of several endpoints.
Both stay within the bus-sugar layer; they simply expand into a lane list.
If both endpoint widths (lane counts) match, the wiring is element-wise.

```rv
x[3:0] - r - y[3:0];           // Wire sub-buses with a component sequence
x[3:0] - y[0:3];               // Reversing the order reverses the bit order
hi - {carry, sum};             // Concatenation as bulk wiring (width is the sum of parts)
{x[2:0], x[3]} - out[3:0];     // Concatenation + slice for a 1-bit left rotate
```

| Notation | Meaning |
|---|---|
| `x[hi:lo]` | Slice (inclusive). When `hi >= lo`, descending `[hi, hi-1, .., lo]`; when `hi < lo`, ascending `[hi, .., lo]`. `x[k:k]` is equivalent to `x[k]` (width 1). `hi` / `lo` are in-range constant expressions (§6.3.1) |
| `{e1, e2, ...}` | Concatenation. Each `ei` is a scalar point, a lane `x[k]`, a slice `x[hi:lo]`, or a whole bus. Lanes are joined from left to right; the width is the sum of each part |

The whole-bus name supplies lanes in ascending index order `[0..N)` (same as a regular whole-bus chain).
Use a slice `x[hi:lo]` to control the bit order explicitly.

Slices and concatenations are endpoint-only; they cannot appear in intermediate chunks or on the right-hand side of `=`.
Concatenation elements may not carry `.side` or nest as `{{..}}`.
Out-of-range indices and slices on non-bus names are errors.

### 6.3.1 Constant Expressions in Indices

Lane indices `a[k]` and slice indices `x[hi:lo]` accept **constant expressions** in addition to literals.
The allowed elements are the same as bus widths `[expr]` (§8.3): literals, `param`, numeric `#define`, `+ - * / %`, unary `-`, and parentheses.

```rv
param N = 2;

logic SPLIT #(W=8)(input[W] x, output[W/2] hi_o, output[W/2] lo_o) {
    x[W-1:W/2] - hi_o[W/2-1:0];    // Upper half
    x[W/2-1:0] - lo_o[W/2-1:0];    // Lower half
}

logic TAP(input[4] a, output y) {
    a[N+1] - y;                    // param references resolve to a[3] at parse time
}
```

Expressions that reference a generic param (§8.4) are evaluated at instantiation time, just like width expressions; all other expressions resolve at parse time.
The descending / ascending and width-1 rules (§6.3) apply to the evaluated values as-is.
An out-of-range result is an error (for expressions with generic params, it is detected only once the logic is instantiated).

### 6.4 Bus-Scalar Wiring (Broadcast)

When one side of a bus chain is a scalar (width 1), broadcasting fills the other side's N lanes.
This too only expands into a lane list at the sugar layer.

```rv
reg[4] x;  reg y;
x - y;                         // Fan-in: all 4 lanes of x feed y directly
y - x;                         // Fan-out: y drives all 4 lanes of x
x - r - y;                     // A form with a component sequence is also allowed
```

- **Fan-in** (`bus[N] - scalar`): N lanes merge at one point. The merge is MAX, so `scalar = max(all lanes)`
- **Fan-out** (`scalar - bus[N]`): one point drives all N lanes (each lane shares the same source)

Connections are directed; the left of `-` is the source and the right is the destination.
This makes "collect several signals into one" and "duplicate one source to many points" expressible as directed-model sugar.
(Real dust is bidirectional, but redv connections do not flow backwards; §11.2.)

The scalar side may be a scalar point, a single lane `a[k]`, or a width-1 slice or concatenation.
The other side may be a whole bus, a slice, or a concatenation (broadcasting kicks in whenever one side has width 1).
If both sides have width > 1 and they disagree, the connection is an error (`bus[4] - bus[2]`, etc.).
To wire one specific lane, index it as `name[k]` to make it a scalar.

### 6.5 Currently Unsupported Items

The following are kept for future extension and are errors or unsupported at present.

- Non-plain bus declarations (`const` / `mutable`, initializers, comparator or repeater bus regs)
- Buses on the left or right side of assignment (`a = ...`)
- Buses in wire sequences, the `wire[N]` syntax
- Tuple binding of heterogeneous multi-outputs
- Passing a single bus lane directly as a logic argument (`g(x[0])`; pass the whole bus, or copy into a scalar var first)

---

## 7. The `sim` Block

A `module`'s `sim { ... }` is a sequential interpreter language for writing testbenches that drive a circuit and observe it.
The core consists of three kinds of statements — assignment, time advance, and observation — augmented with `if` / `while` / `for` control flow.

### 7.1 Structure

```rv
module test() {
    var x, y;
    sim {
        x = 0;
        y = NOT(x);       // Instantiation and I/O binding
        #init                  // Wait until steady state
        x = 10;
        #1                     // Advance 1 tick
        ?monitor("time=% | x=% | y=%\n", $time, x, y);
    }
}
```

A typical `sim` starts with variable initialization and binding, then advances time and observes.
`?monitor` is registered at sim start and fires automatically at every tick (§7.4).

### 7.2 Advancing Time

| Statement | Behavior |
|---|---|
| `#n` | Advances n ticks. `$time` advances and `?monitor` fires |
| `#init` | Waits until the update queue is empty (steady state). Exceeds `INIT_TIMEOUT` (§8.1) is an error |
| `#until(cond)` | An event-driven wait that advances ticks until `cond` is true (`!= 0`) |
| `wait(n)` | Waits n ticks without advancing `$time` and without firing `?monitor` (for oscillating circuits) |

`#until` is a handy wait that needs no knowledge of how many ticks it takes for an output to rise.
`$time` advances like `#n`, and at completion `$time` reflects the ticks consumed.
If `cond` is already true on entry, it exits in 0 ticks (same `while`-style evaluation on entry).
Exceeding `INIT_TIMEOUT` is an error, which catches both oscillation and a condition that never holds.

`$time` is ticks elapsed since `#init` completion (or sim start when `#init` is unused), with the reference at 0.

### 7.3 I/O Binding

`v = logic-name(variables...)` instantiates a circuit and binds variables to its I/O.

```rv
y = NOT(x);               // x becomes the input, y the output
```

The binding rules are:

- The same `(logic-name, argument list)` pair shares a single instance (the target list is not part of the cache key)
- Input variables stay bound; output variables are updated every tick
- Bus ports (§6.2) take a whole bus var of matching shape and width

Example with bus vars:

```rv
var[4] x, y;
y = AND2(x, x);                // Bind a whole bus var to bus ports
```

Scalar ports require scalar vars, bus ports require bus vars; shape or width mismatches are errors.

A multi-output logic is received with a `(t1, t2, ...)` tuple binding (§5.5).

```rv
var x1, x2, sum, carry;
(sum, carry) = HALF_ADDER(x1, x2);   // bind the two outputs
```

The number of targets must equal the number of output ports (both shortage and excess are errors), and the same target cannot appear twice (`(p, p) = ...`).
A 1-output logic may also use the 1-target tuple form `(v) = callee(...)`, equivalent to the conventional `v = callee(...)`.

Arguments may nest other logic calls directly (§5.6).

```rv
y = s_or(s_and(x1, x2), s_xor(x3, x4));   // y = (x1 & x2) | (x3 ^ x4)
```

Instance sharing extends to nested subexpressions:
`s_and(x1, x2)` above shares its instance with a standalone call using the same argument list (`t = s_and(x1, x2);`).

Variables that are bound to the circuit are clamped to 0–15 on input application.
Out-of-range values raise a single warning per variable (§10).

### 7.4 Observation

There are three observation forms: `monitor` for golden tests, and `assert` / `expect` for self-checks.

#### 7.4.1 `?monitor` and `monitor`

`?monitor(fmt, ...)` is the auto-firing form.
It is registered at sim start and runs automatically right after every wait (`#init` / `#n` / `#until`) completes.
You may place it anywhere in the sim — output appears at every observed time (Verilog-style `$monitor`).

`monitor(fmt, ...)` is a one-shot statement that prints at the position it appears.
The `?` prefix distinguishes the auto-firing form from the one-shot form.

Format string rules:

- `%` consumes the next argument in decimal
- `%N` (e.g. `%2`) specifies a minimum width N (right-aligned, space-padded)
- `%%` prints a literal `%`
- Adjacent placeholders must use the width form (e.g. `%1%1%1%1`) because `%%` would collide with the escape

Type suffixes `%d` / `%t` are removed (specifying them is an error).

Base suffixes `%b` (binary) / `%x` (hex, lowercase) / `%o` (octal) are available, with optional width and zero-pad (`%4b`, `%04b`).
Negative values print as `-` plus the absolute value in the given base.
See `docs/LANGUAGE.md` §7.4.1 for the full table.

##### Passing a Bus var as a single argument

Passing a bus var (no index, §7.6) directly to monitor packs each lane's strength (0–15) as a 4-bit nibble, with `lane[0]` at the lowest nibble and `lane[N-1]` at the highest.
The composed integer is then formatted with the usual `%` / `%b` / `%x` / `%o`.

```rv
var[4] bus;
bus[0] = 15;  bus[1] = 0;  bus[2] = 15;  bus[3] = 8;
// packed = 8*16^3 + 15*16^2 + 0*16^1 + 15*16^0 = 0x8F0F = 36623
monitor("%x % %b\n", bus, bus, bus);
// → "8f0f 36623 1000111100001111"
```

`%x` / `%b` zero-pad to N hex digits / 4N bits by default so lane boundaries stay aligned.
User-specified width acts as a lower bound on top of that.
The composition only fires when the bus var appears at the top of a format argument; using it inside an expression (e.g. `bus + 1`) still errs with "index a lane".
Buses wider than 16 lanes cannot be packed into a single i64 and are an error — monitor lanes individually instead.

#### 7.4.2 `assert` and `expect`

`assert(cond)` records a failure on stderr when `cond` is false (`= 0`).
Failures report the line number and the source expression.

`expect(actual, expected)` records a failure when the two disagree, printing the actual and expected values.

Neither stops the sim on failure; both keep running.
After all modules' sims finish, the engine summarizes: on a pass, it prints `N assertion(s), all passed`; on failures, it prints `assertions: M of N failed` and exits with a non-zero code.

This makes it possible to write self-checking testbenches whose pass/fail comes from the exit code rather than `monitor` (stdout) output (`examples/assert_selfcheck.rv`).

### 7.5 Control Flow and Expressions

C-style control flow is available inside a sim block.

```rv
for (i = 0; i < 4; i = i + 1) {
    x = i;
    #1
}
```

`if` / `while` / `for` follow C grammar, and their conditions are ordinary integer expressions.
Comparison and logical operators are also available.

A `var` is an integer (like C's `int`) and cannot be embedded in a circuit.
Sim expressions are plain integer arithmetic, independent of the 0–15 strength scale used in the circuit.
However, variables bound to the circuit are clamped to 0–15 on input application (§7.3).

### 7.6 Bus `var`

`var[N] x;` declares a bus var with N lanes.

```rv
var[4] x;
x[0] = 15;                     // Per-lane assignment (the index is a runtime integer expression)
x = 0;                         // Broadcast to all lanes (one-shot clear)
```

Bus var usage rules:

- Per-lane read/write uses the form `x[k]` (the index is a runtime expression; `for`-loop variables are fine)
- Using the bare name `x` in a scalar expression is an error (a lane must be specified)
- `x = expr;` broadcasts to all lanes (`x = 0;` clears them in one shot)

### 7.7 Pulse Assignment and Clock Generation

#### 7.7.1 Pulse Assignment

`v = expr ~ w;` is a pulse assignment.
After putting the value into `v`, the engine automatically clears it to 0 once `w` ticks have elapsed (you don't have to write the assignment, wait, and 0-assignment by hand).

```rv
x = 10 ~ 3;                    // Hold x = 10 for 3 ticks, then x = 0
```

`w` is an integer ≥ 1 (an expression is fine; it is evaluated at assignment time).
The tick count covers any ticks the sim executes (`#n` / `wait(n)` / `#init`).

A regular assignment to the same var before the deadline cancels the pending pulse.
A new pulse assignment replaces the previous deadline with the new width.

#### 7.7.2 Clock Generation

`clock(var, N)` is sugar for generating a test clock on one line.
It auto-toggles a scalar var between 0 and 15, holding each level for N ticks (full period = 2N, 50% duty).

- Right after the call, the var is Low (0)
- It flips every N ticks afterward
- `clock()` itself does not advance time; the toggling happens while subsequent `#n` / `wait` / `#init` / `#until` accumulate ticks

A regular assignment to the same var clears the clock.
`N` is an integer ≥ 1 (expressions allowed).
`var` must be a declared scalar var; bus vars are not supported.

The value of the var observed in `monitor` reflects "the value applied next tick", so it appears 1 tick ahead (the same behavior as pulse assignment).
Duty ratio and initial phase are future extensions (`examples/clock_sugar.rv`).

### 7.8 Input Reading (`scan`)

`v = scan()` reads one integer (separated by whitespace or newline) from stdin and assigns it to `v`.
EOF (input exhaustion) or a non-numeric token is an error.
Variables bound to the circuit are clamped to 0–15 after reading.

In tests, fixing the input with `redv foo.rv < input.txt` gives a deterministic, reproducible run.

---

## 8. Directives

### 8.1 `#define`

```rv
#define INIT_TIMEOUT 200       // Timeout for #init (default 1000 ticks)
#define BURNOUT_LIMIT 8        // Torch burnout: max toggles within the window (default 8)
#define BURNOUT_WINDOW 30      // ...the window (default 30 ticks)
#define BURNOUT_COOLDOWN 30    // ...the forced-OFF period (default 30 ticks)
#define MODE element           // Component-level mode (logic-level mode is future work)
```

Numeric `#define`s share a single table with `param` (§8.3) and can be referenced by name in bus widths and sim expressions.

The value accepts the same constant expression as `param`: literals, earlier `#define` / `param`, `+ - * / %`, unary `-` / `!`, and parentheses.

```rv
#define W 4
#define N (W * 2)              // Arithmetic: 8
#define HALF (N / 2)           // Nested: 4
#define ONES (0b1111)          // Mixed with binary / hex literals (§1.3)
```

Only `MODE` is reserved for future mode switching and still takes an identifier value (anything other than `element` warns).
Referencing an undefined name or dividing by zero is an error.

### 8.2 `#include`

```rv
#include "other.rv"            // Pull in a file
```

#### 8.2.1 Bundled standard library

Passing a **bundled name** like `#include "stdlogic"` pulls in a standard library that is embedded inside the `redv` binary.
You do not need to ship the file separately or write a relative path; the same source is read no matter where `redv` runs from.

| Name | Contents |
|---|---|
| `stdlogic` | Basic logic gates (`s_not` / `s_and` / `s_or` / `s_xor` / `s_nand` / `s_nor` / `s_xnor`). All are scalar 1–2 input / 1 output |

Repeating `#include` of the same bundled name within one source is a no-op after the first occurrence (no duplicate-definition error).
The check applies even across nested includes.
File-based includes do not have this dedup; the caller must avoid circular or double inclusion.

A name not in the bundled list falls back to the normal file include path.
For example `#include "stdfoo"` is not a bundled name, so `stdfoo` / `stdfoo.rv` is searched as a relative / absolute path, and is an error if not found.

```rv
#include "stdlogic"

logic EQUAL(input x1, input x2, output y) {
    y = s_xnor(x1, x2);        // 1-bit equality detector = XNOR
}
```

`s_xor` / `s_xnor` are layered on top of `s_not` / `s_and` / `s_or`, so they take 4–5 ticks total.
Per-gate propagation delays are listed in the header comment of `examples/stdlogic_demo.rv`.

### 8.3 `param` (parameter constants)

`param NAME = <const-expr>;` declares an integer constant at the top level.
It can be referenced by name in bus widths and sim expressions, so widths are not hard-coded as literals; one definition serves multiple widths.

```rv
param W  = 4;                  // A constant
param W2 = W + 1;              // Earlier params may be used in a const-expr (no forward references)

logic NOT(input[W] x, output[W] y) { x - t - y; }   // Parameterize width by param

module m() {
    var[W] x;                  // Bus width by param
    var[W2] y;                 // Width from a const-expr (= 5)
    var i;
    sim {
        x = 0;
        for (i = 0; i < W; i = i + 1) { x[i] = 15; }   // Also usable in sim expressions
    }
}
```

Resolution rules:

- Values resolve at parse time. `param` and numeric `#define` share a single table that both `[ ... ]` widths and sim expressions read
- Width `[expr]` accepts a constant expression (literal, `param`, `+ - * / %`, unary `-`, parentheses)
- The result must be ≥ 1 (0 or below is an error)
- Disallowed in a const-expr: `$time`, bus indices, comparison and logical operators, undefined names
- In sim expressions, a same-named `var` shadows the `param`. Only when no `var` matches does it fall through to `param`

### 8.4 Per-Logic Generic Widths

The form `logic name #(P=default, ...)(...)` declares **generic width parameters** on a logic.
Each call can pick its own values, so a single definition can be instantiated at multiple widths.

```rv
logic NOT #(W=4)(input[W] x, output[W] y)
{
    reg[W] s;
    x - s;
    s - t - y;
}

module m()
{
    var[4] x4, y4;
    var[8] x8, y8;
    sim {
        x4 = 0;  x8 = 0;
        y4 = NOT(x4);            // Expanded with the default W=4
        y8 = NOT#(W=8)(x8);      // Expanded as a separate instance with W=8
        #init
    }
}
```

Declaration and call rules:

- Declaration: write `#(P=expr, Q, ...)` between the logic name and the port list. Each param may carry an optional default after `=`
- Defaults are evaluated as ordinary `param`-style const expressions at parse time. They cannot reference other generic params
- Call site: write `callee#(P=expr, ...)(args)`. Omit `#(...)` to fall back to defaults for every param
- Actual arguments are evaluated in the caller scope and may reference top-level `param` / numeric `#define`, **plus** the caller logic's own generic params (so values pass through hierarchies)
- It is an error to instantiate a param that has neither a default nor an actual argument

Inside the logic body, declared params can be used in width expressions such as `input[W]` or `reg[W+1]`, and in lane / slice indices such as `x[W-1]` or `x[W-1:W/2]` (§6.3.1).
Expressions that reference a generic param are deferred to instantiation time and resolved per instance.
Width expressions that do not reference any generic param (`input[4]`, `reg[5]`) are still resolved at parse time and behave exactly as before.

Instance identity is the full call key: `callee` + `#(...)` + the argument list.
`g#(W=4)(x)` and `g#(W=8)(x)` are separate instances with their own node groups, while calling `g#(W=4)(x)` twice reuses the same instance.

Module-side `var[N]` is **not** generic; it is still resolved at parse time using `param` / `#define`.

---

## 9. Simulation Semantics

This section defines, tick by tick, what the simulation engine does.
For the implementation structure, see [ARCHITECTURE.md §7](ARCHITECTURE.md) (Japanese).

### 9.1 Tick Granularity

`#1` is one redstone tick.
Game-tick components and sub-redstone-tick pulses are out of scope.

### 9.2 Evaluation Order within a Tick

Each tick proceeds in three phases.

1. Sequential element output (compute output for repeater, torch, comparator, and observer from the front of `hist`)
2. Combinational network fixed-point resolution (converge dust and block values via MAX merge)
3. Sequential element input sampling (advance `hist` for the next tick)

The merge is monotone max, so the fixed point converges deterministically and is independent of traversal order.
This implementation guarantees the result of a sequential queue scheme, order-independently.

### 9.3 Signal Propagation and Attenuation

Attenuation is computed dynamically every tick.
Dust subtracts 1 per piece; merge points take the maximum.
A block is two-valued: 15 when input is `> 0`, 0 when `= 0`.

### 9.4 Component Update Timing

The output rules for each component:

- Dust, block: immediate (output is decided within the same tick)
- 0-tick repeater `r0`: `out(T) = in(T) > 0 ? 15 : 0` (same tick, combinational; §4.3)
- Repeater `rn`: `out(T) = in(T−n) > 0 ? 15 : 0` (also holds 1-tick pulses)
- Lockable repeater: while side input is `> 0`, `out(T) = out(T−1)` (frozen)
- Torch: `out(T) = in(T−1) > 0 ? 0 : 15`
- Comparator: `out(T) = f(back(T−1), side(T−1))`
- Comparator (side unconnected): `out(T) = back(T−1)` (pass-through)
- Observer: `out(T) = in(T−2) != in(T−1) ? 15 : 0` (neighbor-sample change detection)
- Observer (edge variants): `op` / `on` / `oe` replace only the rule with rising / falling / binary edge detection (§4.7.1)

### 9.5 Reflecting Input Changes

`a = 10;` takes effect at the start of the *next* tick (Verilog non-blocking equivalent).
Only variables bound by `logic(args)` affect the circuit.

A pulse assignment `a = v ~ w;` puts `v` into `a` and schedules `a = 0` after `w` ticks.
The decrement happens at each tick's end (after output reflection), so `v` is applied as input for exactly `w` ticks counted from input reflection, and then `a` returns to 0.
`w < 1` is an error.

A regular assignment to `a` before the deadline cancels the schedule.
A new pulse assignment replaces it with the new width.

### 9.6 Steady-state Detection for `#init`

`#init` terminates when all node values are unchanged from the previous tick and all sequential-element pipelines are uniform (`hist` agrees in every stage).
Exceeding `INIT_TIMEOUT` is an error (oscillation detection).
Use `wait(n)` to step past an oscillating circuit.

`#until(cond)` reuses the same `INIT_TIMEOUT`.
It advances ticks until `cond` is true; exceeding the limit is an error.

### 9.7 Observation Timing

`monitor` and output variables observe the value *after* the tick's processing completes.

### 9.8 Merge Rule

Multiple chains merging at the same reg are combined by maximum (matching the game).

---

## 10. Errors and Warnings

The diagnostics are intentionally strict.
Unrecoverable conditions raise errors and halt; recoverable conditions print warnings to stderr without halting execution.

### 10.1 Warnings (execution continues)

| Condition | Message intent |
|---|---|
| An out-of-range var was passed as a circuit input | Rounded to 0 or 15; warned once per variable |
| Reassignment of a wire or reg element | Last assignment is taken; a warning is emitted |
| Torch burnout | Toggle count in the window exceeded; forced OFF for the cooldown |

### 10.2 Errors (execution halts)

| Condition | Intent |
|---|---|
| Out-of-range strength on `const reg`, or reassignment | Breaks the immutability promise |
| Unconnected output port | Caught at instantiation |
| Duplicate names, undeclared variables | Detected statically |
| Argument count mismatch | Caught at logic instantiation |
| `c` alone, `r5` or higher | Validity of component notation |
| Name collides with a component name | `b` / `r` / `cd` / `tb`, etc. (§2.2) |
| After-the-fact assignment to a repeater or comparator reg | `reg m; m = r;` (must initialize at declaration; §3.4) |
| `.side` on an unsupported reg or at a source end | Side is input-only; only cmp / r* support it |
| `#init` exceeds `INIT_TIMEOUT` | Oscillation, or a never-satisfied condition |
| `#until(cond)` exceeds the timeout | Same |
| EOF or non-numeric input from `scan` | Input format validity |

### 10.3 Diagnostic Display (caret)

Errors and warnings print to stderr in Rust-style caret format.
The form is `--> file:line:col`, followed by the source line and a `^` underline.

- Syntax errors (lexer and parser): point at the exact column
- Elaboration errors: line-level (underline the line's content)

See [ARCHITECTURE.md §3.6](ARCHITECTURE.md) (Japanese) for implementation details.

### 10.4 Design Rule Check (lint)

A **lint pass** runs after elaboration and warns about structures that run but look suspicious (issue #48).
To keep it separate from runtime warnings (§10.1), lint uses its own `[lint]` category with a rule name prefix on stderr.
Lint warnings do not stop execution.

```
[lint] floating-reg: reg 'orphan' in logic 'sloppy' is not connected to anything
  --> examples/lint_demo.rv:18:5
```

| Rule | Condition |
|---|---|
| `floating-reg` | A reg is driven by nothing and drives nothing (fully isolated) |
| `unused-wire` | A wire is declared but never used in any chain |
| `unused-input` | An input port is never used in the logic body |
| `always-on-torch` | A torch's back input stays 0 no matter what (output is always ON) |
| `unreachable-output` | An output port stays 0 no matter what (e.g. killed by 15 dusts of decay) |

The first three rules are static: they only look at declarations and connectivity.
The last two use an **upper-bound analysis**: it computes an upper bound of the strength each point can ever reach, and reports only points whose bound is 0.
The bound may overestimate but never underestimates, so a reported point can never light up during execution (no false positives).
See [ARCHITECTURE.md](ARCHITECTURE.md) (Japanese) for how the analysis works.

Instantiating the same logic multiple times emits each declaration-based static warning only once per logic name.
The upper-bound rules fire per top-level instantiation, scoped to that instance's components and ports.

With the CLI flag `-W error`, the run completes as usual, and the exit code becomes 1 if any warning (runtime warnings of §10.1 and lint alike) was emitted.
This is intended for enforcing zero warnings in CI.
In `--json` mode (§7.4.1), lint becomes JSONL of the form `{"kind":"lint","rule":"...","line":N,"msg":"..."}`.

`examples/lint_demo.rv` fires all 5 rules.

---

## 11. Differences from the Game

redv intentionally deviates from the original game's redstone behavior in several places.
The reasons are some combination of: simplification, ensuring determinism, and making text descriptions easier.
This is not a system intended to mimic the game exactly.

### 11.1 No Dust Shape

In the game, the shape of dust ("point", "cross", etc.) depends on placement and affects how the signal spreads.
In redv, where the signal goes is determined by reg-to-reg wiring (chain branching), so there is no shape distinction.
Dust is just `d`; the older `dx` ("cross") notation has been removed (issue #66).

### 11.2 Connections are Directed

Real dust is bidirectional, but redv connections are directed: the signal flows from source to destination only.
This is a natural consequence of the textual model where `-` has a left (source) and a right (destination).
Fan-in and fan-out (§6.4) are sugar that fits "merge several points into one (MAX)" and "broadcast one source to many points" onto this directed model.

### 11.3 Looser Lock Condition

The game's repeater lock only activates when the side input comes from another repeater or comparator.
redv locks whenever the side input is `> 0`, regardless of the source's component type (e.g. dust counts).
This favors textual-description simplicity over the source-component check.

### 11.4 The 0-tick Repeater

The game has no true 0-tick repeater (a delay of 1 is the minimum).
redv provides `r0` as an extension for "building higher-performance circuits in text" (issue #37).
0-tick variants of torch or comparator are non-monotone (inversion or subtraction), so they would require a combinational-loop detector and an extension to the engine; they are tracked separately.

### 11.5 Observer Watches Input Signal Changes

The game's observer fires on block-state updates and has spatial properties (orientation, QC quasi-wiring, output face).
redv works at the component level and watches only input-signal changes (no orientation, no QC).

Also, the game's observer comes in a single kind that detects every change; it has no edge-condition variants.
`op` / `on` / `oe` (§4.7.1) are redv extensions for writing pulse-shaping circuits concisely in text (issue #58).

---

## 12. Sample List

The `examples/*.rv` directory holds sample circuits covering each feature.
All of them run with `cargo run -- examples/foo.rv` and are exercised by the golden tests.

### 12.1 Basic Gates

| File | Contents |
|---|---|
| `examples/not_gate.rv` | A NOT from a single torch |
| `examples/or_gate.rv` | An OR from 2 repeaters + dust merge |
| `examples/and_gate.rv` | An AND from 3 torches (NOR of NOTs) |
| `examples/hier_and.rv` | A hierarchical AND nesting `NOT` and `OR2` (De Morgan) |
| `examples/half_adder.rv` | Multi-output logic with tuple binding `(sum, carry) = HALF_ADDER(x1, x2);` (§5.5) |
| `examples/nested_call.rv` | Nested calls `y = s_or(s_and(x1,x2), s_xor(x3,x4));` and a one-line MUX (§5.6) |
| `examples/chain_mixed.rv` | Merging two chain paths into the same point (max) |

### 12.2 Component Behaviors

| File | Contents |
|---|---|
| `examples/decay.rv` | Comparison of dust attenuation, repeater re-amplification, and comparator strength pass-through |
| `examples/comparator_side.rv` | Comparator side input (`cd` subtract, `cc` compare) |
| `examples/repeater_lock.rv` | Repeater lock (`.side` on `reg m = r;` freezes the output) |
| `examples/repeater_0tick.rv` | 0-tick repeater (`r0`) vs. normal repeater (`r1`): timing comparison |
| `examples/observer.rv` | Observer (`o`): detects input changes and emits a 1-tick pulse |
| `examples/observer_edge.rv` | Observer edge variants (`op` / `on` / `oe`) compared with the base `o` across all 4 modes |
| `examples/wire_reuse.rv` | Define a wire as a reusable component sequence and use it in several places |

### 12.3 sim and Verification

| File | Contents |
|---|---|
| `examples/counter_test.rv` | Verifies the AND truth table with `for` and `if` |
| `examples/assert_selfcheck.rv` | Pass/fail via exit code using `assert` and `expect` |
| `examples/clock.rv` | A torch + repeater-4 clock (period 10). Example of `wait()` |
| `examples/clock_sugar.rv` | Sugar for test clocks: `clock(var, N)` |
| `examples/scan_and.rv` | Reads two values from stdin with `scan()` and feeds them into an AND |
| `examples/until_wait.rv` | `#until(cond)`: event-driven wait that advances ticks until the condition holds |
| `examples/pulse.rv` | Pulse assignment (`a = v ~ w;`) auto-resets a var to 0 after `w` ticks |
| `examples/lint_demo.rv` | Fires all 5 design-rule-check (lint) warnings; `-W error` turns them into a non-zero exit (§10.4) |

### 12.4 Buses and `param`

| File | Contents |
|---|---|
| `examples/bus_or4.rv` | Bus `reg[N]`: wire all 4 lanes in one line with `in - r - buf;` |
| `examples/bus_and4.rv` | Bus ports and bus vars: bitwise AND of two 4-bit buses |
| `examples/bus_slice_concat.rv` | Slice `a[hi:lo]` (bit reversal) and concatenation `{a, b}` (left rotate) |
| `examples/slice_const_expr.rv` | Constant expressions in slice / lane indices (§6.3.1): splitting a bus with `x[W-1:W/2]`, plus `a[N+1]` |
| `examples/bus_scalar.rv` | Bus-to-scalar wiring: fan-in (MAX merge) and fan-out (broadcast) |
| `examples/param_notN.rv` | N-bit NOT with width parameterized by a `param` constant |
| `examples/generic_logic_width.rv` | Per-logic generic widths `#(W=4)`: instantiating one definition at 4 and 8 bits as separate instances |
| `examples/numeric_literals.rv` | Binary / hex integer literals (`0b1010` / `0xff`): usable in strengths, bus widths, `param`, `#define`, sim assignments, and tick counts (§1.3) |
| `examples/define_expr.rv` | Constant expressions in `#define` values (e.g. `(W*2)`) (§8.1) |
| `examples/monitor_bus.rv` | Pass a bus var directly to monitor; each lane packs into a 4-bit nibble for display (§7.4.1) |
| `examples/stdlogic_demo.rv` | The bundled standard library: `#include "stdlogic"` pulls in 7 basic gates (NOT / AND / OR / XOR / NAND / NOR / XNOR) and the demo sweeps them (§8.2.1) |

### 12.5 Waveform / Structured Output

| File | Contents |
|---|---|
| `examples/vcd_demo.rv` | Demo of dumping the waveform as VCD via `--vcd` (torch inversion + repeater delay) |
| `examples/json_output.rv` | Demo of emitting monitor / assert / warning as JSONL via `--json` |
