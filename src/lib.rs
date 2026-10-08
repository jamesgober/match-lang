//! # match-lang
//!
//! Exhaustiveness and usefulness checking for pattern matching, generic over
//! a language's types.
//!
//! Give it the arms of a match and the type being matched, and it tells you
//! which values no arm handles (with a concrete pattern for each), which arms
//! can never be reached, and which alternatives of an or-pattern are
//! redundant. A language plugs in by implementing [`Model`]: for each of its
//! types, a [`Signature`] saying which constructors build its values, and the
//! field types of each constructor. The crate knows nothing else about the
//! language, so every language in the family shares one implementation.
//!
//! ## Quick start
//!
//! ```
//! use match_lang::{check, Arm, Model, Pat, Signature};
//!
//! // The language: `bool` and `Option<T>`.
//! #[derive(Clone)]
//! enum Ty {
//!     Bool,
//!     Option(Box<Ty>),
//! }
//!
//! struct Lang;
//!
//! impl Model for Lang {
//!     type Ty = Ty;
//!
//!     fn signature(&self, ty: &Ty) -> Signature<Ty> {
//!         match ty {
//!             Ty::Bool => Signature::Bool,
//!             Ty::Option(_) => Signature::Sum { count: 2 }, // None, Some
//!         }
//!     }
//!
//!     fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
//!         if let (Ty::Option(inner), 1) = (ty, variant) {
//!             out.push((**inner).clone());
//!         }
//!     }
//!
//!     fn variant_name(&self, _: &Ty, variant: u32) -> Option<&str> {
//!         Some(if variant == 0 { "None" } else { "Some" })
//!     }
//! }
//!
//! // match x: Option<bool> { Some(true) => .., None => .., Some(true) => .. }
//! let arms = [
//!     Arm::new(Pat::variant(1, [Pat::bool(true)])),
//!     Arm::new(Pat::variant(0, [])),
//!     Arm::new(Pat::variant(1, [Pat::bool(true)])),
//! ];
//! let report = check(&Lang, &Ty::Option(Box::new(Ty::Bool)), &arms)?;
//!
//! assert!(!report.is_exhaustive());
//! assert_eq!(report.missing()[0].display(&Lang).to_string(), "Some(false)");
//! assert_eq!(report.unreachable(), &[2]);
//! # Ok::<(), match_lang::Error>(())
//! ```
//!
//! ## What it handles
//!
//! - **Sums, products, booleans**, with fields to any depth. Sum variants
//!   whose fields include an empty type (see [`Model::is_empty`]) have no
//!   values: they are never required, and arms matching only them are
//!   unreachable.
//! - **Integers and characters**, with literals and inclusive ranges over the
//!   type's exact bounds, so `0..=127 | 128..=255` covers a `u8`. Unbounded
//!   integers ([`Signature::BigInt`]) and opaque literals such as strings
//!   ([`Signature::Opaque`]) always need a wildcard.
//! - **Arrays and slices**: fixed-length patterns `[a, b]` and rest patterns
//!   `[a, .., z]`, over fixed-length arrays and variable-length slices.
//! - **Or-patterns**, nested anywhere; each redundant alternative is
//!   reported on its own.
//! - **Guards**: a guarded arm may fail, so it never covers a value for
//!   exhaustiveness and never makes a later arm unreachable.
//!
//! ## How it works
//!
//! This is the usefulness algorithm of Luc Maranget's *Warnings for pattern
//! matching* (JFP 2007), in the form modern compilers use: one pass computes
//! the usefulness of every row of the pattern matrix, columns are split into
//! the constructors their patterns mention plus one constructor standing for
//! all the missing ones (integer and character ranges are cut into disjoint
//! segments; slice lengths are grouped into a finite set), and witnesses are
//! built on the way back. It runs on an explicit stack, not recursion, and
//! rows share their tails, so deep or wide matches cost no call stack and no
//! quadratic copying. Deciding exhaustiveness is NP-hard in general; every
//! unit of work counts against a step limit, and a match too complex to
//! check is an [`Error::StepLimit`], never a hang.
//!
//! ## Tiers
//!
//! - [`check`]: one call with default limits.
//! - [`Checker`]: the same, with configurable limits and buffers reused
//!   across checks.
//!
//! ## Features
//!
//! - `std` (default): nothing beyond `alloc` is used; without it the crate
//!   is `no_std`.
//!
//! The crate is `#![forbid(unsafe_code)]` and has no dependencies.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(
    warnings,
    missing_docs,
    unsafe_op_in_unsafe_fn,
    unused_must_use,
    unused_results,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::undocumented_unsafe_blocks
)]

extern crate alloc;

mod checker;
mod engine;
mod error;
mod model;
mod pat;
mod report;
mod witness;

pub use checker::{Checker, DEFAULT_STEP_LIMIT, DEFAULT_WITNESS_LIMIT};
pub use error::{Error, Reason};
pub use model::{Model, Signature};
pub use pat::{Arm, Pat};
pub use report::{Redundant, Report};
pub use witness::{Ctor, Witness, WitnessDisplay, WitnessFields, WitnessRef};

/// Checks `arms`, in order, against values of type `ty`: the one-call entry
/// point, with the default limits. For many matches, or other limits, use a
/// [`Checker`].
///
/// # Errors
///
/// - [`Error::InvalidPattern`] if a pattern does not fit the type it is
///   matched against (checked before any analysis).
/// - [`Error::StepLimit`] if the analysis needs more than
///   [`DEFAULT_STEP_LIMIT`] steps.
/// - [`Error::Model`] if the model answers inconsistently.
/// - [`Error::TooLarge`] past `u32::MAX - 1` pattern nodes or matrix entries.
///
/// # Examples
///
/// ```
/// use match_lang::{check, Arm, Model, Pat, Signature};
///
/// struct Bytes;
///
/// impl Model for Bytes {
///     type Ty = ();
///     fn signature(&self, _: &()) -> Signature<()> {
///         Signature::Int { min: 0, max: 255 }
///     }
///     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
/// }
///
/// let arms = [
///     Arm::new(Pat::range(0, 127)),
///     Arm::new(Pat::range(128, 255)),
///     Arm::new(Pat::int(7)),
/// ];
/// let report = check(&Bytes, &(), &arms)?;
/// assert!(report.is_exhaustive());
/// assert_eq!(report.unreachable(), &[2]);
/// # Ok::<(), match_lang::Error>(())
/// ```
pub fn check<M: Model>(model: &M, ty: &M::Ty, arms: &[Arm]) -> Result<Report<M::Ty>, Error> {
    Checker::new().check(model, ty, arms)
}

/// Compiles and runs the `rust` code blocks in `README.md` and `docs/API.md` as
/// part of `cargo test`, so the published examples cannot drift from the API.
///
/// Present only while collecting doctests (`#[cfg(doctest)]`); it is not part of
/// the public surface and does not appear in the built library or its docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
#[doc = include_str!("../docs/API.md")]
pub struct MarkdownDocTests;
