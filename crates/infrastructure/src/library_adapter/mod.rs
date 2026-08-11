//! Filesystem and child-process machinery for language adapters (spec §6.9).
//!
//! Everything here is I/O heavy on purpose so the domain and use-case layers
//! can stay platform-agnostic.

pub mod archive;
pub mod build;
pub mod build_state;
pub mod download;
pub mod inputs;
pub mod prepare;
pub mod prepared;
pub mod process;
