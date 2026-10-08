<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>match-lang</b>
    <br>
    <sub><sup>PATTERN MATCH CHECKING</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/match-lang"><img alt="Crates.io" src="https://img.shields.io/crates/v/match-lang"></a>
    <a href="https://crates.io/crates/match-lang"><img alt="Downloads" src="https://img.shields.io/crates/d/match-lang?color=%230099ff"></a>
    <a href="https://docs.rs/match-lang"><img alt="docs.rs" src="https://img.shields.io/docsrs/match-lang"></a>
    <a href="https://github.com/jamesgober/match-lang/actions"><img alt="CI" src="https://github.com/jamesgober/match-lang/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>match-lang</strong> checks pattern matches. Give it the arms of a match and the type being matched, and it reports the values no arm handles, each with a concrete pattern such as <code>Some(false)</code> or <code>10..=19</code>, the arms no value can reach, and the alternatives of an or-pattern that are redundant.
    </p>
    <p>
        It knows nothing about any particular language. A language plugs in by implementing one trait, <code>Model</code>, which says for each of its types which constructors build its values (a sum's variants, a struct's single constructor, an integer type's bounds, a slice's element type) and what each constructor's fields are. Sums, products, booleans, bounded and unbounded integers, characters, opaque literals such as strings, arrays, slices, or-patterns, and guards are all handled, so one implementation serves every language in the <code>-lang</code> family that has a <code>match</code>.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>no_std</code>-compatible (needs only <code>alloc</code>), <code>#![forbid(unsafe_code)]</code>, no dependencies.
    </p>
    <blockquote>
        <strong>Status: 0.2.0, pre-1.0.</strong> Exhaustiveness and usefulness checking are complete; decision-tree compilation arrives in 0.5.0. The API may change before 1.0, which follows once a real consumer has used it end to end. See <a href="./dev/ROADMAP.md"><code>dev/ROADMAP.md</code></a> and <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a>.
    </blockquote>
</div>

<hr>
<br>

## The model

- A **[`Model`](./docs/API.md#model)** is the language's side: `signature(ty)` returns the **[`Signature`](./docs/API.md#signature)** of a type, and `fields(ty, variant)` the field types of one of its constructors. Optional hooks say which types are empty (`!`, empty enums) and name variants for printing.
- A **[`Pat`](./docs/API.md#pat)** is a pattern, built bottom-up: `Pat::variant(1, [Pat::bool(true)])`, `Pat::range(0, 9)`, `Pat::or([..])`, `Pat::slice_rest([..], [..])`. An **[`Arm`](./docs/API.md#arm)** is a pattern, with or without a guard.
- **[`check`](./docs/API.md#check)** takes a model, the scrutinee's type, and the arms, and returns a **[`Report`](./docs/API.md#report)**: missing cases as **[`Witness`](./docs/API.md#witness)** patterns, unreachable arm indices, and **[`Redundant`](./docs/API.md#redundant)** or-alternatives. A **[`Checker`](./docs/API.md#checker)** does the same with configurable limits and reuses its buffers across checks.
- An **[`Error`](./docs/API.md#error)** means the check could not finish: a pattern that does not fit its type, a match too complex for the step budget, or a model that contradicts itself.

<br>

What it guarantees, and how each guarantee is checked:

| Guarantee | How it is held |
|---|---|
| A match is reported exhaustive exactly when every value is matched by an unguarded arm. | Property tests compare the checker with a brute-force reference that enumerates every value of random types (integers, characters, strings, and slices through exact representatives) and runs first-match semantics directly. |
| Every witness describes at least one value, and every value it describes really is unmatched. | The same reference checks every value each witness matches. |
| An arm is reported unreachable exactly when no value reaches it; an or-alternative is reported redundant exactly when no value reaches it. | The reference expands every or-pattern and decides usefulness from the definition, guards included. |
| Empty types are understood: `Err(_)` is not required for a `Result<T, !>`, and is unreachable. | Integration and property tests with empty types, variants, products, and slice elements. |
| Deep and wide matches are safe: no recursion, no quadratic copying. | Tests with 100,000-deep patterns and witnesses, and 100,000-field tuples, whose step counts stay linear. |
| Every check ends in bounded time and memory. | All work is charged to a step budget; matrices that need exponential work (random 3-SAT encodings, or-pattern explosions) return `Error::StepLimit`. A model that answers at random never causes a panic. |

<hr>
<br>

## Installation

```toml
[dependencies]
match-lang = "0.2"
```

Without the standard library:

```toml
[dependencies]
match-lang = { version = "0.2", default-features = false }
```

<hr>
<br>

## Quick start

A model for `bool` and `Option<T>`, and a match with a gap and a dead arm:

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

#[derive(Clone)]
enum Ty {
    Bool,
    Option(Box<Ty>),
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Bool => Signature::Bool,
            Ty::Option(_) => Signature::Sum { count: 2 }, // None, Some
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        if let (Ty::Option(inner), 1) = (ty, variant) {
            out.push((**inner).clone());
        }
    }

    fn variant_name(&self, _: &Ty, variant: u32) -> Option<&str> {
        Some(if variant == 0 { "None" } else { "Some" })
    }
}

// match x { Some(true) => .., None => .., Some(true) => .. }
let arms = [
    Arm::new(Pat::variant(1, [Pat::bool(true)])),
    Arm::new(Pat::variant(0, [])),
    Arm::new(Pat::variant(1, [Pat::bool(true)])),
];
let report = check(&Lang, &Ty::Option(Box::new(Ty::Bool)), &arms)?;

assert!(!report.is_exhaustive());
assert_eq!(report.missing()[0].display(&Lang).to_string(), "Some(false)");
assert_eq!(report.unreachable(), &[2]);
# Ok::<(), match_lang::Error>(())
```

### Integers and ranges

Integer types are covered range by range, up to their exact bounds:

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Bytes;

impl Model for Bytes {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Int { min: 0, max: 255 }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let arms = [Arm::new(Pat::range(0, 9)), Arm::new(Pat::int(11)), Arm::new(Pat::range(13, 255))];
let missing: Vec<String> =
    check(&Bytes, &(), &arms)?.missing().iter().map(|w| w.to_string()).collect();
assert_eq!(missing, ["10", "12"]);
# Ok::<(), match_lang::Error>(())
```

### Guards and or-patterns

A guard may fail, so a guarded arm never covers anything; redundant alternatives are reported by their position in the arm's pattern:

```rust
use match_lang::{check, Arm, Model, Pat, Redundant, Signature};

struct Flag;

impl Model for Flag {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Bool
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let arms = [
    Arm::guarded(Pat::wild()),                             // _ if cond
    Arm::new(Pat::or([Pat::bool(true), Pat::bool(true)])), // true | true
];
let report = check(&Flag, &(), &arms)?;
assert_eq!(report.missing()[0].to_string(), "false");
// Node 0 is the or-pattern; nodes 1 and 2 its alternatives.
assert_eq!(report.redundant(), &[Redundant { arm: 1, node: 2 }]);
# Ok::<(), match_lang::Error>(())
```

<hr>
<br>

## Examples

Runnable programs in [`examples/`](./examples):

| Example | What it shows |
|---|---|
| [`option`](./examples/option.rs) | The one-call path on `Option<bool>`: missing cases and an unreachable arm. `cargo run --example option` |
| [`language`](./examples/language.rs) | A small language's checker with named enums and structs, integers, interned strings, slices, guards, and or-patterns; a reused `Checker`; compiler-style messages that map report positions back to the language's own pattern tree. |

<hr>
<br>

## Performance

A check is one pass over the pattern matrix. Rows are persistent stacks that share their tails, so specializing a row pushes only the head constructor's fields; every arena is a stack truncated when its level finishes and reused by the next check. Columns are split into only the constructors their patterns mention, integer ranges into disjoint segments in one sorted sweep, and slice lengths into intervals. A row whose wildcard head can teach nothing under a constructor explored only for usefulness is not explored there, which is what keeps matrices such as one-test-per-row from going exponential.

Measured with the benchmarks in [`benches/`](./benches), Windows x86_64, Rust stable, release profile, one reused `Checker`:

| Benchmark | What it measures | Windows |
|---|---|---:|
| `enum_100k_variants` | 100,000 arms, one per variant of a 100,000-variant enum. | ~7.9 ms |
| `int_literals_100k` | 100,000 integer literals and a wildcard over `i64`. | ~10.9 ms |
| `int_ranges_100k_exhaustive` | 100,000 adjacent ranges covering all of `i64`. | ~9.4 ms |
| `truth_table_16` | All 65,536 rows of a 16-column boolean truth table. | ~77 ms |
| `wide_1000x1000` | 1,000 rows by 1,000 columns, each row testing one column. | ~55 ms |
| `nested_options_1000` | `None`, `Some(None)`, ... 1,000 levels deep (500,000 pattern nodes). | ~49 ms |
| `or_alternatives_100k` | 1,000 arms of 100 alternatives each. | ~11.4 ms |
| `slices_1000_lengths` | Fixed-length slice patterns of every length to 1,000, then `[..]`. | ~68 ms |
| `small_matches/reused_checker_10k` | 10,000 three-arm matches through one `Checker` (~0.41 µs each). | ~4.1 ms |
| `small_matches/fresh_check_10k` | The same through `check`, allocating each time (~1.3 µs each). | ~13.2 ms |

These are library-only numbers: building the `Pat` trees is not included. Linux and macOS figures come from the CI matrix and are not recorded here.

```bash
cargo bench --bench bench
```

Numbers vary by CPU; use the trend across runs, not a single absolute.

<hr>
<br>

## Design notes

- **Maranget's algorithm, as compilers run it.** Usefulness of every row is computed in one pass; a column is split into the constructors its patterns mention plus one "missing" constructor for the rest; witnesses are built on the way back. Witnesses come from the missing constructors wherever there are any, so they are a sample chosen to be informative, not a list of every missing value.
- **Exhaustiveness is NP-hard, so there is a budget.** Every unit of work counts against a step limit (about 16.7 million by default, configurable). A match that needs more is an `Error::StepLimit`, never a hang; the benchmark matches above use at most about 8.5 million steps, and `Report::steps` shows what a real match uses.
- **No recursion anywhere input reaches.** The specialization recursion runs on an explicit stack of frames; flattening, validation, `Drop`, `Clone`, and `Display` of patterns and witnesses are iterative.
- **Patterns are checked against their types first.** A pattern that does not fit (an integer pattern on an enum, a variant that does not exist, a range outside a type's bounds) is an `Error::InvalidPattern` naming the arm and node, before any analysis.
- **Exact in the presence of empty types.** A variant with an empty field has no values: it is never required and arms matching only it are unreachable. A slice of an empty type has only `[]`.
- **No first-party dependencies.** The crate depends on nothing; a language's types reach it only through `Model`, so `hir-lang` implements the trait rather than the other way round.

<hr>
<br>

## Testing

The suite runs on Windows, Linux, and macOS through the CI matrix, on stable and the 1.85 MSRV:

```bash
cargo test                       # unit + integration + property + doctests
cargo test --no-default-features # no_std + alloc
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench bench
```

[`tests/properties.rs`](./tests/properties.rs) holds the checker to a brute-force reference over random types up to three levels deep (sums with empty variants, products, booleans, bounded and unbounded integers, characters across the surrogate gap, opaque literals, arrays, slices) and random matches with nested or-patterns and guards: exhaustiveness, every witness, every unreachable arm, and every redundant alternative must agree. A one-off run of 200,000 cases per property found no disagreement, and each of more than a dozen deliberate bugs planted in the engine was caught. [`tests/adversarial.rs`](./tests/adversarial.rs) covers deep and wide matches, exponential matrices, and a model that answers at random. Every `rust` example in this README and in [`docs/API.md`](./docs/API.md) is compiled and run as a doctest.

<hr>
<br>

## Cross-platform support

- Linux (x86_64, aarch64)
- macOS (x86_64, Apple Silicon)
- Windows (x86_64)

The crate uses no operating-system facilities and no platform-specific code.

<hr>
<br>

## Contributing

See [`REPS.md`](./REPS.md) for the engineering standards every change is held to, [`dev/DIRECTIVES.md`](./dev/DIRECTIVES.md) for the definition of done, and [`dev/ROADMAP.md`](./dev/ROADMAP.md) for what comes next. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober.</strong></sup>
</div>
