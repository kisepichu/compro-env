//! Concrete `SubmissionStarter` implementations for each supported OJ.
//!
//! These are separate from `online_judge_impl/` because the submission lifecycle
//! is a distinct port: `ce verify` and `ce submit --watch` compose the starter,
//! poller, and recovery adapters, and only some OJs will grow the trackable
//! adapters. Login / whoami / problem metadata stays on `OnlineJudge`.
//!
//! Only starters live here today; pollers and recovery adapters land with
//! plan 058 (LibraryChecker lifecycle).

pub mod atcoder;
pub mod librarychecker;
pub mod registry;
