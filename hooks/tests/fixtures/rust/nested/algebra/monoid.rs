pub trait Monoid {
    type T: Clone;
    fn id() -> Self::T;
    fn op(a: &Self::T, b: &Self::T) -> Self::T;
}

pub struct AddMonoid;

impl Monoid for AddMonoid {
    type T = i64;
    fn id() -> Self::T { 0 }
    fn op(a: &Self::T, b: &Self::T) -> Self::T { a + b }
}
