//! Bounded live socket/process/resource inspection through daemon-owned tasks.
use super::{Dashboard, Screen};
use crate::{
    backend::Handle,
    live::{Kind, Lease, Snapshot, model},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Disableable, IndexPath,
    button::Button,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::sync::{Semaphore, oneshot};
type Picker = Entity<SelectState<SearchableVec<SharedString>>>;
const INTERVALS: &[(u16, &str)] = &[
    (1, "1 second"),
    (2, "2 seconds"),
    (5, "5 seconds"),
    (10, "10 seconds"),
    (30, "30 seconds"),
    (60, "60 seconds"),
];
const PROTOCOLS: &[(u16, &str)] = &[
    (0, "All protocols"),
    (6, "TCP"),
    (17, "UDP"),
    (136, "UDPLite"),
    (132, "SCTP"),
    (33, "DCCP"),
    (1, "ICMP"),
    (58, "ICMPv6"),
    (2, "IGMP"),
    (255, "RAW"),
];
const FAMILIES: &[(u16, &str)] = &[
    (0, "All families"),
    (2, "IPv4"),
    (10, "IPv6"),
    (17, "Packet"),
    (44, "XDP"),
];
fn picker(
    items: impl IntoIterator<Item = SharedString>,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> Picker {
    cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(items.into_iter().collect::<Vec<_>>()),
            Some(IndexPath::new(index)),
            window,
            cx,
        )
        .searchable(true)
    })
}
fn value(picker: &Picker, values: &[(u16, &str)], cx: &App) -> u16 {
    picker
        .read(cx)
        .selected_value()
        .and_then(|label| {
            values
                .iter()
                .find(|(_, v)| *v == label.as_ref())
                .map(|(v, _)| *v)
        })
        .unwrap_or(0)
}
pub(super) enum View {
    Sockets {
        data: model::Sockets,
        matches: Vec<usize>,
    },
    Fields {
        raw: Arc<str>,
        names: Vec<String>,
        field: String,
        text: String,
        lines: Vec<String>,
        shortened: bool,
        summary: Option<String>,
    },
}
impl View {
    fn raw(&self) -> &str {
        match self {
            Self::Sockets { data, .. } => &data.raw,
            Self::Fields { raw, .. } => raw,
        }
    }
}
type Parsed = oneshot::Receiver<Result<View, String>>;
pub(super) struct LiveView {
    pub input: Entity<InputState>,
    pid: Entity<InputState>,
    interval: Picker,
    proto: Picker,
    family: Picker,
    state: Picker,
    field: Picker,
    field_subscription: Option<Subscription>,
    mode: usize,
    task: Option<Lease>,
    session: u64,
    node_label: String,
    pub snapshot: Snapshot,
    pending: Option<(u64, u64, Parsed)>,
    generation: u64,
    dirty: bool,
    pub parsed: Option<View>,
    field_name: String,
    offset: usize,
    selected: Option<usize>,
    error: Option<String>,
    message: Option<String>,
    origin: Option<super::connection_details::ProcessTarget>,
}
impl LiveView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search live rows or selected field…")
                .validate(|v, _| v.len() <= 256)
        });
        let pid = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Process ID")
                .validate(|v, _| v.len() <= 10 && v.bytes().all(|b| b.is_ascii_digit()))
        });
        let interval = picker(INTERVALS.iter().map(|(_, s)| (*s).into()), 2, window, cx);
        let proto = picker(PROTOCOLS.iter().map(|(_, s)| (*s).into()), 0, window, cx);
        let family = picker(FAMILIES.iter().map(|(_, s)| (*s).into()), 0, window, cx);
        let states = (0..=12)
            .map(|n| {
                SharedString::from(if n == 0 {
                    "All states".into()
                } else {
                    model::state(n)
                })
            })
            .collect::<Vec<_>>();
        let state = picker(states, 0, window, cx);
        let field = picker(["Status".into()], 0, window, cx);
        cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.live.generation += 1;
                this.live.dirty = true;
                this.live.offset = 0;
                this.live.selected = None;
                cx.notify();
            }
        })
        .detach();
        Self {
            input,
            pid,
            interval,
            proto,
            family,
            state,
            field,
            field_subscription: None,
            mode: 0,
            task: None,
            session: 0,
            node_label: String::new(),
            snapshot: Default::default(),
            pending: None,
            generation: 0,
            dirty: false,
            parsed: None,
            field_name: "Status".into(),
            offset: 0,
            selected: None,
            error: None,
            message: None,
            origin: None,
        }
    }
    pub fn pause(&self) {
        if let Some(task) = &self.task
            && task.snapshot().running
        {
            task.stop();
        }
    }
    pub(super) fn start(&mut self, backend: &Handle, cx: &mut Context<Dashboard>) {
        self.pause();
        self.error = None;
        self.message = None;
        let Some(session) = backend.status().session.filter(|s| s.notifications) else {
            self.error = Some("Choose a connected node with a notification stream in Nodes".into());
            return;
        };
        let node_label = session.name.clone();
        let seconds = value(&self.interval, INTERVALS, cx);
        let kind = match self.mode {
            0 => {
                let state = self
                    .state
                    .read(cx)
                    .selected_value()
                    .and_then(|label| {
                        (0..=12).find(|n| {
                            label.as_ref()
                                == if *n == 0 {
                                    "All states".into()
                                } else {
                                    model::state(*n)
                                }
                        })
                    })
                    .unwrap_or(0);
                Kind::Sockets {
                    seconds,
                    state,
                    proto: value(&self.proto, PROTOCOLS, cx) as u8,
                    family: value(&self.family, FAMILIES, cx) as u8,
                }
            }
            1 => {
                let Ok(pid) = self.pid.read(cx).value().parse() else {
                    self.error = Some("Enter a positive process ID".into());
                    return;
                };
                Kind::Process { seconds, pid }
            }
            _ => Kind::Resources {
                seconds,
                node: session.name,
            },
        };
        if self.origin.as_ref().is_some_and(|origin| {
            origin.session != session.id
                || !matches!(&kind, Kind::Process { pid, .. } if *pid == origin.pid)
        }) {
            self.origin = None;
        }
        match backend.live.start(session.id, kind) {
            Ok(task) => {
                self.adopt(task, session.id, node_label);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn adopt(&mut self, task: Lease, session: u64, node: String) {
        self.task = Some(task);
        self.session = session;
        self.node_label = node;
        self.snapshot = Default::default();
        self.generation += 1;
        self.dirty = false;
        self.pending = None;
        self.parsed = None;
        self.offset = 0;
        self.selected = None;
    }

    pub(super) fn inspect_history(
        &mut self,
        backend: &Handle,
        target: super::connection_details::ProcessTarget,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Result<(), String> {
        if target.pid == 0 || target.pid > i32::MAX as u32 {
            return Err("This event has no valid Linux process ID".into());
        }
        if !backend
            .session(target.session)
            .is_some_and(|s| s.name == target.node && s.notifications)
        {
            return Err("This event's daemon changed or disconnected; reopen process inspection after reconnecting".into());
        }
        self.pause();
        let task = backend.live.start(
            target.session,
            Kind::Process {
                seconds: value(&self.interval, INTERVALS, cx),
                pid: target.pid,
            },
        )?;
        // Select the captured session, never whichever daemon happens to be selected now.
        if let Err(error) = backend.select_node(target.session) {
            task.stop();
            return Err(error);
        }
        self.mode(1);
        self.pid.update(cx, |pid, cx| {
            pid.set_value(target.pid.to_string(), window, cx)
        });
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.adopt(task, target.session, target.node.clone());
        self.origin = Some(target);
        self.input.focus_handle(cx).focus(window, cx);
        Ok(())
    }
    fn mode(&mut self, mode: usize) {
        self.pause();
        self.mode = mode;
        self.generation += 1;
        self.parsed = None;
        self.task = None;
        self.snapshot = Default::default();
        self.pending = None;
        self.error = None;
        self.message = None;
        self.origin = None;
        self.offset = 0;
        self.selected = None;
        self.field_name = if mode == 1 {
            "Status".into()
        } else {
            "Uptime".into()
        };
    }
    pub fn poll(
        &mut self,
        backend: &Handle,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        if let Some(task) = &self.task {
            if backend.status().session.as_ref().map(|s| s.id) != Some(self.session) {
                self.pause();
            }
            let current = task.snapshot();
            if current.revision != self.snapshot.revision {
                self.dirty = true;
                changed = true;
            }
            if current.status != self.snapshot.status
                || current.error != self.snapshot.error
                || current.running != self.snapshot.running
            {
                changed = true;
            }
            self.snapshot = current;
        }
        if let Some((generation, revision, rx)) = self.pending.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    if *generation == self.generation && *revision == self.snapshot.revision {
                        match result {
                            Ok(view) => {
                                if let View::Sockets { matches, .. } = &view {
                                    self.offset = self.offset.min(
                                        matches.len().saturating_sub(1) / model::PAGE * model::PAGE,
                                    );
                                    self.selected = None;
                                }
                                if let View::Fields { names, field, .. } = &view {
                                    let previous = self.field.read(cx).selected_value();
                                    if previous.is_none_or(|p|p.as_ref()!=field)||self.parsed.as_ref().is_none_or(|old|!matches!(old,View::Fields{names:old_names,..}if old_names==names)){
      self.field=picker(names.iter().cloned().map(Into::into),names.iter().position(|n|n==field).unwrap_or(0),window,cx);
      self.field_name=field.clone();self.field_subscription=Some(cx.subscribe(&self.field,|this,state,_:&SelectEvent<SearchableVec<SharedString>>,cx|{if let Some(name)=state.read(cx).selected_value()&&name.as_ref()!=this.live.field_name{this.live.field_name=name.to_string();this.live.generation+=1;this.live.dirty=true;cx.notify();}}));
    }
                                }
                                self.parsed = Some(view);
                                self.error = None;
                            }
                            Err(error) => {
                                self.error = Some(format!(
                                    "Cannot read this live update: {error}; last displayed data is retained"
                                ))
                            }
                        }
                    } else {
                        self.dirty = true;
                    }
                    self.pending = None;
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some("Live parser stopped".into());
                    changed = true;
                }
                Err(_) => {}
            }
        }
        if self.pending.is_none()
            && self.dirty
            && let Some(raw) = self.snapshot.raw.clone()
        {
            static PARSER: OnceLock<Arc<Semaphore>> = OnceLock::new();
            if let Ok(permit) = PARSER
                .get_or_init(|| Arc::new(Semaphore::new(1)))
                .clone()
                .try_acquire_owned()
            {
                self.dirty = false;
                let generation = self.generation;
                let revision = self.snapshot.revision;
                let mode = self.mode;
                let search = self.input.read(cx).value().to_string();
                let field = self.field_name.clone();
                let (tx, rx) = oneshot::channel();
                self.pending = Some((generation, revision, rx));
                cx.background_executor()
                    .spawn(async move {
                        let _permit = permit;
                        let result = (|| -> anyhow::Result<View> {
                            if mode == 0 {
                                let data = model::sockets(raw)?;
                                let matches = data
                                    .rows
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, r)| r.matches(&search).then_some(i))
                                    .collect();
                                Ok(View::Sockets { data, matches })
                            } else {
                                let mut fields = model::fields(raw)?;
                                let mut names = fields
                                    .fields
                                    .iter()
                                    .map(|f| f.name.clone())
                                    .collect::<Vec<_>>();
                                if names.is_empty() {
                                    names.push("No reported fields".into());
                                }
                                let index = fields
                                    .fields
                                    .iter()
                                    .position(|f| f.name == field)
                                    .unwrap_or(0);
                                let selected = if fields.fields.is_empty() {
                                    model::Field {
                                        name: "No reported fields".into(),
                                        text: "No fields reported".into(),
                                    }
                                } else {
                                    fields.fields.swap_remove(index)
                                };
                                let (lines, preview_shortened) =
                                    model::preview(&selected.text, &search);
                                let shortened = fields.shortened || preview_shortened;
                                let summary = (mode == 2).then(|| {
                                    model::resources(&fields.raw).unwrap_or_else(|_| {
                                        "Summary unavailable; inspect reported fields and Unit"
                                            .into()
                                    })
                                });
                                Ok(View::Fields {
                                    raw: fields.raw,
                                    names,
                                    field: selected.name,
                                    text: selected.text,
                                    lines,
                                    shortened,
                                    summary,
                                })
                            }
                        })()
                        .map_err(|e| e.to_string());
                        let _ = tx.send(result);
                    })
                    .detach();
            }
        }
        changed
    }
}
impl Dashboard {
    pub(super) fn render_live(&mut self, cx: &mut Context<Self>) -> Div {
        let running = self.live.snapshot.running;
        let header = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                div().w(px(150.)).h_8().child(
                    Select::new(&self.live.interval)
                        .id("live-interval")
                        .disabled(running),
                ),
            )
            .when(self.live.mode == 0, |d| {
                d.child(
                    div().w(px(140.)).h_8().child(
                        Select::new(&self.live.proto)
                            .id("live-protocol")
                            .disabled(running),
                    ),
                )
                .child(
                    div().w(px(140.)).h_8().child(
                        Select::new(&self.live.family)
                            .id("live-family")
                            .disabled(running),
                    ),
                )
                .child(
                    div().w(px(165.)).h_8().child(
                        Select::new(&self.live.state)
                            .id("live-state")
                            .disabled(running),
                    ),
                )
            })
            .when(self.live.mode == 1, |d| {
                d.child(
                    div()
                        .w(px(140.))
                        .child(Input::new(&self.live.pid).id("live-pid").disabled(running)),
                )
            })
            .child(
                Button::new("live-start")
                    .label("Start monitoring")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.start(&this.backend, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("live-pause")
                    .label("Pause")
                    .disabled(!running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.pause();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("live-node")
                    .label("Choose node…")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.pause();
                        this.screen = Screen::Nodes;
                        cx.notify();
                    })),
            );
        let mut content = div().flex_1().min_h_0().flex().flex_col().gap_2();
        match &self.live.parsed {
   Some(View::Sockets{data,matches})=>{
    let end=(self.live.offset+model::PAGE).min(matches.len());let offset=self.live.offset.min(end);let count=end-offset;
    content=content.when(matches.is_empty(),|view|view.child(div().text_sm().child("No matching live sockets."))).child(div().text_sm().child(format!("{} matching sockets · showing {}–{} of {}{}",matches.len(),if count==0{0}else{offset+1},end,data.rows.len(),if data.shortened{" · Snapshot shortened at retention limit; narrow daemon filters"}else{""})))
     .child(uniform_list("live-sockets-list",count,cx.processor(|this,range:std::ops::Range<usize>,_,cx|{let Some(View::Sockets{data,matches})=&this.live.parsed else{return vec![]};let offset=this.live.offset.min(matches.len());range.filter_map(|i|matches.get(offset+i).map(|index|(*index,&data.rows[*index]))).map(|(index,row)|ListItem::new(("live-socket",index as u64)).selected(this.live.selected==Some(index)).child(div().flex().flex_col().gap_1().child(div().text_sm().truncate().child(row.title.clone())).child(div().text_sm().truncate().child(row.endpoints.clone())).child(div().text_xs().truncate().child(row.metadata.clone()))).on_click(cx.listener(move|this,_,_,cx|{this.live.selected=Some(index);cx.notify();}))).collect::<Vec<_>>()})).flex_1().min_h_0())
     .child(div().flex().gap_2()
      .child(Button::new("live-previous").label("Previous").disabled(offset==0).on_click(cx.listener(|this,_,_,cx|{this.live.offset=this.live.offset.saturating_sub(model::PAGE);cx.notify();})))
      .child(Button::new("live-next").label("Next").disabled(end==matches.len()).on_click(cx.listener(|this,_,_,cx|{this.live.offset+=model::PAGE;cx.notify();})))
      .child(Button::new("live-copy-row").label("Copy selected socket JSON").disabled(self.live.selected.is_none()).on_click(cx.listener(|this,_,_,cx|{if let Some(View::Sockets{data,..})=&this.live.parsed&&let Some(row)=this.live.selected.and_then(|i|data.rows.get(i)){cx.write_to_clipboard(ClipboardItem::new_string(data.raw[row.raw.clone()].to_owned()));}})))
      .child(Button::new("live-process").label("Inspect selected process").disabled(self.live.selected.and_then(|i|data.rows.get(i)).and_then(|r|r.pid).is_none()).on_click(cx.listener(|this,_,window,cx|{let pid=if let Some(View::Sockets{data,..})=&this.live.parsed{this.live.selected.and_then(|i|data.rows.get(i)).and_then(|r|r.pid)}else{None};if let Some(pid)=pid{this.live.mode(1);this.live.pid.update(cx,|input,cx|input.set_value(pid.to_string(),window,cx));this.live.start(&this.backend,cx);cx.notify();}}))));
   },
   Some(View::Fields{lines,shortened,summary,..})=>{
    content=content.when_some(summary.clone(),|view,summary|view.child(div().text_sm().child(summary))).child(div().h_8().w(px(260.)).child(Select::new(&self.live.field).id("live-field")))
     .when(*shortened,|d|d.child(div().text_sm().child("Preview shortened. Copy selected field or full JSON to keep complete data.")))
     .child(uniform_list("live-fields-list",lines.len(),cx.processor(|this,range:std::ops::Range<usize>,_,_|{let Some(View::Fields{lines,..})=&this.live.parsed else{return vec![]};range.map(|i|div().text_sm().font_family("monospace").child(lines[i].clone())).collect::<Vec<_>>()})).flex_1().min_h_0())
     .child(Button::new("live-copy-field").label("Copy complete selected field").on_click(cx.listener(|this,_,_,cx|{if let Some(View::Fields{text,..})=&this.live.parsed{cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));}})));
   },
   None=>content=content.child(div().text_sm().child("Choose a connected node and start monitoring. Live inspection never changes firewall policy.")),
  }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                TabBar::new("live-tabs")
                    .selected_index(self.live.mode)
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        if this.live.mode != *index {
                            this.live.mode(*index);
                            this.live.input.focus_handle(cx).focus(window, cx);
                            cx.notify();
                        }
                    }))
                    .child(Tab::new().label("Sockets"))
                    .child(Tab::new().label("Process details"))
                    .child(Tab::new().label("Node resources")),
            )
            .child(div().text_sm().child(format!(
                    "Snapshot node: {} · Start target: {} · leaving Live or closing this window requests task stop",
                    if self.live.node_label.is_empty() { "no snapshot" } else { &self.live.node_label },
                    self.backend
                        .status()
                        .session
                        .map(|s| s.name)
                        .unwrap_or_else(|| "disconnected".into())
                )))
            .child(header)
            .when_some(self.live.origin.as_ref(), |view, origin| view.child(
                div().text_sm().child(format!(
                    "Current inspection for connection {} · {} · PID {} · recorded {} · {}. PIDs may be reused; this is current daemon data, not recorded history.",
                    origin.event, origin.node, origin.pid,
                    chrono::DateTime::from_timestamp_nanos(origin.unixnano).to_rfc3339(),
                    crate::history::bounded(&origin.executable, 256)
                ))
            ))
            .child(Input::new(&self.live.input).id("live-search"))
            .child(div().text_sm().child(format!(
                    "{}{}",
                    self.live.snapshot.status,
                    self.live
                        .snapshot
                        .received
                        .map(|t| format!(
                            " · last update {}s ago",
                            Instant::now().saturating_duration_since(t).as_secs()
                        ))
                        .unwrap_or_default()
                )))
            .when_some(
                self.live.snapshot.error.clone().or(self.live.error.clone()),
                |d, e| d.child(super::banner(e, cx)),
            )
            .child(content)
            .child(
                Button::new("live-copy-json")
                    .label("Copy complete snapshot JSON")
                    .disabled(self.live.parsed.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(view) = &this.live.parsed {
                            cx.write_to_clipboard(ClipboardItem::new_string(view.raw().to_owned()));
                        }
                    })),
            )
    }
}
