//! Witnesses: patterns describing values that no arm matches.

use alloc::vec::Vec;
use core::fmt;

use crate::Model;

/// The constructor at one node of a [`Witness`].
///
/// # Examples
///
/// ```
/// use match_lang::Ctor;
///
/// let gap = Ctor::Int { lo: 10, hi: 255 };
/// assert_ne!(gap, Ctor::Wild);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Ctor {
    /// `_`: any value of the type.
    Wild,
    /// A value no pattern can name: an opaque literal that no arm mentions,
    /// or a [`BigInt`](crate::Signature::BigInt) outside the `i128` range.
    /// Printed as `_`.
    Other,
    /// Variant `index` of a sum type.
    Variant(u32),
    /// The constructor of a product type.
    Product,
    /// A boolean.
    Bool(bool),
    /// Any integer in `lo..=hi`.
    Int {
        /// The low end, inclusive.
        lo: i128,
        /// The high end, inclusive.
        hi: i128,
    },
    /// Any character in `lo..=hi`.
    Char {
        /// The low end, inclusive.
        lo: char,
        /// The high end, inclusive.
        hi: char,
    },
    /// An opaque literal, by id.
    Lit(u64),
    /// A sequence of exactly `len` elements; the fields are the elements.
    Slice {
        /// The number of elements.
        len: u32,
    },
    /// A sequence of at least `prefix + suffix` elements; the fields are the
    /// first `prefix` and the last `suffix` elements.
    SliceRest {
        /// The number of leading elements.
        prefix: u32,
        /// The number of trailing elements.
        suffix: u32,
    },
}

/// One node of a witness. Stored with the root last and each node's fields
/// immediately before it, first field nearest: that is the order the
/// algorithm produces them in, so building a witness never moves a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WNode<T> {
    pub(crate) ctor: Ctor,
    pub(crate) ty: T,
    pub(crate) arity: u32,
    /// Nodes in this subtree, including this one.
    pub(crate) size: u32,
}

/// A pattern describing values that no arm matches: proof that a match is
/// not exhaustive.
///
/// Every value matching the witness is matched by no unguarded arm (a
/// [`Wild`](Ctor::Wild) node stands for any value of its type, an
/// [`Other`](Ctor::Other) node for any value no pattern names). Every
/// witness matches at least one value.
///
/// A witness is a tree of [`WitnessRef`] nodes, each with a [`Ctor`], the
/// type of the value at that position, and the fields beneath it. It prints
/// in a neutral syntax with `Display`, or with the language's variant names
/// through [`display`](Witness::display).
///
/// # Examples
///
/// ```
/// use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
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
/// let arms = [Arm::new(Pat::range(0, 9)), Arm::new(Pat::range(20, 255))];
/// let report = check(&Bytes, &(), &arms)?;
/// let witness = &report.missing()[0];
/// assert_eq!(witness.ctor(), Ctor::Int { lo: 10, hi: 19 });
/// assert_eq!(witness.to_string(), "10..=19");
/// # Ok::<(), match_lang::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Witness<T> {
    nodes: Vec<WNode<T>>,
}

impl<T> Witness<T> {
    /// Wraps a node sequence holding exactly one subtree.
    pub(crate) fn from_nodes(nodes: Vec<WNode<T>>) -> Witness<T> {
        Witness { nodes }
    }

    /// The root node.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let report = check(&Flag, &(), &[Arm::new(Pat::bool(true))])?;
    /// assert_eq!(report.missing()[0].root().ctor(), Ctor::Bool(false));
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn root(&self) -> WitnessRef<'_, T> {
        WitnessRef {
            nodes: &self.nodes,
            pos: self.nodes.len().saturating_sub(1),
        }
    }

    /// The root's constructor; shorthand for `root().ctor()`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let report = check(&Flag, &(), &[Arm::new(Pat::bool(false))])?;
    /// assert_eq!(report.missing()[0].ctor(), Ctor::Bool(true));
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn ctor(&self) -> Ctor {
        self.root().ctor()
    }

    /// The type of the whole witness: the scrutinee's type.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = &'static str;
    /// #     fn signature(&self, _: &&'static str) -> Signature<&'static str> { Signature::Bool }
    /// #     fn fields(&self, _: &&'static str, _: u32, _: &mut Vec<&'static str>) {}
    /// # }
    /// let report = check(&Flag, &"bool", &[])?;
    /// assert_eq!(*report.missing()[0].ty(), "bool");
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn ty(&self) -> &T {
        self.root().ty()
    }

    /// The root's fields; shorthand for `root().fields()`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
    /// # struct Pair;
    /// # impl Model for Pair {
    /// #     type Ty = bool; // true: the pair, false: a bool inside it
    /// #     fn signature(&self, ty: &bool) -> Signature<bool> {
    /// #         if *ty { Signature::Product } else { Signature::Bool }
    /// #     }
    /// #     fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) { out.extend([false, false]) }
    /// # }
    /// let arms = [Arm::new(Pat::product([Pat::bool(true), Pat::wild()]))];
    /// let report = check(&Pair, &true, &arms)?;
    /// let fields: Vec<Ctor> = report.missing()[0].fields().map(|f| f.ctor()).collect();
    /// assert_eq!(fields, [Ctor::Bool(false), Ctor::Wild]);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    pub fn fields(&self) -> WitnessFields<'_, T> {
        self.root().fields()
    }

    /// Prints the witness with the model's variant names (see
    /// [`Model::variant_name`]).
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{check, Arm, Model, Pat, Signature};
    ///
    /// struct Opt;
    ///
    /// impl Model for Opt {
    ///     type Ty = u8; // 0: Option<bool>, 1: bool
    ///     fn signature(&self, ty: &u8) -> Signature<u8> {
    ///         if *ty == 0 { Signature::Sum { count: 2 } } else { Signature::Bool }
    ///     }
    ///     fn fields(&self, _: &u8, variant: u32, out: &mut Vec<u8>) {
    ///         if variant == 1 {
    ///             out.push(1);
    ///         }
    ///     }
    ///     fn variant_name(&self, _: &u8, variant: u32) -> Option<&str> {
    ///         Some(if variant == 0 { "None" } else { "Some" })
    ///     }
    /// }
    ///
    /// let arms = [Arm::new(Pat::variant(0, [])), Arm::new(Pat::variant(1, [Pat::bool(true)]))];
    /// let report = check(&Opt, &0, &arms)?;
    /// assert_eq!(report.missing()[0].to_string(), "#1(false)");
    /// assert_eq!(report.missing()[0].display(&Opt).to_string(), "Some(false)");
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    pub fn display<'a, M: Model<Ty = T>>(&'a self, model: &'a M) -> WitnessDisplay<'a, M> {
        WitnessDisplay {
            witness: self.root(),
            model,
        }
    }
}

impl<T> fmt::Display for Witness<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.root().fmt(f)
    }
}

impl<T> fmt::Debug for Witness<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Witness({})", self.root())
    }
}

/// A node of a [`Witness`]: its constructor, its type, and its fields.
///
/// # Examples
///
/// ```
/// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
/// # struct Pair;
/// # impl Model for Pair {
/// #     type Ty = bool;
/// #     fn signature(&self, ty: &bool) -> Signature<bool> {
/// #         if *ty { Signature::Product } else { Signature::Bool }
/// #     }
/// #     fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) { out.extend([false, false]) }
/// # }
/// let arms = [Arm::new(Pat::product([Pat::wild(), Pat::bool(true)]))];
/// let report = check(&Pair, &true, &arms)?;
/// let root = report.missing()[0].root();
/// assert_eq!(root.ctor(), Ctor::Product);
/// assert_eq!(root.arity(), 2);
/// let second = root.fields().nth(1).unwrap();
/// assert_eq!((second.ctor(), *second.ty()), (Ctor::Bool(false), false));
/// # Ok::<(), match_lang::Error>(())
/// ```
pub struct WitnessRef<'a, T> {
    nodes: &'a [WNode<T>],
    pos: usize,
}

impl<T> Clone for WitnessRef<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for WitnessRef<'_, T> {}

impl<'a, T> WitnessRef<'a, T> {
    fn node(&self) -> Option<&'a WNode<T>> {
        self.nodes.get(self.pos)
    }

    /// The constructor at this node.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = ();
    /// #     fn signature(&self, _: &()) -> Signature<()> { Signature::Bool }
    /// #     fn fields(&self, _: &(), _: u32, _: &mut Vec<()>) {}
    /// # }
    /// let report = check(&Flag, &(), &[Arm::new(Pat::bool(true))])?;
    /// assert_eq!(report.missing()[0].root().ctor(), Ctor::Bool(false));
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn ctor(&self) -> Ctor {
        self.node().map_or(Ctor::Wild, |n| n.ctor)
    }

    /// The type of the value at this node.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Flag;
    /// # impl Model for Flag {
    /// #     type Ty = u8;
    /// #     fn signature(&self, _: &u8) -> Signature<u8> { Signature::Bool }
    /// #     fn fields(&self, _: &u8, _: u32, _: &mut Vec<u8>) {}
    /// # }
    /// let report = check(&Flag, &7, &[])?;
    /// assert_eq!(*report.missing()[0].root().ty(), 7);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn ty(&self) -> &'a T {
        // A `WitnessRef` is only made for positions inside its witness, and a
        // witness always has a root, so the fallback is never taken.
        match self.nodes.get(self.pos) {
            Some(node) => &node.ty,
            None => &self.nodes[0].ty,
        }
    }

    /// The number of fields under this node.
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
    /// assert_eq!(report.missing()[0].root().arity(), 0);
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn arity(&self) -> usize {
        self.node().map_or(0, |n| n.arity as usize)
    }

    /// The fields under this node, in order.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Ctor, Model, Pat, Signature};
    /// # struct Pair;
    /// # impl Model for Pair {
    /// #     type Ty = bool;
    /// #     fn signature(&self, ty: &bool) -> Signature<bool> {
    /// #         if *ty { Signature::Product } else { Signature::Bool }
    /// #     }
    /// #     fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) { out.extend([false, false]) }
    /// # }
    /// let arms = [Arm::new(Pat::product([Pat::bool(false), Pat::bool(false)]))];
    /// let report = check(&Pair, &true, &arms)?;
    /// let first = report.missing()[0].root().fields().next().unwrap();
    /// assert_eq!(first.ctor(), Ctor::Bool(true));
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    #[must_use]
    pub fn fields(&self) -> WitnessFields<'a, T> {
        WitnessFields {
            nodes: self.nodes,
            next: self.pos.checked_sub(1),
            remaining: self.arity(),
        }
    }

    /// Prints this node with the model's variant names.
    ///
    /// # Examples
    ///
    /// ```
    /// # use match_lang::{check, Arm, Model, Pat, Signature};
    /// # struct Opt;
    /// # impl Model for Opt {
    /// #     type Ty = u8;
    /// #     fn signature(&self, ty: &u8) -> Signature<u8> {
    /// #         if *ty == 0 { Signature::Sum { count: 2 } } else { Signature::Bool }
    /// #     }
    /// #     fn fields(&self, _: &u8, v: u32, out: &mut Vec<u8>) { if v == 1 { out.push(1) } }
    /// #     fn variant_name(&self, _: &u8, v: u32) -> Option<&str> {
    /// #         Some(if v == 0 { "None" } else { "Some" })
    /// #     }
    /// # }
    /// let report = check(&Opt, &0, &[Arm::new(Pat::variant(1, [Pat::wild()]))])?;
    /// assert_eq!(report.missing()[0].root().display(&Opt).to_string(), "None");
    /// # Ok::<(), match_lang::Error>(())
    /// ```
    pub fn display<M: Model<Ty = T>>(self, model: &'a M) -> WitnessDisplay<'a, M> {
        WitnessDisplay {
            witness: self,
            model,
        }
    }

    /// Writes the subtree, asking `name` for variant and product names.
    fn write_with<'n>(
        &self,
        f: &mut fmt::Formatter<'_>,
        name: impl Fn(&T, u32) -> Option<&'n str>,
    ) -> fmt::Result {
        enum Step<'a, T> {
            Node(WitnessRef<'a, T>),
            Text(&'static str),
        }
        let mut stack = Vec::new();
        stack.push(Step::Node(*self));
        let mut fields = Vec::new();
        while let Some(step) = stack.pop() {
            let node = match step {
                Step::Text(text) => {
                    f.write_str(text)?;
                    continue;
                }
                Step::Node(node) => node,
            };
            let (open, close) = match node.ctor() {
                Ctor::Wild | Ctor::Other => {
                    f.write_str("_")?;
                    continue;
                }
                Ctor::Bool(b) => {
                    write!(f, "{b}")?;
                    continue;
                }
                Ctor::Int { lo, hi } if lo == hi => {
                    write!(f, "{lo}")?;
                    continue;
                }
                Ctor::Int { lo, hi } => {
                    write!(f, "{lo}..={hi}")?;
                    continue;
                }
                Ctor::Char { lo, hi } if lo == hi => {
                    write!(f, "{lo:?}")?;
                    continue;
                }
                Ctor::Char { lo, hi } => {
                    write!(f, "{lo:?}..={hi:?}")?;
                    continue;
                }
                Ctor::Lit(id) => {
                    write!(f, "lit#{id}")?;
                    continue;
                }
                Ctor::Variant(index) => {
                    match name(node.ty(), index) {
                        Some(n) => f.write_str(n)?,
                        None => write!(f, "#{index}")?,
                    }
                    if node.arity() == 0 {
                        continue;
                    }
                    ("(", ")")
                }
                Ctor::Product => match name(node.ty(), 0) {
                    Some(n) => {
                        f.write_str(n)?;
                        if node.arity() == 0 {
                            continue;
                        }
                        ("(", ")")
                    }
                    None if node.arity() == 1 => ("(", ",)"),
                    None => ("(", ")"),
                },
                Ctor::Slice { .. } => ("[", "]"),
                Ctor::SliceRest { prefix, .. } => {
                    f.write_str("[")?;
                    stack.push(Step::Text("]"));
                    fields.clear();
                    fields.extend(node.fields());
                    let split = (prefix as usize).min(fields.len());
                    let (pre, post) = fields.split_at(split);
                    for (i, field) in post.iter().enumerate().rev() {
                        stack.push(Step::Node(*field));
                        if i > 0 {
                            stack.push(Step::Text(", "));
                        }
                    }
                    stack.push(Step::Text(match (pre.is_empty(), post.is_empty()) {
                        (true, true) => "..",
                        (true, false) => ".., ",
                        (false, true) => ", ..",
                        (false, false) => ", .., ",
                    }));
                    for (i, field) in pre.iter().enumerate().rev() {
                        stack.push(Step::Node(*field));
                        if i > 0 {
                            stack.push(Step::Text(", "));
                        }
                    }
                    continue;
                }
            };
            f.write_str(open)?;
            stack.push(Step::Text(close));
            fields.clear();
            fields.extend(node.fields());
            for (i, field) in fields.iter().enumerate().rev() {
                stack.push(Step::Node(*field));
                if i > 0 {
                    stack.push(Step::Text(", "));
                }
            }
        }
        Ok(())
    }
}

impl<T> fmt::Display for WitnessRef<'_, T> {
    /// Prints the subtree in a neutral syntax: `#i(..)` for variants, tuples
    /// for products, `_` for wildcards and unnamed values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_with(f, |_, _| None)
    }
}

impl<T> fmt::Debug for WitnessRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WitnessRef({self})")
    }
}

/// The fields of a [`WitnessRef`], in order. Returned by
/// [`WitnessRef::fields`].
///
/// # Examples
///
/// ```
/// # use match_lang::{check, Arm, Model, Pat, Signature};
/// # struct Pair;
/// # impl Model for Pair {
/// #     type Ty = bool;
/// #     fn signature(&self, ty: &bool) -> Signature<bool> {
/// #         if *ty { Signature::Product } else { Signature::Bool }
/// #     }
/// #     fn fields(&self, _: &bool, _: u32, out: &mut Vec<bool>) { out.extend([false, false]) }
/// # }
/// let report = check(&Pair, &true, &[])?;
/// assert_eq!(report.missing()[0].fields().len(), 2);
/// # Ok::<(), match_lang::Error>(())
/// ```
pub struct WitnessFields<'a, T> {
    nodes: &'a [WNode<T>],
    /// Where the next field's subtree ends (its root).
    next: Option<usize>,
    remaining: usize,
}

impl<'a, T> Iterator for WitnessFields<'a, T> {
    type Item = WitnessRef<'a, T>;

    fn next(&mut self) -> Option<WitnessRef<'a, T>> {
        if self.remaining == 0 {
            return None;
        }
        let pos = self.next?;
        let node = self.nodes.get(pos)?;
        self.remaining -= 1;
        // The next field's subtree ends just before this one starts.
        self.next = pos.checked_sub(node.size as usize);
        Some(WitnessRef {
            nodes: self.nodes,
            pos,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for WitnessFields<'_, T> {}

impl<T> fmt::Debug for WitnessFields<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WitnessFields")
            .field("remaining", &self.remaining)
            .finish()
    }
}

/// A witness printed with the model's variant names. Returned by
/// [`Witness::display`] and [`WitnessRef::display`].
///
/// # Examples
///
/// ```
/// # use match_lang::{check, Arm, Model, Pat, Signature};
/// # struct Opt;
/// # impl Model for Opt {
/// #     type Ty = u8;
/// #     fn signature(&self, ty: &u8) -> Signature<u8> {
/// #         if *ty == 0 { Signature::Sum { count: 2 } } else { Signature::Bool }
/// #     }
/// #     fn fields(&self, _: &u8, v: u32, out: &mut Vec<u8>) { if v == 1 { out.push(1) } }
/// #     fn variant_name(&self, _: &u8, v: u32) -> Option<&str> {
/// #         Some(if v == 0 { "None" } else { "Some" })
/// #     }
/// # }
/// let report = check(&Opt, &0, &[Arm::new(Pat::variant(0, []))])?;
/// assert_eq!(format!("{}", report.missing()[0].display(&Opt)), "Some(_)");
/// # Ok::<(), match_lang::Error>(())
/// ```
pub struct WitnessDisplay<'a, M: Model> {
    witness: WitnessRef<'a, M::Ty>,
    model: &'a M,
}

impl<M: Model> fmt::Display for WitnessDisplay<'_, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.witness
            .write_with(f, |ty, variant| self.model.variant_name(ty, variant))
    }
}

impl<M: Model> fmt::Debug for WitnessDisplay<'_, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
