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
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
    table::{DataTable, TableEvent, TableState},
};
use std::{
    sync::{Arc, OnceLock},
    time::Instant,
};
use tokio::sync::{Semaphore, oneshot};
type Picker<T> = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<T>>>>;
type RawPicker = Entity<SelectState<SearchableVec<SharedString>>>;
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
fn raw_picker(
    items: impl IntoIterator<Item = SharedString>,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> RawPicker {
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
fn picker<T: Clone + PartialEq + 'static>(
    items: impl IntoIterator<Item = crate::i18n::LocalizedOption<T>>,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> Picker<T> {
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
fn value<T: Clone + PartialEq + Default + 'static>(picker: &Picker<T>, cx: &App) -> T {
    picker
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_default()
}
pub(super) enum View {
    Sockets {
        data: Arc<model::Sockets>,
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
    interval: Picker<u16>,
    proto: Picker<u16>,
    family: Picker<u16>,
    state: Picker<u8>,
    field: RawPicker,
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
    presentation: crate::live::presentation::Settings,
    saved_revision: Option<String>,
    preferences_ready: bool,
    preferences_loaded: bool,
    preferences_pending: bool,
    table: Entity<TableState<super::live_table::Rows>>,
    table_receiver: std::sync::mpsc::Receiver<super::live_table::Message>,
    marks: Arc<crate::live::table::Selection>,
    capture: crate::live::table::Capture,
}
impl LiveView {
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_query_ready(&self) -> bool {
        self.pending.is_none()
    }

    pub fn new(backend: &Handle, window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Search live rows or selected field…",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 256)
        });
        let pid = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder("Process ID", window, cx))
                .validate(|v, _| v.len() <= 10 && v.bytes().all(|b| b.is_ascii_digit()))
        });
        let interval = picker(
            INTERVALS
                .iter()
                .map(|(value, label)| crate::i18n::LocalizedOption::new(*value, *label)),
            2,
            window,
            cx,
        );
        let proto = picker(
            PROTOCOLS
                .iter()
                .map(|(value, label)| crate::i18n::LocalizedOption::new(*value, *label)),
            0,
            window,
            cx,
        );
        let family = picker(
            FAMILIES
                .iter()
                .map(|(value, label)| crate::i18n::LocalizedOption::new(*value, *label)),
            0,
            window,
            cx,
        );
        let states = (0u8..=12).map(|n| {
            if n == 0 {
                crate::i18n::LocalizedOption::new(n, "All states")
            } else {
                crate::i18n::LocalizedOption::raw(n, model::state(n))
            }
        });
        let state = picker(states, 0, window, cx);
        let field = raw_picker(["Status".into()], 0, window, cx);
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
        let (table_sender, table_receiver) = std::sync::mpsc::sync_channel(16);
        let table = cx.new(|cx| {
            TableState::new(
                super::live_table::Rows::new(table_sender, backend.highlighting.snapshot()),
                window,
                cx,
            )
            .col_movable(false)
        });
        cx.subscribe(&table, |this, table, event: &TableEvent, cx| {
            if let TableEvent::SelectRow(index) = event {
                this.live.selected = if let Some(View::Sockets { matches, .. }) = &this.live.parsed
                {
                    matches.get(this.live.offset + *index).copied()
                } else {
                    None
                };
                table.update(cx, |table, _| {
                    table.delegate_mut().selected = this.live.selected
                });
                cx.notify();
            }
            if let TableEvent::ColumnWidthsChanged(widths) = event {
                for (column, width) in this.live.presentation.columns.iter_mut().zip(widths) {
                    column.width = f32::from(*width).clamp(40., 2000.) as u16;
                }
                cx.notify();
            }
        })
        .detach();
        Self {
            presentation: Default::default(),
            saved_revision: None,
            preferences_ready: false,
            preferences_loaded: false,
            preferences_pending: false,
            table,
            table_receiver,
            marks: Arc::new(Default::default()),
            capture: crate::live::table::Capture {
                session: 0,
                revision: 0,
            },
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
            self.error = Some(crate::i18n::tr(
                "Choose a connected node with a notification stream in Nodes",
            ));
            return;
        };
        let node_label = session.name.clone();
        let seconds = value(&self.interval, cx);
        let kind = match self.mode {
            0 => {
                let state = value(&self.state, cx);
                Kind::Sockets {
                    seconds,
                    state,
                    proto: value(&self.proto, cx) as u8,
                    family: value(&self.family, cx) as u8,
                }
            }
            1 => {
                let Ok(pid) = self.pid.read(cx).value().parse() else {
                    self.error = Some(crate::i18n::tr("Enter a positive process ID"));
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
        Arc::make_mut(&mut self.marks).clear();
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
                seconds: value(&self.interval, cx),
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
        Arc::make_mut(&mut self.marks).clear();
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
        let saved = backend.live_presentation.snapshot();
        self.preferences_ready = saved.ready;
        if saved.ready && (!self.preferences_loaded || saved.revision != self.saved_revision) {
            self.preferences_loaded = true;
            self.saved_revision = saved.revision.clone();
            self.presentation = saved.settings.clone();
            self.interval.update(cx, |state, cx| {
                state.set_selected_value(&self.presentation.seconds, window, cx)
            });
            self.proto.update(cx, |state, cx| {
                state.set_selected_value(&self.presentation.protocol, window, cx)
            });
            self.family.update(cx, |state, cx| {
                state.set_selected_value(&self.presentation.family, window, cx)
            });
            self.state.update(cx, |state, cx| {
                state.set_selected_value(&self.presentation.state, window, cx)
            });
            self.input.update(cx, |input, cx| {
                input.set_value(self.presentation.search.clone(), window, cx)
            });
            self.generation += 1;
            self.dirty = true;
            changed = true;
        }
        while let Ok(message) = self.table_receiver.try_recv() {
            match message {
                super::live_table::Message::Sort(sort) => {
                    self.presentation.sort = sort;
                    self.generation += 1;
                    self.dirty = true;
                    self.offset = 0;
                    Arc::make_mut(&mut self.marks).forget_anchor();
                }
                super::live_table::Message::Mark {
                    capture,
                    position,
                    extend,
                } => {
                    if let Some(View::Sockets { matches, .. }) = &self.parsed
                        && let Err(error) = Arc::make_mut(&mut self.marks)
                            .toggle(capture, matches, position, extend)
                    {
                        self.error = Some(error.to_string());
                    }
                }
            }
            changed = true;
        }
        if let Some(error) = self
            .table
            .update(cx, |table, _| table.delegate_mut().error.take())
        {
            self.error = Some(error);
            changed = true;
        }
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
                                    self.capture = crate::live::table::Capture {
                                        session: self.session,
                                        revision: *revision,
                                    };
                                    Arc::make_mut(&mut self.marks).reset(self.capture);
                                    self.offset = self.offset.min(
                                        matches.len().saturating_sub(1) / model::PAGE * model::PAGE,
                                    );
                                    self.selected = None;
                                }
                                if let View::Fields { names, field, .. } = &view {
                                    let previous = self.field.read(cx).selected_value();
                                    if previous.is_none_or(|p|p.as_ref()!=field)||self.parsed.as_ref().is_none_or(|old|!matches!(old,View::Fields{names:old_names,..}if old_names==names)){
      self.field=raw_picker(names.iter().cloned().map(Into::into),names.iter().position(|n|n==field).unwrap_or(0),window,cx);
      self.field_name=field.clone();self.field_subscription=Some(cx.subscribe(&self.field,|this,state,_:&SelectEvent<SearchableVec<SharedString>>,cx|{if let Some(name)=state.read(cx).selected_value()&&name.as_ref()!=this.live.field_name{this.live.field_name=name.to_string();this.live.generation+=1;this.live.dirty=true;cx.notify();}}));
    }
                                }
                                self.parsed = Some(view);
                                self.error = None;
                            }
                            Err(error) => {
                                self.error = Some(crate::i18n::tr_args(
                                    "Cannot read this live update: %{error}; last displayed data is retained",
                                    &[("error", &error.to_string())],
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
                    self.error = Some(crate::i18n::tr("Live parser stopped"));
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
                let sort = self.presentation.sort;
                let field = self.field_name.clone();
                let (tx, rx) = oneshot::channel();
                self.pending = Some((generation, revision, rx));
                cx.background_executor()
                    .spawn(async move {
                        let _permit = permit;
                        let result = (|| -> anyhow::Result<View> {
                            if mode == 0 {
                                let data = model::sockets(raw)?;
                                let matches = crate::live::table::matching(&data, &search, sort);
                                let data = Arc::new(data);
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
    fn open_live_tools(&mut self, selected: Option<bool>, cx: &mut Context<Self>) {
        if selected.is_some() && (self.live.pending.is_some() || self.live.dirty) {
            self.live.error = Some(crate::i18n::tr(
                "Wait for the socket view to finish updating before exporting.",
            ));
            cx.notify();
            return;
        }
        let mut settings = self.live.presentation.clone();
        settings.search = self.live.input.read(cx).value().to_string();
        settings.seconds = value(&self.live.interval, cx);
        settings.protocol = value(&self.live.proto, cx);
        settings.family = value(&self.live.family, cx);
        settings.state = value(&self.live.state, cx);
        let export = if let Some(selected) = selected {
            let Some(View::Sockets { data, matches }) = &self.live.parsed else {
                return;
            };
            match crate::live::table::Export::capture(
                self.live.capture,
                data.clone(),
                matches,
                &settings
                    .columns
                    .iter()
                    .map(|column| column.field)
                    .collect::<Vec<_>>(),
                selected.then_some(self.live.marks.as_ref()),
            ) {
                Ok(export) => Some(export),
                Err(error) => {
                    self.live.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        if let Err(error) = super::live_tools::open(
            self.backend.live_presentation.clone(),
            settings,
            self.live.saved_revision.clone(),
            export,
            self.backend.history.database_path().to_owned(),
            cx,
        ) {
            self.live.error = Some(crate::i18n::tr_args(
                "Cannot open live tools: %{error}",
                &[("error", &error.to_string())],
            ));
        }
        cx.notify();
    }
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
                    .label(crate::i18n::tr("Start monitoring"))
                    .disabled(running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.start(&this.backend, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("live-pause")
                    .label(crate::i18n::tr("Pause"))
                    .disabled(!running)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.pause();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("live-view-tools")
                    .label(crate::i18n::tr("Live view preferences…"))
                    .disabled(!self.live.preferences_ready)
                    .on_click(cx.listener(|this, _, _, cx| this.open_live_tools(None, cx))),
            )
            .child(
                Button::new("live-view-reload")
                    .label(crate::i18n::tr("Reload live view"))
                    .disabled(running || self.live.preferences_pending)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.preferences_pending = true;
                        let receiver = this
                            .backend
                            .live_presentation
                            .request(crate::live::presentation::Command::Reload);
                        cx.spawn(async move |this, cx| {
                            let result = receiver.await;
                            let _ = this.update(cx, |this, cx| {
                                this.live.preferences_pending = false;
                                if !matches!(result, Ok(Ok(_))) {
                                    this.live.error = Some(crate::i18n::tr(
                                        "Live view reload is unavailable; try again.",
                                    ));
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("live-node")
                    .label(crate::i18n::tr("Choose node…"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.live.pause();
                        this.screen = Screen::Nodes;
                        cx.notify();
                    })),
            );
        if let Some(View::Sockets { data, matches }) = &self.live.parsed {
            let highlights = self.backend.highlighting.snapshot();
            self.live.table.update(cx, |table, cx| {
                let rows = table.delegate_mut();
                rows.data = Some(data.clone());
                if rows.order.as_ref() != matches {
                    rows.order = Arc::new(matches.clone());
                }
                rows.offset = self.live.offset;
                rows.columns = self.live.presentation.columns.clone();
                rows.sort = self.live.presentation.sort;
                rows.marks = self.live.marks.clone();
                rows.selected = self.live.selected;
                rows.capture = self.live.capture;
                rows.highlights = highlights;
                table.refresh(cx);
            });
        }
        let mut content = div().flex_1().min_h_0().flex().flex_col().gap_2();
        match &self.live.parsed {
   Some(View::Sockets{data,matches})=>{
    let end=(self.live.offset+model::PAGE).min(matches.len());let offset=self.live.offset.min(end);let count=end-offset;
    content=content.when(matches.is_empty(),|view|view.child(div().text_sm().child(crate::i18n::tr("No matching live sockets.")))).child(div().text_sm().child(crate::i18n::tr_args("%{arg0} matching sockets · showing %{arg1}–%{arg2} of %{arg3}%{arg4}", &[("arg0", &(matches.len()).to_string()), ("arg1", &(if count==0{0}else{offset+1}).to_string()), ("arg2", &(end).to_string()), ("arg3", &(data.rows.len()).to_string()), ("arg4", (if data.shortened{" · Snapshot shortened at retention limit; narrow daemon filters"}else{""}))])))
     .child(div().flex_1().min_h_0().child(DataTable::new(&self.live.table)))
     .child(div().text_sm().child(crate::i18n::tr_args("%{count} socket rows selected from this snapshot.",&[("count",&self.live.marks.len().to_string())])))
     .child(div().flex().flex_wrap().gap_2()
      .child(Button::new("live-previous").label(crate::i18n::tr("Previous")).disabled(offset==0).on_click(cx.listener(|this,_,_,cx|{this.live.offset=this.live.offset.saturating_sub(model::PAGE);cx.notify();})))
      .child(Button::new("live-next").label(crate::i18n::tr("Next")).disabled(end==matches.len()).on_click(cx.listener(|this,_,_,cx|{this.live.offset+=model::PAGE;cx.notify();})))
      .child(Button::new("live-select-all").label(crate::i18n::tr("Select all matching sockets")).on_click(cx.listener(|this,_,_,cx|{if let Some(View::Sockets{matches,..})=&this.live.parsed && let Err(error)=Arc::make_mut(&mut this.live.marks).all(this.live.capture,matches){this.live.error=Some(error.to_string());}cx.notify();})))
      .child(Button::new("live-clear-selection").label(crate::i18n::tr("Clear socket selection")).disabled(self.live.marks.is_empty()).on_click(cx.listener(|this,_,_,cx|{Arc::make_mut(&mut this.live.marks).clear();cx.notify();})))
      .child(Button::new("live-selected-output").label(crate::i18n::tr("Selected socket output…")).disabled(self.live.marks.is_empty()||self.live.pending.is_some()||self.live.dirty).on_click(cx.listener(|this,_,_,cx|this.open_live_tools(Some(true),cx))))
      .child(Button::new("live-all-output").label(crate::i18n::tr("All matching socket output…")).disabled(self.live.pending.is_some()||self.live.dirty).on_click(cx.listener(|this,_,_,cx|this.open_live_tools(Some(false),cx))))
      .child(Button::new("live-copy-row").label(crate::i18n::tr("Copy selected socket JSON")).disabled(self.live.selected.is_none()).on_click(cx.listener(|this,_,_,cx|{if let Some(View::Sockets{data,..})=&this.live.parsed&&let Some(row)=this.live.selected.and_then(|i|data.rows.get(i)){cx.write_to_clipboard(ClipboardItem::new_string(data.raw[row.raw.clone()].to_owned()));}})))
      .child(Button::new("live-process").label(crate::i18n::tr("Inspect selected process")).disabled(self.live.selected.and_then(|i|data.rows.get(i)).and_then(|r|r.pid).is_none()).on_click(cx.listener(|this,_,window,cx|{let pid=if let Some(View::Sockets{data,..})=&this.live.parsed{this.live.selected.and_then(|i|data.rows.get(i)).and_then(|r|r.pid)}else{None};if let Some(pid)=pid{this.live.mode(1);this.live.pid.update(cx,|input,cx|input.set_value(pid.to_string(),window,cx));this.live.start(&this.backend,cx);cx.notify();}}))));
   },
   Some(View::Fields{lines,shortened,summary,..})=>{
    content=content.when_some(summary.clone(),|view,summary|view.child(div().text_sm().child(summary))).child(div().h_8().w(px(260.)).child(Select::new(&self.live.field).id("live-field")))
     .when(*shortened,|d|d.child(div().text_sm().child(crate::i18n::tr("Preview shortened. Copy selected field or full JSON to keep complete data."))))
     .child(uniform_list("live-fields-list",lines.len(),cx.processor(|this,range:std::ops::Range<usize>,_,_|{let Some(View::Fields{lines,..})=&this.live.parsed else{return vec![]};range.map(|i|div().text_sm().font_family("monospace").child(lines[i].clone())).collect::<Vec<_>>()})).flex_1().min_h_0())
     .child(Button::new("live-copy-field").label(crate::i18n::tr("Copy complete selected field")).on_click(cx.listener(|this,_,_,cx|{if let Some(View::Fields{text,..})=&this.live.parsed{cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));}})));
   },
   None=>content=content.child(div().text_sm().child(crate::i18n::tr("Choose a connected node and start monitoring. Live inspection never changes firewall policy."))),
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
                    .child(Tab::new().label(crate::i18n::tr("Sockets")))
                    .child(Tab::new().label(crate::i18n::tr("Process details")))
                    .child(Tab::new().label(crate::i18n::tr("Node resources"))),
            )
            .child(div().text_sm().child(crate::i18n::tr_args("Snapshot node: %{arg0} · Start target: %{arg1} · leaving Live or closing this window requests task stop", &[("arg0", &(if self.live.node_label.is_empty() { crate::i18n::tr("no snapshot") } else { self.live.node_label.clone() })), ("arg1", &(self.backend
                        .status()
                        .session
                        .map(|s| s.name)
                        .unwrap_or_else(|| crate::i18n::tr("disconnected"))).to_string())])))
            .child(header)
            .when_some(self.live.origin.as_ref(), |view, origin| view.child(
                div().text_sm().child(crate::i18n::tr_args("Current inspection for connection %{arg0} · %{arg1} · PID %{arg2} · recorded %{arg3} · %{arg4}. PIDs may be reused; this is current daemon data, not recorded history.", &[("arg0", &(origin.event).to_string()), ("arg1", &(origin.node).to_string()), ("arg2", &(origin.pid).to_string()), ("arg3", &(chrono::DateTime::from_timestamp_nanos(origin.unixnano).to_rfc3339()).to_string()), ("arg4", &(crate::history::bounded(&origin.executable, 256)).to_string())]))
            ))
            .when_some(self.backend.live_presentation.snapshot().warning.clone(),|view,error|view.child(super::banner(error,cx)))
            .child(Input::new(&self.live.input).id("live-search"))
            .child(div().text_sm().child(format!(
                    "{}{}",
                    crate::i18n::presentation(&self.live.snapshot.status),
                    self.live
                        .snapshot
                        .received
                        .map(|t| crate::i18n::tr_args(" · last update %{arg0}s ago", &[("arg0", &format!("{}", Instant::now().saturating_duration_since(t).as_secs()))]))
                        .unwrap_or_default()
                )))
            .when_some(
                self.live.snapshot.error.clone().or(self.live.error.clone()),
                |d, e| d.child(super::banner(e, cx)),
            )
            .child(content)
            .child(
                Button::new("live-copy-json")
                    .label(crate::i18n::tr("Copy complete snapshot JSON"))
                    .disabled(self.live.parsed.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(view) = &this.live.parsed {
                            cx.write_to_clipboard(ClipboardItem::new_string(view.raw().to_owned()));
                        }
                    })),
            )
    }
}
