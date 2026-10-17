//! Frozen in-memory socket scopes; no update can retarget a reviewed export.
use super::{
    model::{SocketRow, Sockets},
    presentation::{Field, Sort},
};
use anyhow::{Result, ensure};
use std::{
    cmp::Ordering,
    collections::BTreeSet,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
};
impl Field {
    pub fn text(self, row: &SocketRow) -> String {
        let v = &row.values;
        match self {
            Self::Comm => v.comm.clone(),
            Self::Process => v.process.clone(),
            Self::State => super::model::state(v.state),
            Self::SourcePort => v.source_port.to_string(),
            Self::SourceIp => v.source_ip.clone(),
            Self::DestinationIp => v.destination_ip.clone(),
            Self::DestinationPort => v.destination_port.to_string(),
            Self::Protocol => super::model::protocol(v.protocol),
            Self::Uid => v.uid.to_string(),
            Self::Pid => row.pid.map(|n| n.to_string()).unwrap_or_default(),
            Self::Family => match v.family {
                2 => "IPv4".into(),
                10 => "IPv6".into(),
                17 => "Packet".into(),
                44 => "XDP".into(),
                n => format!("Family {n}"),
            },
            Self::Interface => v.interface.clone(),
            Self::Metadata => row.metadata.clone(),
        }
    }
    fn compare(self, a: &SocketRow, b: &SocketRow) -> Ordering {
        let av = &a.values;
        let bv = &b.values;
        match self {
            Self::Comm => av.comm.cmp(&bv.comm),
            Self::Process => av.process.cmp(&bv.process),
            Self::State => av.state.cmp(&bv.state),
            Self::SourcePort => av.source_port.cmp(&bv.source_port),
            Self::DestinationPort => av.destination_port.cmp(&bv.destination_port),
            Self::Protocol => av.protocol.cmp(&bv.protocol),
            Self::Uid => av.uid.cmp(&bv.uid),
            Self::Pid => a.pid.cmp(&b.pid),
            Self::Family => av.family.cmp(&bv.family),
            Self::SourceIp => address(&av.source_ip).cmp(&address(&bv.source_ip)),
            Self::DestinationIp => address(&av.destination_ip).cmp(&address(&bv.destination_ip)),
            Self::Interface => av.interface.cmp(&bv.interface),
            Self::Metadata => a.metadata.cmp(&b.metadata),
        }
    }
}
fn address(value: &str) -> (Option<std::net::IpAddr>, &str) {
    (value.parse().ok(), value)
}
pub fn matching(data: &Sockets, search: &str, sort: Sort) -> Vec<usize> {
    let mut rows = data
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| row.matches(search).then_some(i))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        let order = sort.field.compare(&data.rows[*a], &data.rows[*b]);
        let order = if sort.ascending {
            order
        } else {
            order.reverse()
        };
        order.then_with(|| a.cmp(b))
    });
    rows
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capture {
    pub session: u64,
    pub revision: u64,
}
#[derive(Clone, Default)]
pub struct Selection {
    capture: Option<Capture>,
    marked: BTreeSet<usize>,
    anchor: Option<usize>,
}
impl Selection {
    pub fn reset(&mut self, capture: Capture) {
        self.capture = Some(capture);
        self.marked.clear();
        self.anchor = None;
    }
    pub fn contains(&self, index: usize) -> bool {
        self.marked.contains(&index)
    }
    pub fn len(&self) -> usize {
        self.marked.len()
    }
    pub fn is_empty(&self) -> bool {
        self.marked.is_empty()
    }
    pub fn toggle(
        &mut self,
        capture: Capture,
        order: &[usize],
        position: usize,
        extend: bool,
    ) -> Result<()> {
        ensure!(
            self.capture == Some(capture),
            "Socket snapshot changed; select rows again"
        );
        ensure!(
            order.len() <= super::model::MAX_ROWS
                && order.iter().all(|i| *i < super::model::MAX_ROWS),
            "Invalid socket selection scope"
        );
        let index = *order
            .get(position)
            .ok_or_else(|| anyhow::anyhow!("Socket row is unavailable"))?;
        let selected = !self.marked.contains(&index);
        let range = if extend {
            self.anchor
                .filter(|a| *a < order.len())
                .map(|a| a.min(position)..=a.max(position))
        } else {
            None
        };
        for index in range
            .map(|r| order[r].to_vec())
            .unwrap_or_else(|| vec![index])
        {
            if selected {
                self.marked.insert(index);
            } else {
                self.marked.remove(&index);
            }
        }
        self.anchor = Some(position);
        Ok(())
    }
    pub fn all(&mut self, capture: Capture, order: &[usize]) -> Result<()> {
        ensure!(
            order.len() <= super::model::MAX_ROWS
                && order.iter().all(|i| *i < super::model::MAX_ROWS),
            "Invalid socket selection scope"
        );
        self.reset(capture);
        self.marked.extend(order);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.marked.clear();
        self.anchor = None;
    }
    pub fn forget_anchor(&mut self) {
        self.anchor = None;
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}
#[derive(Clone)]
pub struct Export {
    pub capture: Capture,
    pub data: Arc<Sockets>,
    pub order: Vec<usize>,
    pub columns: Vec<Field>,
}
impl Export {
    pub fn capture(
        capture: Capture,
        data: Arc<Sockets>,
        order: &[usize],
        columns: &[Field],
        selection: Option<&Selection>,
    ) -> Result<Self> {
        ensure!(
            !columns.is_empty()
                && columns.len() <= 13
                && columns.iter().copied().collect::<BTreeSet<_>>().len() == columns.len(),
            "Choose 1–13 distinct socket columns"
        );
        ensure!(
            order.len() <= super::model::MAX_ROWS
                && data.rows.len() <= super::model::MAX_ROWS
                && data.raw.len() <= super::MAX_UPDATE,
            "Socket export exceeds its snapshot limit"
        );
        let members = order.iter().copied().collect::<BTreeSet<_>>();
        ensure!(
            order.iter().all(|i| *i < data.rows.len()) && members.len() == order.len(),
            "Invalid socket export scope"
        );
        if let Some(selection) = selection {
            ensure!(
                selection.capture == Some(capture) && !selection.is_empty(),
                "Select rows from this socket snapshot first"
            );
            ensure!(
                selection.marked.iter().all(|i| members.contains(i)),
                "Socket selection changed; select rows again"
            );
        }
        let order = order
            .iter()
            .copied()
            .filter(|index| selection.is_none_or(|s| s.contains(*index)))
            .collect();
        Ok(Self {
            capture,
            data,
            order,
            columns: columns.to_vec(),
        })
    }
    fn write(
        &self,
        writer: &mut impl Write,
        format: Format,
        stop: &AtomicBool,
        limit: usize,
    ) -> Result<usize> {
        let mut written = 0;
        let mut append = |text: &str| -> Result<()> {
            ensure!(
                !stop.load(AtomicOrdering::Acquire),
                "Socket export cancelled"
            );
            ensure!(
                written + text.len() <= limit,
                "Socket export exceeds its size limit"
            );
            writer.write_all(text.as_bytes())?;
            written += text.len();
            Ok(())
        };
        if format == Format::Csv {
            append(&crate::history::export::csv_line(
                self.columns.iter().map(|field| field.label().to_owned()),
            ))?;
        } else {
            append("[")?;
        }
        for (position, index) in self.order.iter().enumerate() {
            let row = &self.data.rows[*index];
            if format == Format::Csv {
                append(&crate::history::export::csv_line(
                    self.columns.iter().map(|field| field.text(row)),
                ))?;
            } else {
                if position > 0 {
                    append(",")?;
                }
                append(&self.data.raw[row.raw.clone()])?;
            }
        }
        if format == Format::Json {
            append("]")?;
        }
        Ok(written)
    }
    pub fn copy(&self, format: Format, stop: &AtomicBool) -> Result<String> {
        let mut bytes = Vec::new();
        self.write(&mut bytes, format, stop, 4 * 1024 * 1024)?;
        Ok(String::from_utf8(bytes)?)
    }
    pub fn save(
        &self,
        database: &Path,
        target: &Path,
        replace: bool,
        format: Format,
        stop: &AtomicBool,
    ) -> Result<usize> {
        crate::history::export::validate_target(database, target, replace)?;
        let parent = target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Choose an export directory"))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        let size = {
            let mut writer = BufWriter::with_capacity(16 * 1024, &mut temp);
            let size = self.write(&mut writer, format, stop, 16 * 1024 * 1024)?;
            writer.flush()?;
            size
        };
        temp.as_file().sync_all()?;
        ensure!(
            !stop.load(AtomicOrdering::Acquire),
            "Socket export cancelled"
        );
        crate::history::export::validate_target(database, target, replace)?;
        if replace {
            temp.persist(target).map_err(|error| error.error)?;
        } else {
            temp.persist_noclobber(target)
                .map_err(|error| error.error)?;
        }
        std::fs::File::open(parent)?.sync_all()?;
        Ok(size)
    }
}
