use crate::protocol::Event;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub available: bool,
    pub source: String,
    pub source_port: Option<u32>,
    pub destination_ip: String,
    pub host: String,
    pub user: Option<u32>,
    pub pid: Option<u32>,
    pub command: String,
    pub rule: String,
    pub duration: String,
    pub logged: Option<bool>,
    pub priority: Option<bool>,
    pub truncated: bool,
}
impl Metadata {
    pub fn from_event(event: &Event) -> Self {
        let mut metadata = Self {
            available: true,
            ..Default::default()
        };
        if let Some(c) = &event.connection {
            metadata.source = super::bounded(&c.src_ip, 64);
            metadata.source_port = Some(c.src_port);
            metadata.destination_ip = super::bounded(&c.dst_ip, 64);
            metadata.host = super::bounded(&c.dst_host, 1024);
            metadata.user = Some(c.user_id);
            metadata.pid = Some(c.process_id);
            for (index, arg) in c.process_args.iter().take(64).enumerate() {
                if index > 0 && metadata.command.len() < 8192 {
                    metadata.command.push(' ');
                }
                let room = 8192 - metadata.command.len();
                metadata.command.push_str(&super::bounded(arg, room));
                if arg.len() > room {
                    metadata.truncated = true;
                    break;
                }
            }
            metadata.truncated |= c.process_args.len() > 64
                || c.src_ip.len() > 64
                || c.dst_ip.len() > 64
                || c.dst_host.len() > 1024;
        }
        if let Some(rule) = &event.rule {
            metadata.rule = super::bounded(&rule.name, 1024);
            metadata.duration = super::bounded(&rule.duration, 64);
            metadata.logged = Some(!rule.nolog);
            metadata.priority = Some(rule.precedence);
            metadata.truncated |= rule.name.len() > 1024 || rule.duration.len() > 64;
        }
        metadata
    }
}
