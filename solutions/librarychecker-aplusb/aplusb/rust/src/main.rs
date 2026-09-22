// Library Checker A+B — uses the shared AddMonoid via `#[path]` import so
// the solution page can surface `libraries/rust/algebra/monoid.rs` as a
// direct dependency. `hooks/expand-libraries.sh` inlines the library at
// submission time so the submitted single-file source still compiles on
// the judge.

mod libs;
use libs::monoid::{AddMonoid, Monoid};

use std::io::{self, BufRead, BufWriter, Write};

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
