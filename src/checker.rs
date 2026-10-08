//! The configurable, reusable entry point.

use core::fmt;

use crate::{Arm, Error, Model, Report, engine::Engine};

/// The default [step limit](Checker::with_step_limit): about 16.7 million
/// units of work. Real matches, even with tens of thousands of arms, use a
/// small fraction of it; a pathological one stops in well under a second.
pub const DEFAULT_STEP_LIMIT: u64 = 1 << 24;

/// The default [witness limit](Checker::with_witness_limit).
pub const DEFAULT_WITNESS_LIMIT: usize = 16;

/// Checks matches for exhaustiveness and usefulness, reusing its buffers
/// from one check to the next.
///
/// [`check`](crate::check) is a `Checker` used once with default limits. Keep
/// a `Checker` around when checking many matches (a whole program's): after
/// the first few checks it allocates almost nothing. Configure it with:
///
/// - [`with_step_limit`](Checker::with_step_limit): how much work one check
///   may do before giving up with [`Error::StepLimit`].
/// - [`with_witness_limit`](Checker::with_witness_limit): how many missing
///   cases one report lists at most.
///
/// # Examples
///
/// ```
/// use match_lang::{Arm, Checker, Model, Pat, Signature};
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
/// let mut checker = Checker::new().with_witness_limit(2);
/// for n in 0..10 {
///     let arms: Vec<Arm> = (0..n).map(|i| Arm::new(Pat::int(i * 2))).collect();
///     let report = checker.check(&Bytes, &(), &arms)?;
///     assert!(!report.is_exhaustive());
///     assert!(report.missing().len() <= 2);
/// }
/// # Ok::<(), match_lang::Error>(())
/// ```
pub struct Checker<T> {
    engine: Engine<T>,
    step_limit: u64,
    witness_limit: usize,
}

impl<T: Clone> Checker<T> {
    /// A checker with the default limits ([`DEFAULT_STEP_LIMIT`] and
    /// [`DEFAULT_WITNESS_LIMIT`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Checker, DEFAULT_STEP_LIMIT};
    ///
    /// let checker: Checker<()> = Checker::new();
    /// assert_eq!(checker.step_limit(), DEFAULT_STEP_LIMIT);
    /// ```
    #[must_use]
    pub fn new() -> Checker<T> {
        Checker {
            engine: Engine::new(),
            step_limit: DEFAULT_STEP_LIMIT,
            witness_limit: DEFAULT_WITNESS_LIMIT,
        }
    }

    /// Sets how many units of work one check may do. A unit is roughly one
    /// matrix row, cell, or witness node built, or one constructor examined;
    /// [`Report::steps`] tells how many a check used. A check that would
    /// exceed the limit stops with [`Error::StepLimit`], so time and memory
    /// per check are bounded by the limit whatever the input.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Checker, Error, Model, Pat, Signature};
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
    /// let arms: Vec<Arm> = (0..100).map(|i| Arm::new(Pat::int(i))).collect();
    /// let mut tight = Checker::new().with_step_limit(10);
    /// assert_eq!(tight.check(&Bytes, &(), &arms).unwrap_err(), Error::StepLimit { limit: 10 });
    /// ```
    #[must_use]
    pub fn with_step_limit(mut self, limit: u64) -> Checker<T> {
        self.step_limit = limit;
        self
    }

    /// Sets the most missing cases one report lists. At least one is always
    /// kept, since the first witness is what proves a match non-exhaustive;
    /// `0` is treated as `1`.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Checker;
    ///
    /// let checker: Checker<()> = Checker::new().with_witness_limit(0);
    /// assert_eq!(checker.witness_limit(), 1);
    /// ```
    #[must_use]
    pub fn with_witness_limit(mut self, limit: usize) -> Checker<T> {
        self.witness_limit = limit.max(1);
        self
    }

    /// The step limit in effect.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Checker;
    ///
    /// assert_eq!(Checker::<()>::new().with_step_limit(99).step_limit(), 99);
    /// ```
    #[must_use]
    pub fn step_limit(&self) -> u64 {
        self.step_limit
    }

    /// The witness limit in effect.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Checker, DEFAULT_WITNESS_LIMIT};
    ///
    /// assert_eq!(Checker::<()>::new().witness_limit(), DEFAULT_WITNESS_LIMIT);
    /// ```
    #[must_use]
    pub fn witness_limit(&self) -> usize {
        self.witness_limit
    }

    /// Checks `arms`, in order, against values of type `ty`.
    ///
    /// # Errors
    ///
    /// - [`Error::InvalidPattern`] if a pattern does not fit the type it is
    ///   matched against (checked before any analysis).
    /// - [`Error::StepLimit`] if the analysis needs more steps than the limit.
    /// - [`Error::Model`] if the model answers inconsistently.
    /// - [`Error::TooLarge`] past `u32::MAX - 1` pattern nodes or matrix
    ///   entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Checker, Model, Pat, Signature};
    ///
    /// struct Flags;
    ///
    /// impl Model for Flags {
    ///     type Ty = u8; // 0: (bool, bool), 1: bool
    ///     fn signature(&self, ty: &u8) -> Signature<u8> {
    ///         if *ty == 0 { Signature::Product } else { Signature::Bool }
    ///     }
    ///     fn fields(&self, _: &u8, _: u32, out: &mut Vec<u8>) {
    ///         out.extend([1, 1]);
    ///     }
    /// }
    ///
    /// let arms = [
    ///     Arm::new(Pat::product([Pat::bool(true), Pat::wild()])),
    ///     Arm::new(Pat::product([Pat::wild(), Pat::bool(true)])),
    /// ];
    /// let report = Checker::new().check(&Flags, &0, &arms)?;
    /// assert_eq!(report.missing()[0].to_string(), "(false, false)");
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    pub fn check<M: Model<Ty = T>>(
        &mut self,
        model: &M,
        ty: &T,
        arms: &[Arm],
    ) -> Result<Report<T>, Error> {
        self.engine
            .check(model, ty, arms, self.step_limit, self.witness_limit)
    }
}

impl<T: Clone> Default for Checker<T> {
    fn default() -> Checker<T> {
        Checker::new()
    }
}

impl<T> fmt::Debug for Checker<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Checker")
            .field("step_limit", &self.step_limit)
            .field("witness_limit", &self.witness_limit)
            .finish_non_exhaustive()
    }
}
