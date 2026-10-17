pub mod alerts;
pub mod appearance;
pub mod backend;
pub mod config;
pub mod desktop_notifications;
pub mod firewall;
pub mod highlighting;
pub mod history;
pub mod i18n;
pub mod live;
pub mod nodes;
pub mod preferences;
pub mod prompts;
pub mod raw_downloads;
pub mod reputation;
pub mod rules;
pub mod startup;
pub mod statistics;
pub mod subscriptions;
pub mod transport;
pub mod updates;
pub mod window_state;
// Generated tonic signatures intentionally return tonic::Status by value.
#[allow(clippy::result_large_err)]
#[rustfmt::skip]
pub mod protocol;
#[cfg(feature = "desktop")]
pub mod app;

pub mod view_preferences;
