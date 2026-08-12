// Library Checker A+B — reference implementation using the Rust monoid.
//
// The problem reads two integers per line and prints their sum.
// The `AddMonoid` in `libraries/rust/algebra/monoid.rs` is inlined below to
// keep the submission self-contained; the on-disk library is what the site
// publishes and what verification records reference.

use std::io::{self, BufRead, BufWriter, Write};

trait Monoid {
    type T: Clone;
    fn id() -> Self::T;
    fn op(a: &Self::T, b: &Self::T) -> Self::T;
}

struct AddMonoid;

impl Monoid for AddMonoid {
    type T = i64;
    fn id() -> Self::T {
        0
    }
    fn op(a: &Self::T, b: &Self::T) -> Self::T {
        a + b
    }
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line.expect("read line");
        let mut it = line.split_whitespace();
        let (Some(a), Some(b)) = (it.next(), it.next()) else {
            continue;
        };
        let a: i64 = a.parse().expect("parse a");
        let b: i64 = b.parse().expect("parse b");
        let acc = AddMonoid::op(&AddMonoid::op(&AddMonoid::id(), &a), &b);
        writeln!(out, "{acc}").expect("write");
    }
}
