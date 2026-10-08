//! The constructor model: how a language tells the checker what its types'
//! values look like.

use alloc::vec::Vec;

/// A language's constructor model.
///
/// The checker never sees the language's types directly. It asks the model
/// two questions about a type: what its [`Signature`] is (which constructors
/// build its values), and what the field types of a constructor are. That is
/// enough to decide exhaustiveness and usefulness for every type the
/// signatures can describe: sums, products, booleans, bounded and unbounded
/// integers, characters, opaque literals, arrays, and slices.
///
/// The model must answer consistently: the same type must get the same
/// signature and fields every time during one check. A model that changes
/// its answers mid-check is reported as [`Error::Model`](crate::Error::Model),
/// never a panic.
///
/// `Ty` is whatever names a type in the language: an interned id is ideal,
/// since the checker clones types as it descends into fields.
///
/// # Examples
///
/// A model for `bool` and `Option<T>`:
///
/// ```
/// use match_lang::{Model, Signature};
///
/// #[derive(Clone)]
/// enum Ty {
///     Bool,
///     Option(Box<Ty>),
/// }
///
/// struct Lang;
///
/// impl Model for Lang {
///     type Ty = Ty;
///
///     fn signature(&self, ty: &Ty) -> Signature<Ty> {
///         match ty {
///             Ty::Bool => Signature::Bool,
///             Ty::Option(_) => Signature::Sum { count: 2 },
///         }
///     }
///
///     fn fields(&self, ty: &Ty, variant: u32, out: &mut Vec<Ty>) {
///         // `None` (variant 0) has no fields; `Some` (variant 1) has one.
///         if let (Ty::Option(inner), 1) = (ty, variant) {
///             out.push((**inner).clone());
///         }
///     }
///
///     fn variant_name(&self, ty: &Ty, variant: u32) -> Option<&str> {
///         match (ty, variant) {
///             (Ty::Option(_), 0) => Some("None"),
///             (Ty::Option(_), 1) => Some("Some"),
///             _ => None,
///         }
///     }
/// }
///
/// let mut fields = Vec::new();
/// Lang.fields(&Ty::Option(Box::new(Ty::Bool)), 1, &mut fields);
/// assert!(matches!(fields[..], [Ty::Bool]));
/// ```
pub trait Model {
    /// A type of the language.
    type Ty: Clone;

    /// The constructors that build values of `ty`.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Model, Signature};
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
    /// assert_eq!(Bytes.signature(&()), Signature::Int { min: 0, max: 255 });
    /// ```
    fn signature(&self, ty: &Self::Ty) -> Signature<Self::Ty>;

    /// Pushes onto `out` the field types of a constructor of `ty`, in field
    /// order: variant `variant` of a [`Sum`](Signature::Sum), or the single
    /// constructor of a [`Product`](Signature::Product) (`variant` is then
    /// 0). `out` is empty when called. It is not called for other
    /// signatures: booleans, integers, characters, and opaque literals have
    /// no fields, and arrays and slices name their element type in the
    /// signature.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Model, Signature};
    ///
    /// // A model of `(bool, bool)`: the unit type `()` stands for `bool`
    /// // inside, and `None` for the pair.
    /// struct Pair;
    ///
    /// impl Model for Pair {
    ///     type Ty = Option<()>;
    ///     fn signature(&self, ty: &Option<()>) -> Signature<Option<()>> {
    ///         match ty {
    ///             None => Signature::Product,
    ///             Some(()) => Signature::Bool,
    ///         }
    ///     }
    ///     fn fields(&self, _: &Option<()>, _: u32, out: &mut Vec<Option<()>>) {
    ///         out.extend([Some(()), Some(())]);
    ///     }
    /// }
    ///
    /// let mut out = Vec::new();
    /// Pair.fields(&None, 0, &mut out);
    /// assert_eq!(out.len(), 2);
    /// ```
    fn fields(&self, ty: &Self::Ty, variant: u32, out: &mut Vec<Self::Ty>);

    /// Whether `ty` has no values at all, like `!` or an enum with no
    /// variants.
    ///
    /// The checker treats a constructor as impossible when one of its fields
    /// is empty, so `Err(_)` is not required, and is reported unreachable,
    /// for a `Result<T, !>`. For that to be exact the answer should be deep:
    /// a tuple or struct with an empty field is itself empty. The default
    /// recognizes only `Sum { count: 0 }`.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Model, Signature};
    ///
    /// struct Never;
    ///
    /// impl Model for Never {
    ///     type Ty = ();
    ///     fn signature(&self, _: &()) -> Signature<()> {
    ///         Signature::Sum { count: 0 }
    ///     }
    ///     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// }
    ///
    /// assert!(Never.is_empty(&()));
    /// ```
    fn is_empty(&self, ty: &Self::Ty) -> bool {
        matches!(self.signature(ty), Signature::Sum { count: 0 })
    }

    /// A name for variant `variant` of `ty` (or, with `variant` 0, for a
    /// product type), used only when printing witnesses with
    /// [`Witness::display`](crate::Witness::display). Without a name a
    /// variant prints as `#index` and a product as a tuple.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Model, Signature};
    ///
    /// struct Unit;
    ///
    /// impl Model for Unit {
    ///     type Ty = ();
    ///     fn signature(&self, _: &()) -> Signature<()> {
    ///         Signature::Product
    ///     }
    ///     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// }
    ///
    /// assert_eq!(Unit.variant_name(&(), 0), None);
    /// ```
    fn variant_name(&self, ty: &Self::Ty, variant: u32) -> Option<&str> {
        let _unnamed = (ty, variant);
        None
    }
}

/// The constructors that build the values of one type.
///
/// Each signature fixes which [`Pat`](crate::Pat) constructors fit the type
/// and how they partition its values. Finite signatures can be covered
/// constructor by constructor; infinite ones need a wildcard (or, for
/// integers and characters, ranges that cover every value).
///
/// Types the signatures do not name directly map onto them:
///
/// - `()`, tuples, structs, records, `Box<T>` and `&T`: [`Product`](Self::Product).
/// - enums and other tagged unions, including the never type
///   (`Sum { count: 0 }`): [`Sum`](Self::Sum).
/// - fixed-width integers: [`Int`](Self::Int) with the type's bounds. A
///   `u128` maps into `i128` by flipping the top bit (`(v ^ (1 << 127)) as
///   i128`), which preserves order.
/// - floats: [`Opaque`](Self::Opaque) when compared by equality, or
///   [`Int`](Self::Int) over a total-order key of the bits when ranges are
///   allowed.
/// - strings and other interned values: [`Opaque`](Self::Opaque).
///
/// # Examples
///
/// ```
/// use match_lang::Signature;
///
/// let u8_sig: Signature<()> = Signature::Int { min: 0, max: 255 };
/// let option_sig: Signature<()> = Signature::Sum { count: 2 };
/// assert_ne!(u8_sig, option_sig);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Signature<T> {
    /// A tagged union with variants `0..count`, matched by
    /// [`Pat::variant`](crate::Pat::variant). Each variant's fields come from
    /// [`Model::fields`]. `count` 0 is an empty type: no value, so a match
    /// on it needs no arms.
    Sum {
        /// The number of variants.
        count: u32,
    },
    /// A type with exactly one constructor (a tuple, struct, record, box, or
    /// reference), matched by [`Pat::product`](crate::Pat::product). Its
    /// fields come from [`Model::fields`] with variant 0.
    Product,
    /// `false` and `true`, matched by [`Pat::bool`](crate::Pat::bool).
    Bool,
    /// The integers `min..=max`, matched by [`Pat::int`](crate::Pat::int)
    /// and [`Pat::range`](crate::Pat::range). Covering every value from
    /// `min` to `max` makes a match exhaustive.
    Int {
        /// The smallest value of the type.
        min: i128,
        /// The largest value of the type.
        max: i128,
    },
    /// The Unicode scalar values (`'\0'..='\u{10FFFF}'` without the
    /// surrogates), matched by [`Pat::char`](crate::Pat::char) and
    /// [`Pat::char_range`](crate::Pat::char_range).
    Char,
    /// Integers without bounds (arbitrary precision). Patterns are `i128`
    /// literals and ranges; values outside `i128` can only be matched by a
    /// wildcard, so a match is never exhaustive without one.
    BigInt,
    /// Infinitely many values told apart only by equality: strings, floats
    /// compared by value, symbols. Matched by [`Pat::lit`](crate::Pat::lit);
    /// a match always needs a wildcard.
    Opaque,
    /// Arrays of exactly `len` elements of type `elem`, matched by
    /// [`Pat::slice`](crate::Pat::slice) (with exactly `len` elements) and
    /// [`Pat::slice_rest`](crate::Pat::slice_rest).
    Array {
        /// The array length.
        len: u32,
        /// The element type.
        elem: T,
    },
    /// Sequences of any length of elements of type `elem`, matched by
    /// [`Pat::slice`](crate::Pat::slice) and
    /// [`Pat::slice_rest`](crate::Pat::slice_rest).
    Slice {
        /// The element type.
        elem: T,
    },
}
