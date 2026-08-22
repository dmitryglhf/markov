#![recursion_limit = "256"]

#[cfg(not(any(feature = "rustls-tls", feature = "native-tls")))]
compile_error!("At least one of `rustls-tls` or `native-tls` features must be enabled");

#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!("Features `rustls-tls` and `native-tls` are mutually exclusive");

pub mod cli;
pub mod commands;
pub mod logging;
pub mod recipes;
pub mod scenario_tests;
pub mod session;
pub mod signal;

// Ours. Declared apart from the upstream list so a neighbour upstream deletes
// cannot turn this line into a conflict.
pub mod markov;

// Re-export commonly used types
pub use cli::Cli;
pub use session::CliSession;
