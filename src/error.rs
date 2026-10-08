//! Why a check could not be completed.

use core::fmt;

/// Why a check could not produce a [`Report`](crate::Report).
///
/// # Examples
///
/// ```
/// use match_lang::{check, Arm, Error, Model, Pat, Reason, Signature};
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
/// let err = check(&Bytes, &(), &[Arm::new(Pat::int(300))]).unwrap_err();
/// assert_eq!(err, Error::InvalidPattern { arm: 0, node: 0, reason: Reason::OutOfDomain });
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The analysis took more steps than the
    /// [step limit](crate::Checker::with_step_limit) allows. Deciding
    /// exhaustiveness is NP-hard in general, so some matrices (typically
    /// many columns of overlapping or-patterns or ranges) need exponential
    /// work; the limit turns them into this error instead of a hang. Raise
    /// the limit if the match is legitimate, or report it to the user as too
    /// complex to check.
    StepLimit {
        /// The limit that was exceeded.
        limit: u64,
    },
    /// A pattern does not fit the type it is matched against. This is a type
    /// error the language's checker should have caught; nothing was analysed.
    InvalidPattern {
        /// The index of the arm.
        arm: usize,
        /// The preorder index of the offending node within the arm's pattern
        /// (see [`Pat`](crate::Pat#node-numbering)).
        node: usize,
        /// What is wrong with it.
        reason: Reason,
    },
    /// The model answered inconsistently: the same type got different
    /// signatures or field counts during one check, or a signature was
    /// itself invalid (an `Int` with `min > max`). Fix the model.
    Model {
        /// What was inconsistent.
        reason: &'static str,
    },
    /// The patterns, or the matrix built from them, need more than
    /// `u32::MAX - 1` nodes, rows, or cells.
    TooLarge,
}

/// What is wrong with a pattern, in [`Error::InvalidPattern`].
///
/// # Examples
///
/// ```
/// use match_lang::Reason;
///
/// let reason = Reason::Fields { expected: 1, found: 2 };
/// assert_eq!(reason.to_string(), "expected 1 fields, found 2");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Reason {
    /// The kind of pattern does not fit the type's signature, such as an
    /// integer pattern on a sum type.
    Kind,
    /// A variant index at or past the sum type's variant count.
    Variant {
        /// The variant index used.
        index: u32,
        /// The number of variants the type has.
        count: u32,
    },
    /// A variant or product pattern with the wrong number of fields.
    Fields {
        /// The number of fields the model reports.
        expected: usize,
        /// The number of field patterns given.
        found: usize,
    },
    /// A range whose low end is above its high end.
    EmptyRange,
    /// An integer or range outside the bounds of an `Int` type.
    OutOfDomain,
    /// An array pattern that cannot have the array's length: a fixed slice
    /// pattern of another length, or a rest pattern needing more elements.
    Length {
        /// The array's length.
        expected: u32,
        /// The number of elements the pattern requires.
        found: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::StepLimit { limit } => {
                write!(
                    f,
                    "match too complex to check: exceeded the step limit of {limit}"
                )
            }
            Error::InvalidPattern { arm, node, reason } => {
                write!(f, "arm {arm}, pattern node {node}: {reason}")
            }
            Error::Model { reason } => write!(f, "inconsistent constructor model: {reason}"),
            Error::TooLarge => f.write_str("match too large: more than u32::MAX - 1 nodes"),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Kind => f.write_str("pattern does not fit the type"),
            Reason::Variant { index, count } => {
                write!(f, "variant {index} does not exist (the type has {count})")
            }
            Reason::Fields { expected, found } => {
                write!(f, "expected {expected} fields, found {found}")
            }
            Reason::EmptyRange => f.write_str("range is empty (low end above high end)"),
            Reason::OutOfDomain => f.write_str("value outside the type's bounds"),
            Reason::Length { expected, found } => {
                write!(
                    f,
                    "array of length {expected} cannot match a pattern of {found} elements"
                )
            }
        }
    }
}

impl core::error::Error for Error {}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    #[test]
    fn test_error_messages_name_the_problem() {
        assert!(
            Error::StepLimit { limit: 5 }
                .to_string()
                .contains("step limit of 5")
        );
        assert_eq!(
            Error::InvalidPattern {
                arm: 1,
                node: 2,
                reason: Reason::Kind
            }
            .to_string(),
            "arm 1, pattern node 2: pattern does not fit the type"
        );
        assert!(Error::Model { reason: "x" }.to_string().ends_with(": x"));
        assert!(Error::TooLarge.to_string().contains("too large"));
        assert!(
            Reason::Variant { index: 3, count: 2 }
                .to_string()
                .contains("variant 3")
        );
        assert!(Reason::EmptyRange.to_string().contains("empty"));
        assert!(Reason::OutOfDomain.to_string().contains("bounds"));
        assert!(
            Reason::Length {
                expected: 2,
                found: 3
            }
            .to_string()
            .contains("length 2")
        );
    }
}
