// Same limit the rest of the workspace carries: goose, goose-cli and our own
// main.rs all hit rustc's default of 128 on this dependency graph. The lib
// target needs its own copy — the attribute is per crate target, so main.rs
// having it never covered this one.
#![recursion_limit = "256"]

pub mod cli;
pub mod commands;
pub mod hooks;
pub mod ui;
