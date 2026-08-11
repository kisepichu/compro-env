//! Reusable analysis primitives for the ce-rust adapter.
//!
//! `main.rs` is the process entry point; everything reusable lives here so
//! integration tests can call directly into the resolver without spawning a
//! subprocess.

pub mod dependencies;
pub mod module_graph;
pub mod request;
