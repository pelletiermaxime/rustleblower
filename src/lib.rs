pub mod backend;
pub mod config;
pub mod history;
pub mod nodes;
pub mod preferences;
pub mod prompts;
pub mod rules;
pub mod statistics;
// Generated tonic signatures intentionally return tonic::Status by value.
#[allow(clippy::result_large_err)]
#[rustfmt::skip]
pub mod protocol;
#[cfg(feature = "desktop")]
pub mod app;
