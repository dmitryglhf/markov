//! Everything of ours that has to live inside the upstream crate.
//!
//! Upstream has no `src/markov/`, so nothing here ever appears in a merge.

pub mod hooks;
pub mod types;
pub mod ui;
