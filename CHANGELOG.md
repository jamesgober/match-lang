<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>match-lang</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

---

## [0.2.0] - 2026-10-08

The foundation: exhaustiveness and usefulness checking for pattern matching,
generic over a language's constructor model. Given a match's arms and the
scrutinee's type, the checker reports the values no arm handles (each as a
concrete witness pattern), the arms no value reaches, and the redundant
alternatives of or-patterns.

### Added

- `Model`, the constructor-model trait a language implements: `signature`
  (the constructors of a type), `fields` (the field types of a sum variant or
  a product), and the optional `is_empty` (empty types) and `variant_name`
  (names for printing witnesses).
- `Signature`: `Sum`, `Product`, `Bool`, `Int { min, max }`, `Char`,
  `BigInt`, `Opaque`, `Array { len, elem }`, `Slice { elem }`
  (`#[non_exhaustive]`).
- `Pat`, built with `wild`, `variant`, `product`, `bool`, `int`, `range`,
  `char`, `char_range`, `lit`, `or`, `slice`, and `slice_rest`; iterative
  `Drop`, `Clone`, `Debug`, and `Display`. `Arm`, with `new` and `guarded`.
- `check`, the one-call entry point, and `Checker`, with
  `with_step_limit`, `with_witness_limit`, and buffers reused across checks;
  `DEFAULT_STEP_LIMIT` (2^24) and `DEFAULT_WITNESS_LIMIT` (16).
- `Report`: `is_exhaustive`, `missing`, `unreachable`, `redundant`, and
  `steps`. `Redundant` names an or-alternative by arm and preorder node index.
- `Witness`, `WitnessRef`, `WitnessFields`, and `WitnessDisplay`: witnesses
  as navigable trees of `Ctor` nodes with their types, printed in a neutral
  syntax or with the model's names.
- `Error` (`StepLimit`, `InvalidPattern`, `Model`, `TooLarge`) and `Reason`
  (`Kind`, `Variant`, `Fields`, `EmptyRange`, `OutOfDomain`, `Length`), both
  `#[non_exhaustive]`.
- The usefulness algorithm (Maranget, "Warnings for pattern matching", 2007)
  with constructor splitting: integer and character ranges cut into disjoint
  segments, slice lengths grouped into intervals, or-patterns expanded in
  place, guards treated as possibly failing, empty types and constructors
  with empty fields excluded, and row relevancy to avoid exponential blowup
  on ordinary matrices.
- Hardening: an explicit frame stack instead of recursion, rows as
  persistent stacks sharing their tails (no quadratic copying), a step
  budget on all work, and model inconsistencies reported as errors.
- Property tests against a brute-force reference, integration and
  adversarial tests, criterion benchmarks, and the `option` and `language`
  examples.

### Changed

- Version 0.2.0. Crate description updated to what the crate now does.

## [0.1.0] - 2026-10-08

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/match-lang/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jamesgober/match-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/match-lang/releases/tag/v0.1.0
