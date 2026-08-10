//! Library platform infrastructure: strict project-local config, discovery,
//! and solution metadata. Kept isolated from the existing CLI's `ConfigImpl`
//! (spec §6.1: project-local config is not merged with the user-global one).

pub mod config;
pub mod discovery;
pub mod metadata;
pub mod solution_metadata;
