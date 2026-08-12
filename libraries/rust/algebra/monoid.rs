//! Monoid trait shared across algebraic libraries.
//!
//! A monoid `(M, op, id)` satisfies associativity and identity:
//! `op(id, x) = op(x, id) = x` and `op(op(a, b), c) = op(a, op(b, c))`.

pub trait Monoid {
    type T: Clone;

    fn id() -> Self::T;

    fn op(a: &Self::T, b: &Self::T) -> Self::T;
}

pub struct AddMonoid;

impl Monoid for AddMonoid {
    type T = i64;

    fn id() -> Self::T {
        0
    }

    fn op(a: &Self::T, b: &Self::T) -> Self::T {
        a + b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_monoid_identity() {
        assert_eq!(AddMonoid::op(&AddMonoid::id(), &7), 7);
        assert_eq!(AddMonoid::op(&7, &AddMonoid::id()), 7);
    }

    #[test]
    fn add_monoid_associativity() {
        let a = AddMonoid::op(&AddMonoid::op(&1, &2), &3);
        let b = AddMonoid::op(&1, &AddMonoid::op(&2, &3));
        assert_eq!(a, b);
    }
}
