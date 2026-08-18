//! Library pins for this solution.
//!
//! `#[path]` requires a string literal (no `env!`/`concat!` expansion), so
//! the long relative path is consolidated here to keep `main.rs` short.
//! `hooks/expand-libraries.sh` (Rust branch) inlines the referenced file
//! into the submission source at OJ submission time.

#[path = "../../../../../libraries/rust/algebra/monoid.rs"]
pub mod monoid;
