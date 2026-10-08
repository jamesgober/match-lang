//! Patterns and match arms, as the caller builds them.
//!
//! A [`Pat`] is an owned tree. Everything that walks one (`Drop`, `Clone`,
//! `Display`, and the checker's flattening pass) does so with an explicit
//! stack, so a pattern nested a hundred thousand levels deep is as safe as a
//! shallow one.

use alloc::vec::Vec;
use core::fmt;

/// What a single pattern node tests. Private: callers build nodes through
/// [`Pat`]'s constructors, which keep each kind's children consistent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatKind {
    /// `_`: matches every value; no children.
    Wild,
    /// `p | q | ...`: children are the alternatives.
    Or,
    /// Variant `index` of a sum type; children are its fields.
    Variant(u32),
    /// The single constructor of a product type; children are its fields.
    Product,
    /// `true` or `false`.
    Bool(bool),
    /// An inclusive integer range; `lo == hi` for a single value.
    Int { lo: i128, hi: i128 },
    /// An inclusive range of Unicode scalar values.
    Char { lo: char, hi: char },
    /// An opaque literal, identified by the caller's id.
    Lit(u64),
    /// A slice or array pattern. The first `prefix` children come before the
    /// `..`, the rest after it; without `rest` there is no `..` and every
    /// child is in the prefix.
    Slice { prefix: u32, rest: bool },
}

/// A pattern: the left-hand side of one match arm, or a part of one.
///
/// Patterns are built bottom-up with the constructors below and describe
/// values of the type they are checked against, in terms of the
/// [`Signature`](crate::Signature) the [`Model`](crate::Model) gives that
/// type. Which constructor fits which signature:
///
/// | Signature | Patterns |
/// |---|---|
/// | `Sum` | [`variant`](Pat::variant) |
/// | `Product` | [`product`](Pat::product) |
/// | `Bool` | [`bool`](Pat::bool) |
/// | `Int`, `BigInt` | [`int`](Pat::int), [`range`](Pat::range) |
/// | `Char` | [`char`](Pat::char), [`char_range`](Pat::char_range) |
/// | `Opaque` | [`lit`](Pat::lit) |
/// | `Array`, `Slice` | [`slice`](Pat::slice), [`slice_rest`](Pat::slice_rest) |
/// | any | [`wild`](Pat::wild), [`or`](Pat::or) |
///
/// A binding such as `x` or `x @ p` is, for matching purposes, `_` or `p`.
///
/// # Node numbering
///
/// Diagnostics about part of a pattern (see
/// [`Redundant`](crate::Redundant)) name the part by its **preorder index**
/// in the arm's pattern: the root is node 0, and each node is followed by
/// its children (fields, alternatives, or slice elements, prefix before
/// suffix) left to right, each child's whole subtree before the next child.
///
/// # Examples
///
/// ```
/// use match_lang::Pat;
///
/// // `Some(1 | 2..=5)`, with `Some` as variant 1 of its sum type.
/// let pat = Pat::variant(1, [Pat::or([Pat::int(1), Pat::range(2, 5)])]);
/// assert_eq!(pat.to_string(), "#1(1 | 2..=5)");
/// ```
pub struct Pat {
    pub(crate) kind: PatKind,
    pub(crate) children: Vec<Pat>,
}

impl Pat {
    fn leaf(kind: PatKind) -> Pat {
        Pat {
            kind,
            children: Vec::new(),
        }
    }

    /// The wildcard `_`, which matches every value of any type. A binding
    /// pattern (`x`) is a wildcard to the checker.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::wild().to_string(), "_");
    /// ```
    #[must_use]
    pub fn wild() -> Pat {
        Pat::leaf(PatKind::Wild)
    }

    /// Variant `index` of a [`Sum`](crate::Signature::Sum) type, with one
    /// pattern per field of that variant, in field order.
    ///
    /// The number of fields must match what the model's
    /// [`fields`](crate::Model::fields) reports for the variant; a mismatch
    /// is reported as [`Reason::Fields`](crate::Reason::Fields).
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// let none = Pat::variant(0, []);
    /// let some = Pat::variant(1, [Pat::wild()]);
    /// assert_eq!(none.to_string(), "#0");
    /// assert_eq!(some.to_string(), "#1(_)");
    /// ```
    #[must_use]
    pub fn variant(index: u32, fields: impl IntoIterator<Item = Pat>) -> Pat {
        Pat {
            kind: PatKind::Variant(index),
            children: fields.into_iter().collect(),
        }
    }

    /// The single constructor of a [`Product`](crate::Signature::Product)
    /// type: a tuple, struct, record, box, or reference. One pattern per
    /// field, in the field order the model reports; a struct pattern with
    /// `..` is written with `_` for the omitted fields.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// let pair = Pat::product([Pat::bool(true), Pat::wild()]);
    /// assert_eq!(pair.to_string(), "(true, _)");
    /// ```
    #[must_use]
    pub fn product(fields: impl IntoIterator<Item = Pat>) -> Pat {
        Pat {
            kind: PatKind::Product,
            children: fields.into_iter().collect(),
        }
    }

    /// A boolean literal, for a [`Bool`](crate::Signature::Bool) type.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::bool(false).to_string(), "false");
    /// ```
    #[must_use]
    pub fn bool(value: bool) -> Pat {
        Pat::leaf(PatKind::Bool(value))
    }

    /// An integer literal, for an [`Int`](crate::Signature::Int) or
    /// [`BigInt`](crate::Signature::BigInt) type. Equivalent to
    /// `Pat::range(value, value)`.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::int(-7).to_string(), "-7");
    /// ```
    #[must_use]
    pub fn int(value: i128) -> Pat {
        Pat::range(value, value)
    }

    /// An inclusive integer range `lo..=hi`. An exclusive range `lo..hi` is
    /// `range(lo, hi - 1)`; a half-open range takes the type's bound for the
    /// open end. `lo > hi` is reported as
    /// [`Reason::EmptyRange`](crate::Reason::EmptyRange), and a range that
    /// leaves an `Int` type's bounds as
    /// [`Reason::OutOfDomain`](crate::Reason::OutOfDomain).
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::range(0, 9).to_string(), "0..=9");
    /// ```
    #[must_use]
    pub fn range(lo: i128, hi: i128) -> Pat {
        Pat::leaf(PatKind::Int { lo, hi })
    }

    /// A character literal, for a [`Char`](crate::Signature::Char) type.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::char('x').to_string(), "'x'");
    /// ```
    #[must_use]
    pub fn char(value: char) -> Pat {
        Pat::char_range(value, value)
    }

    /// An inclusive character range `lo..=hi`. It covers every Unicode scalar
    /// value between the two, so a range across the surrogate block simply
    /// skips it.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::char_range('a', 'z').to_string(), "'a'..='z'");
    /// ```
    #[must_use]
    pub fn char_range(lo: char, hi: char) -> Pat {
        Pat::leaf(PatKind::Char { lo, hi })
    }

    /// An opaque literal, for an [`Opaque`](crate::Signature::Opaque) type
    /// such as strings or floats compared by equality. Two literals are the
    /// same value exactly when their ids are equal, so the caller interns
    /// the literal (for example a string through `intern-lang`) and passes
    /// the id.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::lit(3).to_string(), "lit#3");
    /// ```
    #[must_use]
    pub fn lit(id: u64) -> Pat {
        Pat::leaf(PatKind::Lit(id))
    }

    /// An or-pattern: matches when any alternative matches, trying them left
    /// to right. Alternatives may themselves be or-patterns. An or-pattern
    /// with no alternatives matches nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// let p = Pat::or([Pat::int(1), Pat::int(2)]);
    /// assert_eq!(p.to_string(), "1 | 2");
    /// ```
    #[must_use]
    pub fn or(alternatives: impl IntoIterator<Item = Pat>) -> Pat {
        Pat {
            kind: PatKind::Or,
            children: alternatives.into_iter().collect(),
        }
    }

    /// A fixed-length slice or array pattern `[p1, ..., pn]`: matches a
    /// sequence of exactly `n` elements. Against an
    /// [`Array`](crate::Signature::Array) type, `n` must equal the array's
    /// length ([`Reason::Length`](crate::Reason::Length) otherwise).
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// assert_eq!(Pat::slice([Pat::int(1), Pat::wild()]).to_string(), "[1, _]");
    /// ```
    #[must_use]
    pub fn slice(elements: impl IntoIterator<Item = Pat>) -> Pat {
        let children: Vec<Pat> = elements.into_iter().collect();
        let prefix = u32::try_from(children.len()).unwrap_or(u32::MAX);
        Pat {
            kind: PatKind::Slice {
                prefix,
                rest: false,
            },
            children,
        }
    }

    /// A slice pattern with a rest, `[p1, ..., pk, .., q1, ..., qm]`:
    /// matches any sequence of at least `k + m` elements whose first `k` match
    /// `prefix` and whose last `m` match `suffix`.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::Pat;
    ///
    /// let first_and_last = Pat::slice_rest([Pat::int(0)], [Pat::int(9)]);
    /// assert_eq!(first_and_last.to_string(), "[0, .., 9]");
    /// assert_eq!(Pat::slice_rest([], []).to_string(), "[..]");
    /// ```
    #[must_use]
    pub fn slice_rest(
        prefix: impl IntoIterator<Item = Pat>,
        suffix: impl IntoIterator<Item = Pat>,
    ) -> Pat {
        let mut children: Vec<Pat> = prefix.into_iter().collect();
        let prefix = u32::try_from(children.len()).unwrap_or(u32::MAX);
        children.extend(suffix);
        Pat {
            kind: PatKind::Slice { prefix, rest: true },
            children,
        }
    }
}

impl Drop for Pat {
    /// Drops the tree with an explicit stack instead of recursion.
    fn drop(&mut self) {
        // Leaves and flat patterns, the overwhelmingly common case, need no
        // stack at all.
        if self.children.iter().all(|c| c.children.is_empty()) {
            return;
        }
        let mut stack = core::mem::take(&mut self.children);
        while let Some(mut pat) = stack.pop() {
            stack.append(&mut pat.children);
            // `pat` now has no children, so dropping it does not recurse.
        }
    }
}

impl Clone for Pat {
    /// Clones the tree with an explicit stack instead of recursion.
    fn clone(&self) -> Pat {
        // `work` holds nodes whose children are being cloned, with the index
        // of the next child; `done` holds finished clones, children of the
        // node on top of `work` last.
        let mut work: Vec<(&Pat, usize)> = Vec::new();
        let mut done: Vec<Pat> = Vec::new();
        work.push((self, 0));
        while let Some(top) = work.last_mut() {
            let (pat, next) = *top;
            if let Some(child) = pat.children.get(next) {
                top.1 += 1;
                work.push((child, 0));
                continue;
            }
            let _finished = work.pop();
            let children = done.split_off(done.len().saturating_sub(pat.children.len()));
            done.push(Pat {
                kind: pat.kind,
                children,
            });
        }
        // The root is the only clone left once its own children were taken.
        done.pop().unwrap_or_else(Pat::wild)
    }
}

/// A step of the iterative pattern printer.
enum Print<'a> {
    Pat(&'a Pat),
    Text(&'static str),
}

impl fmt::Display for Pat {
    /// Prints the pattern in a neutral syntax: `_`, `#i(..)` for variants,
    /// `(..)` for products, literals and `lo..=hi` ranges, `lit#id` for opaque
    /// literals, `[..]` for slices, and `p | q` for alternatives.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut stack = Vec::new();
        stack.push(Print::Pat(self));
        while let Some(item) = stack.pop() {
            let pat = match item {
                Print::Text(text) => {
                    f.write_str(text)?;
                    continue;
                }
                Print::Pat(pat) => pat,
            };
            let (open, close, sep) = match pat.kind {
                PatKind::Wild => {
                    f.write_str("_")?;
                    continue;
                }
                PatKind::Bool(b) => {
                    write!(f, "{b}")?;
                    continue;
                }
                PatKind::Int { lo, hi } => {
                    if lo == hi {
                        write!(f, "{lo}")?;
                    } else {
                        write!(f, "{lo}..={hi}")?;
                    }
                    continue;
                }
                PatKind::Char { lo, hi } => {
                    if lo == hi {
                        write!(f, "{lo:?}")?;
                    } else {
                        write!(f, "{lo:?}..={hi:?}")?;
                    }
                    continue;
                }
                PatKind::Lit(id) => {
                    write!(f, "lit#{id}")?;
                    continue;
                }
                PatKind::Or if pat.children.is_empty() => {
                    f.write_str("!")?;
                    continue;
                }
                PatKind::Or => ("", "", " | "),
                PatKind::Variant(index) => {
                    write!(f, "#{index}")?;
                    if pat.children.is_empty() {
                        continue;
                    }
                    ("(", ")", ", ")
                }
                PatKind::Product if pat.children.len() == 1 => ("(", ",)", ", "),
                PatKind::Product => ("(", ")", ", "),
                PatKind::Slice { prefix, rest } => {
                    f.write_str("[")?;
                    stack.push(Print::Text("]"));
                    let split = (prefix as usize).min(pat.children.len());
                    let (pre, post) = pat.children.split_at(split);
                    push_list(&mut stack, post, ", ");
                    if rest {
                        match (pre.is_empty(), post.is_empty()) {
                            (true, true) => stack.push(Print::Text("..")),
                            (true, false) => stack.push(Print::Text(".., ")),
                            (false, true) => stack.push(Print::Text(", ..")),
                            (false, false) => stack.push(Print::Text(", .., ")),
                        }
                    } else if !pre.is_empty() && !post.is_empty() {
                        stack.push(Print::Text(", "));
                    }
                    push_list(&mut stack, pre, ", ");
                    continue;
                }
            };
            f.write_str(open)?;
            stack.push(Print::Text(close));
            push_list(&mut stack, &pat.children, sep);
        }
        Ok(())
    }
}

/// Pushes `items` so that they pop in order, separated by `sep`.
fn push_list<'a>(stack: &mut Vec<Print<'a>>, items: &'a [Pat], sep: &'static str) {
    for (i, item) in items.iter().enumerate().rev() {
        stack.push(Print::Pat(item));
        if i > 0 {
            stack.push(Print::Text(sep));
        }
    }
}

impl fmt::Debug for Pat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pat({self})")
    }
}

/// One arm of a match: a pattern, and whether the arm has a guard.
///
/// A guard is treated as possibly failing: a guarded arm never makes later
/// arms unreachable and never counts toward exhaustiveness, but it is itself
/// reachable whenever a value matching its pattern can get past the earlier
/// unguarded arms. With an or-pattern, the guard is assumed to be tried again
/// for each alternative that matches, so alternatives of a guarded arm never
/// make each other redundant.
///
/// # Examples
///
/// ```
/// use match_lang::{Arm, Pat};
///
/// let plain = Arm::new(Pat::wild());
/// let guarded = Arm::guarded(Pat::int(0));
/// assert!(!plain.is_guarded());
/// assert!(guarded.is_guarded());
/// assert_eq!(guarded.pat().to_string(), "0");
/// ```
#[derive(Clone, Debug)]
pub struct Arm {
    pat: Pat,
    guarded: bool,
}

impl Arm {
    /// An arm without a guard.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Pat};
    ///
    /// let arm = Arm::new(Pat::bool(true));
    /// assert!(!arm.is_guarded());
    /// ```
    #[must_use]
    pub fn new(pat: Pat) -> Arm {
        Arm {
            pat,
            guarded: false,
        }
    }

    /// An arm with a guard (`pat if condition`). The condition itself is not
    /// analysed.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Pat};
    ///
    /// let arm = Arm::guarded(Pat::wild());
    /// assert!(arm.is_guarded());
    /// ```
    #[must_use]
    pub fn guarded(pat: Pat) -> Arm {
        Arm { pat, guarded: true }
    }

    /// The arm's pattern.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Pat};
    ///
    /// assert_eq!(Arm::new(Pat::int(3)).pat().to_string(), "3");
    /// ```
    #[must_use]
    pub fn pat(&self) -> &Pat {
        &self.pat
    }

    /// Whether the arm has a guard.
    ///
    /// # Examples
    ///
    /// ```
    /// use match_lang::{Arm, Pat};
    ///
    /// assert!(Arm::guarded(Pat::wild()).is_guarded());
    /// ```
    #[must_use]
    pub fn is_guarded(&self) -> bool {
        self.guarded
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    fn deep(depth: usize) -> Pat {
        let mut pat = Pat::wild();
        for _ in 0..depth {
            pat = Pat::variant(1, [pat]);
        }
        pat
    }

    #[test]
    fn test_display_covers_every_kind() {
        let pat = Pat::or([
            Pat::variant(2, [Pat::product([Pat::bool(true), Pat::range(1, 3)])]),
            Pat::product([Pat::char('a')]),
            Pat::product([]),
            Pat::slice_rest([Pat::lit(4)], [Pat::char_range('a', 'c')]),
            Pat::slice_rest([], [Pat::int(1)]),
            Pat::slice_rest([Pat::int(1)], []),
            Pat::slice([]),
            Pat::or([]),
        ]);
        assert_eq!(
            pat.to_string(),
            "#2((true, 1..=3)) | ('a',) | () | [lit#4, .., 'a'..='c'] | [.., 1] | [1, ..] | [] | !"
        );
        assert_eq!(alloc::format!("{pat:?}").len(), pat.to_string().len() + 5);
    }

    #[test]
    fn test_deep_pattern_clone_display_drop_do_not_recurse() {
        let pat = deep(200_000);
        let copy = pat.clone();
        let text = copy.to_string();
        assert_eq!(text.len(), 200_000 * 4 + 1);
        drop(pat);
        drop(copy);
    }

    #[test]
    fn test_clone_preserves_shape() {
        let pat = Pat::or([
            Pat::variant(1, [Pat::int(1), Pat::wild()]),
            Pat::slice([Pat::lit(2)]),
        ]);
        assert_eq!(pat.clone().to_string(), pat.to_string());
    }
}
