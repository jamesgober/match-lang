//! The result of checking a match.

use alloc::vec::Vec;

use crate::Witness;

/// What checking a match found: missing cases, unreachable arms, and
/// redundant or-pattern alternatives.
///
/// # Examples
///
/// ```
/// use match_lang::{check, Arm, Model, Pat, Signature};
///
/// struct Flag;
///
/// impl Model for Flag {
///     type Ty = ();
///     fn signature(&self, _: &()) -> Signature<()> {
///         Signature::Bool
///     }
///     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
/// }
///
/// let arms = [
///     Arm::new(Pat::bool(true)),
///     Arm::new(Pat::or([Pat::bool(false), Pat::bool(true)])),
///     Arm::new(Pat::wild()),
/// ];
/// let report = check(&Flag, &(), &arms)?;
/// assert!(report.is_exhaustive());
/// assert_eq!(report.unreachable(), &[2]);
/// assert_eq!(report.redundant()[0].arm, 1);
/// assert_eq!(report.redundant()[0].node, 2); // the second alternative, `true`
/// # Ok::<(), match_lang::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Report<T> {
    pub(crate) missing: Vec<Witness<T>>,
    pub(crate) unreachable: Vec<usize>,
    pub(crate) redundant: Vec<Redundant>,
    pub(crate) steps: u64,
}

impl<T> Report<T> {
    /// Whether every value of the scrutinee's type is matched by some
    /// unguarded arm.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let guarded = [Arm::guarded(Pat::wild())];
    /// assert!(!check(&Flag, &(), &guarded)?.is_exhaustive());
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn is_exhaustive(&self) -> bool {
        self.missing.is_empty()
    }

    /// Witnesses of values no unguarded arm matches; empty exactly when the
    /// match is exhaustive. There is at least one when it is not, and at most
    /// the [witness limit](crate::Checker::with_witness_limit). They are a
    /// sample chosen to be informative, not an enumeration of every missing
    /// value.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let report = check(&Flag, &(), &[])?;
    /// let missing: Vec<String> = report.missing().iter().map(|w| w.to_string()).collect();
    /// assert_eq!(missing, ["false", "true"]);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn missing(&self) -> &[Witness<T>] {
        &self.missing
    }

    /// The indices of arms no value can reach, in ascending order. An arm is
    /// unreachable when every value its pattern matches is matched by an
    /// earlier unguarded arm, or when its pattern matches no value at all.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let arms = [Arm::new(Pat::wild()), Arm::new(Pat::bool(true))];
    /// assert_eq!(check(&Flag, &(), &arms)?.unreachable(), &[1]);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn unreachable(&self) -> &[usize] {
        &self.unreachable
    }

    /// Or-pattern alternatives that no value reaches, inside arms that are
    /// themselves reachable, in arm order and then preorder. Only the
    /// outermost redundant alternative is reported: alternatives nested
    /// inside it are not listed again.
    ///
    /// An alternative is redundant when every value it matches is matched by
    /// an earlier unguarded arm or, in an unguarded arm, by an earlier
    /// alternative of the same arm.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Redundant, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// // `true | false | true`: node 0 is the or-pattern, 1..=3 its alternatives.
    /// let arms = [Arm::new(Pat::or([Pat::bool(true), Pat::bool(false), Pat::bool(true)]))];
    /// let report = check(&Flag, &(), &arms)?;
    /// assert_eq!(report.redundant(), &[Redundant { arm: 0, node: 3 }]);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn redundant(&self) -> &[Redundant] {
        &self.redundant
    }

    /// The number of steps the check took, against the
    /// [step limit](crate::Checker::with_step_limit). Useful to see how close
    /// real matches come to the limit.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let report = check(&Flag, &(), &[Arm::new(Pat::wild())])?;
    /// assert!(report.steps() > 0);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn steps(&self) -> u64 {
        self.steps
    }
}

/// A redundant or-pattern alternative: in arm `arm`, the alternative whose
/// root is node `node` of the arm's pattern, by
/// [preorder index](crate::Pat#node-numbering).
///
/// # Examples
///
/// ```
/// use match_lang::Redundant;
///
/// let r = Redundant { arm: 2, node: 5 };
/// assert_eq!((r.arm, r.node), (2, 5));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Redundant {
    /// The index of the arm.
    pub arm: usize,
    /// The preorder index of the alternative within the arm's pattern.
    pub node: usize,
}
