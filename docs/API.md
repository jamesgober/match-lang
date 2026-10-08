# match-lang &mdash; API Reference

> Complete reference for every public item in `match-lang`, with examples.
> **Status: 0.2.0, pre-1.0.** The surface may still change before `1.0`; see
> [`../dev/ROADMAP.md`](../dev/ROADMAP.md).

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Concepts](#concepts)
  - [Types and signatures](#types-and-signatures)
  - [What the checker decides](#what-the-checker-decides)
  - [Guards](#guards)
  - [Or-patterns and node numbering](#or-patterns-and-node-numbering)
  - [Empty types](#empty-types)
  - [Integers and characters](#integers-and-characters)
  - [Arrays and slices](#arrays-and-slices)
  - [Witnesses](#witnesses)
  - [Limits](#limits)
- [`check`](#check)
- [`Checker`](#checker)
- [`DEFAULT_STEP_LIMIT` and `DEFAULT_WITNESS_LIMIT`](#default_step_limit-and-default_witness_limit)
- [`Model`](#model)
- [`Signature`](#signature)
- [`Pat`](#pat)
- [`Arm`](#arm)
- [`Report`](#report)
- [`Redundant`](#redundant)
- [`Witness`](#witness)
- [`WitnessRef`](#witnessref)
- [`WitnessFields`](#witnessfields)
- [`WitnessDisplay`](#witnessdisplay)
- [`Ctor`](#ctor)
- [`Error`](#error)
- [`Reason`](#reason)
- [Feature flags](#feature-flags)
- [Guide: writing a model for a language](#guide-writing-a-model-for-a-language)

## Overview

`match-lang` checks pattern matches for **exhaustiveness** (does every value
have an arm?) and **usefulness** (can every arm, and every alternative of an
or-pattern, be reached?). It is generic over the language: a
[`Model`](#model) describes the language's types to it, and patterns are
built with [`Pat`](#pat).

| Item | Kind | Purpose |
|---|---|---|
| [`check`](#check) | fn | Checks a match with the default limits. |
| [`Checker`](#checker) | struct | Checks matches with configurable limits, reusing buffers. |
| [`DEFAULT_STEP_LIMIT`, `DEFAULT_WITNESS_LIMIT`](#default_step_limit-and-default_witness_limit) | consts | The default limits. |
| [`Model`](#model) | trait | The language's constructor model. |
| [`Signature`](#signature) | enum | The constructors of one type. |
| [`Pat`](#pat) | struct | A pattern. |
| [`Arm`](#arm) | struct | A pattern and whether it has a guard. |
| [`Report`](#report) | struct | Missing cases, unreachable arms, redundant alternatives. |
| [`Redundant`](#redundant) | struct | A redundant or-pattern alternative. |
| [`Witness`](#witness) | struct | A pattern describing unmatched values. |
| [`WitnessRef`](#witnessref) | struct | A node of a witness. |
| [`WitnessFields`](#witnessfields) | struct | Iterator over a witness node's fields. |
| [`WitnessDisplay`](#witnessdisplay) | struct | A witness printed with the model's names. |
| [`Ctor`](#ctor) | enum | The constructor at a witness node. |
| [`Error`](#error) | enum | Why a check could not finish. |
| [`Reason`](#reason) | enum | What is wrong with an invalid pattern. |

## Installation

```toml
[dependencies]
match-lang = "0.2"
```

For `no_std` targets (the crate needs only `alloc`):

```toml
[dependencies]
match-lang = { version = "0.2", default-features = false }
```

## Quick start

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

/// `Option<bool>`: 0 is the option, 1 the bool inside it.
struct Lang;

impl Model for Lang {
    type Ty = u8;

    fn signature(&self, ty: &u8) -> Signature<u8> {
        if *ty == 0 { Signature::Sum { count: 2 } } else { Signature::Bool }
    }

    fn fields(&self, _: &u8, variant: u32, out: &mut Vec<u8>) {
        if variant == 1 {
            out.push(1); // Some(bool)
        }
    }

    fn variant_name(&self, _: &u8, variant: u32) -> Option<&str> {
        Some(if variant == 0 { "None" } else { "Some" })
    }
}

let arms = [
    Arm::new(Pat::variant(0, [])),                 // None
    Arm::new(Pat::variant(1, [Pat::bool(false)])), // Some(false)
];
let report = check(&Lang, &0, &arms)?;
assert_eq!(report.missing()[0].display(&Lang).to_string(), "Some(true)");
assert!(report.unreachable().is_empty());
# Ok::<(), match_lang::Error>(())
```

## Concepts

### Types and signatures

The checker sees a type only through its [`Signature`](#signature): the set
of constructors that build its values. A sum type's constructors are its
variants; a product (tuple, struct, record, box, reference) has exactly one;
`bool` has `false` and `true`; an integer type has one constructor per value,
grouped into ranges; strings and other opaque values have infinitely many,
told apart by equality; arrays have one per length they can have, slices one
per length. [`Model::fields`](#model) gives the field types of a sum variant
or a product, and arrays and slices name their element type in the signature.

Each kind of [`Pat`](#pat) fits one kind of signature; a pattern that does
not fit its type is an [`Error::InvalidPattern`](#error), found before any
analysis. Wildcards and or-patterns fit every type.

### What the checker decides

For arms `p1, ..., pn` in order:

- The match is **exhaustive** when every value of the type is matched by some
  arm without a guard.
- Arm `i` is **reachable** when some value matches `pi` and no earlier
  unguarded arm.
- An alternative of an or-pattern in a reachable arm is **redundant** when no
  value reaches it: every value it matches is matched by an earlier unguarded
  arm, or (in an unguarded arm) by an earlier alternative of the same arm.

All three are exact for every signature. They are decided by Maranget's
usefulness algorithm and checked against a brute-force reference in the
property tests.

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Flag;

impl Model for Flag {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Bool
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let arms = [Arm::new(Pat::bool(true)), Arm::new(Pat::wild()), Arm::new(Pat::bool(false))];
let report = check(&Flag, &(), &arms)?;
assert!(report.is_exhaustive());
assert_eq!(report.unreachable(), &[2]);
# Ok::<(), match_lang::Error>(())
```

### Guards

The checker does not look at guard conditions; it assumes any guard may fail.
So a guarded arm:

- never counts toward exhaustiveness,
- never makes a later arm, or a later alternative, unreachable,
- is itself unreachable if every value its pattern matches is matched by an
  earlier unguarded arm.

Within a guarded arm the guard is assumed to be tried again for each
alternative of an or-pattern that matches (as Rust does), so the alternatives
of a guarded arm never make each other redundant.

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Flag;

impl Model for Flag {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Bool
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let arms = [Arm::guarded(Pat::bool(true)), Arm::new(Pat::bool(true))];
let report = check(&Flag, &(), &arms)?;
assert!(report.unreachable().is_empty()); // the guard may fail
assert_eq!(report.missing()[0].to_string(), "false");
# Ok::<(), match_lang::Error>(())
```

### Or-patterns and node numbering

Or-patterns may appear anywhere, nested to any depth. Each alternative that
no value reaches is reported as a [`Redundant`](#redundant) by its arm and
its **preorder index** in the arm's pattern: node 0 is the arm's root, and
every node is followed by the whole subtree of each of its children in turn
(fields, alternatives, or slice elements, prefix before suffix). A language
numbers its own pattern tree the same way to map the index back to source.

Only the outermost redundant alternative is reported; alternatives nested
inside it are not listed again. Alternatives of an unreachable arm are not
listed at all: the arm is.

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

// (false | (false | false)) => .., true => ..
//  node 0: the outer or; 1: false; 2: the inner or; 3, 4: its alternatives.
let arms = [
    Arm::new(Pat::or([Pat::bool(false), Pat::or([Pat::bool(false), Pat::bool(false)])])),
    Arm::new(Pat::bool(true)),
];
let report = check(&Flag, &(), &arms)?;
assert_eq!(report.redundant(), &[Redundant { arm: 0, node: 2 }]);
# Ok::<(), match_lang::Error>(())
```

### Empty types

A type with no values (the never type, an enum with no variants) is
`Signature::Sum { count: 0 }`, or anything the model's
[`is_empty`](#model) says is empty. A constructor with an empty field has no
values either. Such constructors are never required, and arms that only they
could reach are unreachable:

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

/// `Result<bool, !>`: 0 is the result, 1 is `bool`, 2 is `!`.
struct Lang;

impl Model for Lang {
    type Ty = u8;
    fn signature(&self, ty: &u8) -> Signature<u8> {
        match ty {
            0 => Signature::Sum { count: 2 },
            1 => Signature::Bool,
            _ => Signature::Sum { count: 0 },
        }
    }
    fn fields(&self, _: &u8, variant: u32, out: &mut Vec<u8>) {
        out.push(if variant == 0 { 1 } else { 2 }); // Ok(bool), Err(!)
    }
}

let ok_only = [Arm::new(Pat::variant(0, [Pat::wild()]))];
assert!(check(&Lang, &0, &ok_only)?.is_exhaustive());

let with_err = [
    Arm::new(Pat::variant(0, [Pat::wild()])),
    Arm::new(Pat::variant(1, [Pat::wild()])),
];
assert_eq!(check(&Lang, &0, &with_err)?.unreachable(), &[1]);
# Ok::<(), match_lang::Error>(())
```

For exactness `is_empty` should be deep: a tuple or struct with an empty
field is itself empty. The default only recognizes `Sum { count: 0 }`.

### Integers and characters

`Signature::Int { min, max }` is the integers `min..=max`, matched by
literals and inclusive ranges; covering every value covers the type, and
missing values are reported as maximal ranges. Values are `i128`, which holds
every fixed-width integer type except the top half of `u128`; map `u128` by
flipping its top bit (`(v ^ (1 << 127)) as i128`), which keeps the order.
`Signature::BigInt` is unbounded: ranges never cover it, and the missing value
is reported as [`Ctor::Other`](#ctor).

`Signature::Char` is the Unicode scalar values. A character range covers
every scalar value between its ends, so `'\0'..='\u{10FFFF}'` covers the type
even though it spans the surrogate block.

```rust
use match_lang::{check, Arm, Ctor, Model, Pat, Signature};

struct Chars;

impl Model for Chars {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Char
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let arms = [Arm::new(Pat::char_range('\0', '\u{D7FF}'))];
let report = check(&Chars, &(), &arms)?;
assert_eq!(report.missing()[0].ctor(), Ctor::Char { lo: '\u{E000}', hi: char::MAX });
# Ok::<(), match_lang::Error>(())
```

Floats compared by value fit `Signature::Opaque` (equality only). If the
language allows float ranges, map each float to an integer key that orders
like the floats and use `Signature::Int`.

### Arrays and slices

`Signature::Array { len, elem }` is arrays of exactly `len` elements;
`Signature::Slice { elem }` is sequences of any length. Both are matched by
fixed-length patterns ([`Pat::slice`](#pat), `[a, b, c]`) and rest patterns
([`Pat::slice_rest`](#pat), `[a, .., z]`, which matches any length from the
prefix plus the suffix up).

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Bools;

impl Model for Bools {
    type Ty = u8; // 0: a slice of bools, 1: bool
    fn signature(&self, ty: &u8) -> Signature<u8> {
        if *ty == 0 { Signature::Slice { elem: 1 } } else { Signature::Bool }
    }
    fn fields(&self, _: &u8, _: u32, _: &mut Vec<u8>) {}
}

let arms = [
    Arm::new(Pat::slice([])),
    Arm::new(Pat::slice_rest([Pat::bool(true)], [])),
    Arm::new(Pat::slice_rest([], [Pat::bool(true)])),
];
let missing: Vec<String> =
    check(&Bools, &0, &arms)?.missing().iter().map(|w| w.to_string()).collect();
assert_eq!(missing, ["[false]", "[false, .., false]"]);
# Ok::<(), match_lang::Error>(())
```

### Witnesses

When a match is not exhaustive, [`Report::missing`](#report) holds at least
one [`Witness`](#witness): a pattern without or-patterns whose every value is
matched by no unguarded arm, and which matches at least one value. A
[`Wild`](#ctor) node stands for any value of its type, an [`Other`](#ctor)
node for a value no pattern can name (an unmentioned string, an integer
beyond `i128`).

Witnesses are a sample, not an enumeration of every missing value. At the
top of the match every missing constructor is named (`Red`, `Blue`); deeper
down, a column no arm tests is `_`. Once a missing constructor proves the
match non-exhaustive, the constructors that are present are not explored for
further witnesses, which is what keeps the analysis fast. A run of missing
slice lengths is shown at its shortest length. At most the
[witness limit](#checker) are kept.

### Limits

Deciding exhaustiveness is NP-hard in general, so every check runs against
a **step budget**: a unit is roughly one matrix row, cell, or witness node
built, or one constructor examined. A check that would exceed it stops with
[`Error::StepLimit`](#error), so time and memory per check are bounded
whatever the input. The default,
[`DEFAULT_STEP_LIMIT`](#default_step_limit-and-default_witness_limit), is
about 16.7 million; the largest benchmark matches (100,000 arms, a
65,536-row truth table, 1,000 by 1,000 columns) use at most about 8.5
million. [`Report::steps`](#report) reports what a check used.

Nothing in the checker recurses on input: deep and wide patterns cost heap,
not stack. Patterns, matrix rows, and cells are each limited to
`u32::MAX - 1` ([`Error::TooLarge`](#error)).

## `check`

```rust,ignore
pub fn check<M: Model>(model: &M, ty: &M::Ty, arms: &[Arm]) -> Result<Report<M::Ty>, Error>
```

Checks `arms`, in order, against values of type `ty`, with the default
limits. The one-call entry point; it creates a fresh [`Checker`](#checker)
each time, so for many matches keep a `Checker` instead.

**Errors:** [`Error::InvalidPattern`](#error) if a pattern does not fit its
type, [`Error::StepLimit`](#error) past the default step limit,
[`Error::Model`](#error) if the model contradicts itself,
[`Error::TooLarge`](#error) past `u32::MAX - 1` nodes or matrix entries.

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

let arms = [Arm::new(Pat::range(0, 127)), Arm::new(Pat::range(128, 255))];
assert!(check(&Bytes, &(), &arms)?.is_exhaustive());
# Ok::<(), match_lang::Error>(())
```

## `Checker`

```rust,ignore
pub struct Checker<T> { /* private */ }
```

Checks matches, reusing its buffers from one check to the next. After the
first few checks it allocates almost nothing beyond the reports it returns.
`Checker<T>` implements `Default` and `Debug` (showing its limits).

| Method | Description |
|---|---|
| `Checker::new() -> Checker<T>` | A checker with the default limits. |
| `with_step_limit(self, limit: u64) -> Checker<T>` | Sets the step budget per check. |
| `with_witness_limit(self, limit: usize) -> Checker<T>` | Sets the most missing cases a report lists; `0` is treated as `1`. |
| `step_limit(&self) -> u64` | The step limit in effect. |
| `witness_limit(&self) -> usize` | The witness limit in effect. |
| `check(&mut self, model: &M, ty: &T, arms: &[Arm]) -> Result<Report<T>, Error>` | Checks a match; errors as for [`check`](#check). |

A check that fails leaves the checker ready for the next one.

```rust
use match_lang::{Arm, Checker, Error, Model, Pat, Signature};

struct Bytes;

impl Model for Bytes {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Int { min: 0, max: 255 }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let mut checker = Checker::new().with_witness_limit(2);
let even: Vec<Arm> = (0..128).map(|i| Arm::new(Pat::int(i * 2))).collect();
let report = checker.check(&Bytes, &(), &even)?;
assert_eq!(report.missing().len(), 2);

let mut tight = Checker::new().with_step_limit(100);
assert_eq!(tight.step_limit(), 100);
assert_eq!(tight.check(&Bytes, &(), &even).unwrap_err(), Error::StepLimit { limit: 100 });
// A failed check leaves the checker usable.
assert!(tight.check(&Bytes, &(), &[Arm::new(Pat::wild())])?.is_exhaustive());
# Ok::<(), match_lang::Error>(())
```

## `DEFAULT_STEP_LIMIT` and `DEFAULT_WITNESS_LIMIT`

```rust,ignore
pub const DEFAULT_STEP_LIMIT: u64 = 1 << 24;
pub const DEFAULT_WITNESS_LIMIT: usize = 16;
```

The limits [`check`](#check) and `Checker::new` use.

```rust
use match_lang::{Checker, DEFAULT_STEP_LIMIT, DEFAULT_WITNESS_LIMIT};

let checker: Checker<()> = Checker::new();
assert_eq!(checker.step_limit(), DEFAULT_STEP_LIMIT);
assert_eq!(checker.witness_limit(), DEFAULT_WITNESS_LIMIT);
```

## `Model`

```rust,ignore
pub trait Model {
    type Ty: Clone;
    fn signature(&self, ty: &Self::Ty) -> Signature<Self::Ty>;
    fn fields(&self, ty: &Self::Ty, variant: u32, out: &mut Vec<Self::Ty>);
    fn is_empty(&self, ty: &Self::Ty) -> bool { /* Sum { count: 0 } */ }
    fn variant_name(&self, ty: &Self::Ty, variant: u32) -> Option<&str> { None }
}
```

The language's constructor model.

| Item | Description |
|---|---|
| `Ty` | A type of the language. An interned id is ideal: the checker clones types as it descends into fields. |
| `signature(ty)` | The [`Signature`](#signature) of `ty`. |
| `fields(ty, variant, out)` | Pushes the field types of variant `variant` of a `Sum`, or of a `Product` (`variant` 0), in order. `out` is empty on entry. Not called for other signatures. |
| `is_empty(ty)` | Whether `ty` has no values. Should be deep: a product with an empty field is empty. Default: `true` exactly for `Sum { count: 0 }`. |
| `variant_name(ty, variant)` | A name for printing witnesses with [`Witness::display`](#witness): variant `variant` of a sum, or the product itself with `variant` 0. Default: none, so variants print as `#index` and products as tuples. |

The model must answer the same way for the same type throughout a check; a
contradiction is reported as [`Error::Model`](#error), never a panic.

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

/// `(bool, Color)` with `enum Color { Red, Green, Blue }`.
#[derive(Clone, PartialEq)]
enum Ty {
    Pair,
    Bool,
    Color,
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Pair => Signature::Product,
            Ty::Bool => Signature::Bool,
            Ty::Color => Signature::Sum { count: 3 },
        }
    }

    fn fields(&self, ty: &Ty, _: u32, out: &mut Vec<Ty>) {
        if *ty == Ty::Pair {
            out.extend([Ty::Bool, Ty::Color]);
        }
    }

    fn variant_name(&self, ty: &Ty, variant: u32) -> Option<&str> {
        match ty {
            Ty::Color => ["Red", "Green", "Blue"].get(variant as usize).copied(),
            _ => None,
        }
    }
}

let arms = [
    Arm::new(Pat::product([Pat::bool(true), Pat::wild()])),
    Arm::new(Pat::product([Pat::wild(), Pat::variant(0, [])])),
];
let report = check(&Lang, &Ty::Pair, &arms)?;
let missing: Vec<String> = report.missing().iter().map(|w| w.display(&Lang).to_string()).collect();
assert_eq!(missing, ["(false, Green)", "(false, Blue)"]);
# Ok::<(), match_lang::Error>(())
```

## `Signature`

```rust,ignore
#[non_exhaustive]
pub enum Signature<T> {
    Sum { count: u32 },
    Product,
    Bool,
    Int { min: i128, max: i128 },
    Char,
    BigInt,
    Opaque,
    Array { len: u32, elem: T },
    Slice { elem: T },
}
```

The constructors of one type. Derives `Clone`, `Debug`, `PartialEq`, `Eq`,
`Hash`.

| Variant | Values | Patterns |
|---|---|---|
| `Sum { count }` | variants `0..count`, fields from `Model::fields`; `count` 0 is empty | `Pat::variant` |
| `Product` | one constructor, fields from `Model::fields` with variant 0 | `Pat::product` |
| `Bool` | `false`, `true` | `Pat::bool` |
| `Int { min, max }` | the integers `min..=max` (`min > max` is a model error) | `Pat::int`, `Pat::range` |
| `Char` | the Unicode scalar values | `Pat::char`, `Pat::char_range` |
| `BigInt` | all integers; patterns name only `i128` values | `Pat::int`, `Pat::range` |
| `Opaque` | infinitely many values, told apart by equality | `Pat::lit` |
| `Array { len, elem }` | sequences of exactly `len` elements | `Pat::slice` (exactly `len`), `Pat::slice_rest` |
| `Slice { elem }` | sequences of any length | `Pat::slice`, `Pat::slice_rest` |

Wildcards and or-patterns fit every signature.

```rust
use match_lang::Signature;

let u8_sig: Signature<()> = Signature::Int { min: 0, max: 255 };
let never: Signature<()> = Signature::Sum { count: 0 };
assert_ne!(u8_sig, never);
```

## `Pat`

```rust,ignore
pub struct Pat { /* private */ }
```

A pattern, built bottom-up. `Pat` implements `Clone`, `Debug`, and
`Display` (in a neutral syntax), all iterative, as is its `Drop`.

| Constructor | Pattern | Fits |
|---|---|---|
| `Pat::wild()` | `_` (also any binding) | any type |
| `Pat::variant(index: u32, fields)` | variant `index` with one pattern per field | `Sum` |
| `Pat::product(fields)` | the single constructor, one pattern per field (`_` for fields a struct pattern omits with `..`) | `Product` |
| `Pat::bool(value)` | `true` or `false` | `Bool` |
| `Pat::int(value: i128)` | one integer | `Int`, `BigInt` |
| `Pat::range(lo: i128, hi: i128)` | `lo..=hi`; an exclusive range is `range(lo, hi - 1)` | `Int`, `BigInt` |
| `Pat::char(value)` | one character | `Char` |
| `Pat::char_range(lo, hi)` | `lo..=hi` | `Char` |
| `Pat::lit(id: u64)` | an opaque literal by id: the caller interns strings, floats, symbols | `Opaque` |
| `Pat::or(alternatives)` | `p \| q \| ...`; no alternatives matches nothing | any type |
| `Pat::slice(elements)` | `[p1, ..., pn]`: exactly `n` elements | `Array` (with `n == len`), `Slice` |
| `Pat::slice_rest(prefix, suffix)` | `[p.., .., q..]`: at least `prefix + suffix` elements | `Array`, `Slice` |

`fields`, `alternatives`, `elements`, `prefix`, and `suffix` take any
`IntoIterator<Item = Pat>`.

```rust
use match_lang::Pat;

let pat = Pat::variant(1, [Pat::or([Pat::int(1), Pat::range(5, 9)]), Pat::wild()]);
assert_eq!(pat.to_string(), "#1(1 | 5..=9, _)");
assert_eq!(Pat::slice_rest([Pat::char('a')], [Pat::lit(7)]).to_string(), "['a', .., lit#7]");
assert_eq!(Pat::product([Pat::bool(true)]).to_string(), "(true,)");
```

## `Arm`

```rust,ignore
pub struct Arm { /* private */ }
```

One arm of a match: a pattern and whether it has a guard (see
[Guards](#guards)). Derives `Clone`, `Debug`.

| Method | Description |
|---|---|
| `Arm::new(pat) -> Arm` | An arm without a guard. |
| `Arm::guarded(pat) -> Arm` | An arm with a guard (`pat if ...`). |
| `pat(&self) -> &Pat` | The pattern. |
| `is_guarded(&self) -> bool` | Whether there is a guard. |

```rust
use match_lang::{Arm, Pat};

let arm = Arm::guarded(Pat::int(0));
assert!(arm.is_guarded());
assert_eq!(arm.pat().to_string(), "0");
```

## `Report`

```rust,ignore
pub struct Report<T> { /* private */ }
```

The result of a check. Derives `Clone`, `Debug`.

| Method | Description |
|---|---|
| `is_exhaustive(&self) -> bool` | Every value is matched by an unguarded arm. |
| `missing(&self) -> &[Witness<T>]` | Witnesses of unmatched values: empty exactly when exhaustive, otherwise between one and the witness limit. |
| `unreachable(&self) -> &[usize]` | Indices of arms no value reaches, ascending. |
| `redundant(&self) -> &[Redundant]` | Redundant or-alternatives of reachable arms, in arm order and then preorder. |
| `steps(&self) -> u64` | Steps the check used, against the step limit. |

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Flag;

impl Model for Flag {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Bool
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let report = check(&Flag, &(), &[Arm::new(Pat::bool(true))])?;
assert!(!report.is_exhaustive());
assert_eq!(report.missing().len(), 1);
assert!(report.unreachable().is_empty() && report.redundant().is_empty());
assert!(report.steps() > 0);
# Ok::<(), match_lang::Error>(())
```

## `Redundant`

```rust,ignore
pub struct Redundant {
    pub arm: usize,
    pub node: usize,
}
```

A redundant or-pattern alternative: the alternative rooted at node `node`
([preorder index](#or-patterns-and-node-numbering)) of arm `arm`'s pattern.
Derives `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`,
`Hash`; ordered by arm, then node.

```rust
use match_lang::Redundant;

let mut found = vec![Redundant { arm: 1, node: 0 }, Redundant { arm: 0, node: 3 }];
found.sort();
assert_eq!(found[0], Redundant { arm: 0, node: 3 });
```

## `Witness`

```rust,ignore
pub struct Witness<T> { /* private */ }
```

A pattern describing values no unguarded arm matches (see
[Witnesses](#witnesses)). Implements `Clone`, `PartialEq`, `Eq` (for
`T: PartialEq`/`Eq`), `Debug`, and `Display` (neutral syntax: `#index` for
unnamed variants, tuples for unnamed products, `_` for `Wild` and `Other`).

| Method | Description |
|---|---|
| `root(&self) -> WitnessRef<'_, T>` | The root node. |
| `ctor(&self) -> Ctor` | The root's constructor. |
| `ty(&self) -> &T` | The root's type: the scrutinee type. |
| `fields(&self) -> WitnessFields<'_, T>` | The root's fields. |
| `display(&self, model: &M) -> WitnessDisplay<'_, M>` | Prints with the model's variant names. |

```rust
use match_lang::{check, Arm, Ctor, Model, Pat, Signature};

struct Opt;

impl Model for Opt {
    type Ty = u8; // 0: Option<bool>, 1: bool
    fn signature(&self, ty: &u8) -> Signature<u8> {
        if *ty == 0 { Signature::Sum { count: 2 } } else { Signature::Bool }
    }
    fn fields(&self, _: &u8, variant: u32, out: &mut Vec<u8>) {
        if variant == 1 {
            out.push(1);
        }
    }
    fn variant_name(&self, _: &u8, variant: u32) -> Option<&str> {
        Some(if variant == 0 { "None" } else { "Some" })
    }
}

let arms = [Arm::new(Pat::variant(0, [])), Arm::new(Pat::variant(1, [Pat::bool(false)]))];
let report = check(&Opt, &0, &arms)?;
let w = &report.missing()[0];
assert_eq!((w.ctor(), *w.ty()), (Ctor::Variant(1), 0));
assert_eq!(w.fields().next().unwrap().ctor(), Ctor::Bool(true));
assert_eq!(w.to_string(), "#1(true)");
assert_eq!(w.display(&Opt).to_string(), "Some(true)");
# Ok::<(), match_lang::Error>(())
```

## `WitnessRef`

```rust,ignore
pub struct WitnessRef<'a, T> { /* private */ }
```

A node of a witness. `Copy`; implements `Display` and `Debug` like
[`Witness`](#witness).

| Method | Description |
|---|---|
| `ctor(&self) -> Ctor` | The constructor at this node. |
| `ty(&self) -> &'a T` | The type of the value at this node. |
| `arity(&self) -> usize` | The number of fields. |
| `fields(&self) -> WitnessFields<'a, T>` | The fields, in order. |
| `display(self, model: &'a M) -> WitnessDisplay<'a, M>` | Prints this subtree with the model's names. |

For a `Slice { len }` node the fields are the `len` elements; for a
`SliceRest { prefix, suffix }` node, the first `prefix` and last `suffix`
elements.

```rust
use match_lang::{check, Arm, Ctor, Model, Pat, Signature};

struct Pair;

impl Model for Pair {
    type Ty = bool; // true: (bool, bool), false: bool
    fn signature(&self, ty: &bool) -> Signature<bool> {
        if *ty { Signature::Product } else { Signature::Bool }
    }
    fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) {
        out.extend([false, false]);
    }
}

let arms = [Arm::new(Pat::product([Pat::wild(), Pat::bool(true)]))];
let report = check(&Pair, &true, &arms)?;
let root = report.missing()[0].root();
assert_eq!((root.ctor(), root.arity()), (Ctor::Product, 2));
let ctors: Vec<Ctor> = root.fields().map(|f| f.ctor()).collect();
assert_eq!(ctors, [Ctor::Wild, Ctor::Bool(false)]);
# Ok::<(), match_lang::Error>(())
```

## `WitnessFields`

```rust,ignore
pub struct WitnessFields<'a, T> { /* private */ }
```

The fields of a witness node, in order: an `ExactSizeIterator` of
[`WitnessRef`](#witnessref). Returned by `Witness::fields` and
`WitnessRef::fields`.

```rust
use match_lang::{check, Model, Signature};

struct Triple;

impl Model for Triple {
    type Ty = bool; // true: (bool, bool, bool), false: bool
    fn signature(&self, ty: &bool) -> Signature<bool> {
        if *ty { Signature::Product } else { Signature::Bool }
    }
    fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) {
        out.extend([false, false, false]);
    }
}

let report = check(&Triple, &true, &[])?;
assert_eq!(report.missing()[0].fields().len(), 3);
# Ok::<(), match_lang::Error>(())
```

## `WitnessDisplay`

```rust,ignore
pub struct WitnessDisplay<'a, M: Model> { /* private */ }
```

A witness printed with [`Model::variant_name`](#model): named variants print
as `Name` or `Name(fields)`, named products as `Name(fields)`. Implements
`Display` and `Debug`. Returned by `Witness::display` and
`WitnessRef::display`.

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

struct Color;

impl Model for Color {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Sum { count: 3 }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    fn variant_name(&self, _: &(), variant: u32) -> Option<&str> {
        ["Red", "Green", "Blue"].get(variant as usize).copied()
    }
}

let report = check(&Color, &(), &[Arm::new(Pat::variant(1, []))])?;
let names: Vec<String> = report.missing().iter().map(|w| w.display(&Color).to_string()).collect();
assert_eq!(names, ["Red", "Blue"]);
# Ok::<(), match_lang::Error>(())
```

## `Ctor`

```rust,ignore
#[non_exhaustive]
pub enum Ctor {
    Wild,
    Other,
    Variant(u32),
    Product,
    Bool(bool),
    Int { lo: i128, hi: i128 },
    Char { lo: char, hi: char },
    Lit(u64),
    Slice { len: u32 },
    SliceRest { prefix: u32, suffix: u32 },
}
```

The constructor at a witness node. Derives `Clone`, `Copy`, `Debug`,
`PartialEq`, `Eq`, `Hash`.

| Variant | Stands for | Fields |
|---|---|---|
| `Wild` | any value of the type | none |
| `Other` | a value no pattern can name: an opaque literal no arm mentions, a `BigInt` beyond `i128` | none |
| `Variant(i)` | variant `i` | the variant's fields |
| `Product` | the product's constructor | its fields |
| `Bool(b)` | `b` | none |
| `Int { lo, hi }` | any integer in `lo..=hi` | none |
| `Char { lo, hi }` | any character in `lo..=hi` | none |
| `Lit(id)` | the opaque literal `id` | none |
| `Slice { len }` | sequences of exactly `len` elements | the elements |
| `SliceRest { prefix, suffix }` | sequences of at least `prefix + suffix` elements | the first `prefix` and last `suffix` elements |

```rust
use match_lang::{check, Arm, Ctor, Model, Pat, Signature};

struct Strings;

impl Model for Strings {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Opaque
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let report = check(&Strings, &(), &[Arm::new(Pat::lit(0)), Arm::new(Pat::lit(1))])?;
assert_eq!(report.missing()[0].ctor(), Ctor::Other);
# Ok::<(), match_lang::Error>(())
```

## `Error`

```rust,ignore
#[non_exhaustive]
pub enum Error {
    StepLimit { limit: u64 },
    InvalidPattern { arm: usize, node: usize, reason: Reason },
    Model { reason: &'static str },
    TooLarge,
}
```

Why a check could not produce a report. Derives `Clone`, `Debug`,
`PartialEq`, `Eq`; implements `Display` and `core::error::Error`.

| Variant | Meaning | What to do |
|---|---|---|
| `StepLimit { limit }` | The analysis needed more than `limit` steps. | Raise the limit if the match is legitimate, or report it as too complex to check. |
| `InvalidPattern { arm, node, reason }` | Node `node` (preorder) of arm `arm` does not fit its type; nothing was analysed. | A type error the language's checker should have caught: report it there. |
| `Model { reason }` | The model contradicted itself (or gave an `Int` with `min > max`). | Fix the model. |
| `TooLarge` | More than `u32::MAX - 1` pattern nodes, rows, or cells. | Split the match. |

```rust
use match_lang::{check, Arm, Error, Model, Pat, Reason, Signature};

struct Flag;

impl Model for Flag {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Bool
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let err = check(&Flag, &(), &[Arm::new(Pat::or([Pat::bool(true), Pat::int(1)]))]).unwrap_err();
assert_eq!(err, Error::InvalidPattern { arm: 0, node: 2, reason: Reason::Kind });
assert_eq!(err.to_string(), "arm 0, pattern node 2: pattern does not fit the type");
```

## `Reason`

```rust,ignore
#[non_exhaustive]
pub enum Reason {
    Kind,
    Variant { index: u32, count: u32 },
    Fields { expected: usize, found: usize },
    EmptyRange,
    OutOfDomain,
    Length { expected: u32, found: usize },
}
```

What is wrong with an invalid pattern. Derives `Clone`, `Copy`, `Debug`,
`PartialEq`, `Eq`, `Hash`; implements `Display`.

| Variant | Meaning |
|---|---|
| `Kind` | The pattern kind does not fit the type's signature (an integer pattern on a sum). |
| `Variant { index, count }` | Variant `index` of a sum with `count` variants. |
| `Fields { expected, found }` | A variant or product pattern with the wrong number of fields. |
| `EmptyRange` | A range whose low end is above its high end. |
| `OutOfDomain` | An integer outside an `Int` type's bounds. |
| `Length { expected, found }` | An array pattern that cannot have the array's length. |

```rust
use match_lang::{check, Arm, Error, Model, Pat, Reason, Signature};

struct Bytes;

impl Model for Bytes {
    type Ty = ();
    fn signature(&self, _: &()) -> Signature<()> {
        Signature::Int { min: 0, max: 255 }
    }
    fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
}

let err = check(&Bytes, &(), &[Arm::new(Pat::range(9, 3))]).unwrap_err();
assert_eq!(err, Error::InvalidPattern { arm: 0, node: 0, reason: Reason::EmptyRange });
```

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Nothing beyond `alloc` is used; without it the crate is `no_std`. |

## Guide: writing a model for a language

A front end that lowers source to a typed IR implements `Model` over its
type ids. The mapping, type by type:

- **Enums and tagged unions** are `Sum { count }`, with `fields` returning
  each variant's payload types in declaration order. A unit variant has no
  fields; a struct-like variant lists its fields in a fixed order, and its
  patterns put `_` for fields a pattern omits.
- **Tuples, structs, records, boxes, references** are `Product`. A reference
  pattern `&p` or a box pattern is `Pat::product([p])`; for a language where
  the reference is transparent, skip it in both the model and the patterns.
- **`bool`** is `Bool`; **fixed-width integers** `Int` with the type's
  bounds; **arbitrary-precision integers** `BigInt`; **`char`** `Char`.
- **Strings, floats, symbols** are `Opaque`, with the literal interned to an
  id (`intern-lang` symbols work directly).
- **Arrays** are `Array { len, elem }`; **slices, lists, and vectors**
  matched by shape are `Slice { elem }`. A cons list defined as an enum is
  just a `Sum`.
- **The never type and empty enums** are `Sum { count: 0 }`; implement
  `is_empty` so that products and arrays containing one are empty too.
- **Dynamic languages**, where a match can see values of any runtime type,
  model the universal type as a `Sum` over runtime tags, each tag a variant
  whose fields are that tag's components.

Patterns map one to one: bindings and `_` are `Pat::wild()`, `x @ p` is `p`,
and `lo..hi` is `Pat::range(lo, hi - 1)`. Lower guards to `Arm::guarded`.

To report a redundant alternative at its source location, number the
language's own pattern nodes in preorder as described in
[Or-patterns and node numbering](#or-patterns-and-node-numbering), or keep a
side table from preorder index to span while lowering.

```rust
use match_lang::{check, Arm, Model, Pat, Signature};

/// `enum Expr { Num(i64), Neg(Box<Expr>), Add(Box<Expr>, Box<Expr>) }`
#[derive(Clone, Copy, PartialEq)]
enum Ty {
    Expr,
    I64,
    BoxExpr,
}

struct Lang;

impl Model for Lang {
    type Ty = Ty;

    fn signature(&self, ty: &Ty) -> Signature<Ty> {
        match ty {
            Ty::Expr => Signature::Sum { count: 3 },
            Ty::I64 => Signature::Int { min: i64::MIN.into(), max: i64::MAX.into() },
            Ty::BoxExpr => Signature::Product,
        }
    }

    fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
        match (ty, variant) {
            (Ty::Expr, 0) => out.push(Ty::I64),
            (Ty::Expr, 1) => out.push(Ty::BoxExpr),
            (Ty::Expr, _) => out.extend([Ty::BoxExpr, Ty::BoxExpr]),
            (Ty::BoxExpr, _) => out.push(Ty::Expr),
            _ => {}
        }
    }

    fn variant_name(&self, ty: &Ty, variant: u32) -> Option<&str> {
        match ty {
            Ty::Expr => ["Num", "Neg", "Add"].get(variant as usize).copied(),
            Ty::BoxExpr => Some("Box"),
            Ty::I64 => None,
        }
    }
}

let boxed = |p: Pat| Pat::product([p]);
// match e { Num(0) => .., Num(n) if n < 0 => .., Neg(box Num(_)) => .., Add(..) => .. }
let arms = [
    Arm::new(Pat::variant(0, [Pat::int(0)])),
    Arm::guarded(Pat::variant(0, [Pat::wild()])),
    Arm::new(Pat::variant(1, [boxed(Pat::variant(0, [Pat::wild()]))])),
    Arm::new(Pat::variant(2, [Pat::wild(), Pat::wild()])),
];
let report = check(&Lang, &Ty::Expr, &arms)?;
let missing: Vec<String> =
    report.missing().iter().map(|w| w.display(&Lang).to_string()).collect();
assert_eq!(
    missing,
    [
        format!("Num({}..=-1)", i64::MIN),
        format!("Num(1..={})", i64::MAX),
        "Neg(Box(Neg(_)))".to_string(),
        "Neg(Box(Add(_, _)))".to_string(),
    ]
);
# Ok::<(), match_lang::Error>(())
```
