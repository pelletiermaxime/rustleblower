use clap::{CommandFactory, FromArgMatches, Parser, ValueEnum};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Decision {
    Allow,
    #[default]
    Deny,
    Reject,
}

impl Decision {
    pub fn action(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Reject => "reject",
        }
    }
}

#[derive(Clone, Debug, Parser)]
#[command(
    version,
    about = "Native OpenSnitch UI server. Existing sockets are never replaced."
)]
pub struct Config {
    /// Absolute Unix socket path, unix:///path, IPv4:port or [IPv6]:port.
    #[arg(long, default_value = "/tmp/osui.sock")]
    pub socket: PathBuf,
    #[command(flatten)]
    pub transport: crate::transport::Settings,
    #[arg(long, default_value_os_t = default_database())]
    pub database: PathBuf,
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=110))]
    pub prompt_timeout: u64,
    /// Applied once on timeout, overload, or explicit application shutdown.
    #[arg(long, value_enum, default_value_t = Decision::Deny)]
    pub timeout_action: Decision,
    #[arg(long, default_value_t = 7, value_parser = clap::value_parser!(u32).range(1..=365))]
    pub history_days: u32,
    /// Retention target; reserves space for SQLite pages and its WAL.
    #[arg(long, default_value_t = 256, value_parser = clap::value_parser!(u32).range(16..=4096))]
    pub history_mib: u32,
    /// Run the server without a desktop. Unanswered prompts use timeout-action.
    #[arg(long)]
    pub headless: bool,
}

impl Config {
    pub fn parse_launch() -> (Self, crate::history::storage::Overrides) {
        let matches = Self::command().get_matches();
        let overrides = crate::history::storage::Overrides {
            days: (matches.value_source("history_days")
                == Some(clap::parser::ValueSource::CommandLine))
            .then(|| *matches.get_one::<u32>("history_days").unwrap()),
            mib: (matches.value_source("history_mib")
                == Some(clap::parser::ValueSource::CommandLine))
            .then(|| *matches.get_one::<u32>("history_mib").unwrap()),
        };
        (
            Self::from_arg_matches(&matches).unwrap_or_else(|e| e.exit()),
            overrides,
        )
    }

    pub fn prompt_duration(&self) -> Duration {
        Duration::from_secs(self.prompt_timeout)
    }
}

fn default_database() -> PathBuf {
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    root.join("rustleblower/history.sqlite3")
}
