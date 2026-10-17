//! A single on-demand connection snapshot with native Kit descriptions and tabs.
use crate::{
    backend::Handle,
    history::details::{Detail, DraftKind, endpoint},
    protocol::Rule,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    description_list::DescriptionList,
    tab::{Tab, TabBar},
};
use std::sync::mpsc::SyncSender;

pub(super) struct OpenRule {
    pub node: String,
    pub session: u64,
    pub rule: Rule,
    pub existing: bool,
    pub reply: tokio::sync::oneshot::Sender<Result<(), String>>,
}

#[derive(Clone)]
pub(super) struct ProcessTarget {
    pub node: String,
    pub session: u64,
    pub pid: u32,
    pub executable: String,
    pub event: i64,
    pub unixnano: i64,
}
pub(super) struct OpenProcess {
    pub target: ProcessTarget,
    pub reply: tokio::sync::oneshot::Sender<Result<(), String>>,
}

pub(super) fn open(
    backend: Handle,
    detail: Detail,
    actions: SyncSender<OpenRule>,
    processes: SyncSender<OpenProcess>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(860.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(680.), px(520.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — connection details").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| ConnectionDetails::new(backend, detail, actions, processes, window, cx))
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — connection details", cx);
    Ok(window)
}

struct ConnectionDetails {
    backend: Handle,
    detail: Detail,
    actions: SyncSender<OpenRule>,
    processes: SyncSender<OpenProcess>,
    tab: usize,
    error: Option<String>,
    message: Option<String>,
    busy: bool,
    focus: FocusHandle,
}
impl ConnectionDetails {
    fn new(
        backend: Handle,
        detail: Detail,
        actions: SyncSender<OpenRule>,
        processes: SyncSender<OpenProcess>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            backend,
            detail,
            actions,
            processes,
            tab: 0,
            error: None,
            message: None,
            busy: false,
            focus,
        }
    }
    fn session(&self) -> Option<u64> {
        self.backend
            .node(&self.detail.row.node)
            .filter(|session| {
                session.notifications && self.backend.history.health.catalogues.ready(session.id)
            })
            .map(|session| session.id)
    }

    fn process_target(&self) -> Option<ProcessTarget> {
        let connection = self.detail.connection()?;
        if connection.process_id == 0 || connection.process_id > i32::MAX as u32 {
            return None;
        }
        let session = self.backend.node(&self.detail.row.node)?;
        session.notifications.then(|| ProcessTarget {
            node: session.name,
            session: session.id,
            pid: connection.process_id,
            executable: crate::history::bounded(&self.detail.row.process, 4096),
            event: self.detail.row.id,
            unixnano: self.detail.row.unixnano,
        })
    }

    fn inspect_process(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.process_target() else {
            self.error = Some(crate::i18n::tr(
                "Connect this event's daemon; a recorded positive PID is required",
            ));
            cx.notify();
            return;
        };
        let (reply, pending) = tokio::sync::oneshot::channel();
        match self.processes.try_send(OpenProcess { target, reply }) {
            Ok(()) => {
                self.busy = true;
                self.error = None;
                self.message = Some(crate::i18n::tr(
                    "Opening current process data on this event's daemon…",
                ));
                cx.spawn(async move |this, cx| {
                    let result = pending.await.unwrap_or_else(|_| {
                        Err("Process inspection request stopped before opening".into())
                    });
                    let _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => {
                                this.message = Some(crate::i18n::tr(
                                    "Live process inspection opened in the main window",
                                ))
                            }
                            Err(error) => {
                                this.message = None;
                                this.error = Some(error);
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(_) => {
                self.error = Some(crate::i18n::tr(
                    "Process inspection could not be queued; try again",
                ))
            }
        }
        cx.notify();
    }

    fn send(&mut self, rule: Rule, existing: bool, session: u64, cx: &mut Context<Self>) {
        let (reply, pending) = tokio::sync::oneshot::channel();
        match self.actions.try_send(OpenRule {
            node: self.detail.row.node.clone(),
            session,
            rule,
            existing,
            reply,
        }) {
            Ok(()) => {
                self.error = None;
                self.message = Some(crate::i18n::tr("Opening the rule editor for review…"));
                self.busy = true;
                cx.spawn(async move |this, cx| {
                    let result = pending.await.unwrap_or_else(|_| {
                        Err("The rule editor request stopped before opening".into())
                    });
                    let _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => {
                                this.message =
                                    Some(crate::i18n::tr("Rule editor opened for review"))
                            }
                            Err(error) => {
                                this.message = None;
                                this.error = Some(error);
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(_) => {
                self.error = Some(crate::i18n::tr(
                    "The rule editor request could not be queued; try again",
                ))
            }
        }
        cx.notify();
    }
    fn create(&mut self, kind: DraftKind, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            self.error = Some(crate::i18n::tr(
                "Connect this event's daemon and reload its rules before creating a rule",
            ));
            cx.notify();
            return;
        };
        match self.detail.draft(kind) {
            Ok(rule) => self.send(rule, false, session, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }
    fn edit_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        let Some(rule) = self
            .detail
            .matched_rule()
            .filter(|rule| !rule.name.is_empty())
        else {
            return;
        };
        let name = rule.name.clone();
        let node = self.detail.row.node.clone();
        let database = self.backend.history.database_path().to_owned();
        self.busy = true;
        self.error = None;
        self.message = None;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            crate::rules::transfer::find_confirmed_rule(&database, &node, &name)
        });
        cx.spawn_in(window,async move |this,cx|{let result=task.await;let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(Some(rule))=>this.send(rule,true,session,cx),Ok(None)=>this.error=Some(crate::i18n::tr("The matched rule is no longer in the current inventory; it may have expired or been deleted")),Err(error)=>this.error=Some(crate::i18n::tr_args("Cannot read the current rule: %{error}", &[("error", &format!("{:#}", error))]))}cx.notify();});}).detach();
    }
    fn text(&self) -> String {
        // Copy every captured value unchanged. Translate headings separately so
        // large command lines are not squeezed into bounded UI format args.
        let row = &self.detail.row;
        let mut fields = vec![
            ("Connection", row.id.to_string()),
            (
                "Time (UTC)",
                chrono::DateTime::from_timestamp_nanos(row.unixnano).to_rfc3339(),
            ),
            ("Node", row.node.clone()),
            ("Action", row.action.clone()),
            ("Reason", row.reason.clone()),
            ("Executable", row.process.clone()),
            ("Destination", endpoint(&row.destination, row.port as u32)),
            ("Protocol", row.protocol.clone()),
        ];
        if let Some(c) = self.detail.connection() {
            fields.extend([
                ("Working directory", c.process_cwd.clone()),
                ("User ID", c.user_id.to_string()),
                ("Process ID", c.process_id.to_string()),
                ("Source", endpoint(&c.src_ip, c.src_port)),
                ("Destination IP", endpoint(&c.dst_ip, c.dst_port)),
                ("Host", c.dst_host.clone()),
                ("Command line", c.process_args.join(" ")),
            ]);
        }
        if let Some(rule) = self.detail.matched_rule() {
            fields.extend([
                ("Matched rule", rule.name.clone()),
                ("Duration", rule.duration.clone()),
            ]);
        }
        let mut text = String::new();
        for (heading, value) in fields {
            text.push_str(&crate::i18n::tr(heading));
            text.push_str(": ");
            text.push_str(&value);
            text.push('\n');
        }
        if self.detail.event.is_none() {
            text.push_str(&crate::i18n::tr(
                "Detailed metadata was not recorded for this older event.\n",
            ));
        }
        if self.detail.truncated != 0 {
            text.push_str(&crate::i18n::tr("Limited fields"));
            text.push_str(": ");
            text.push_str(&self.detail.omissions().join(", "));
            text.push('\n');
        }
        text
    }
}
impl Render for ConnectionDetails {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.detail.row;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let connected = self.session().is_some();
        let process_available = self.process_target().is_some();
        let mut content = div()
            .id("connection-detail-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .flex()
            .flex_col()
            .gap_3();
        if self.tab == 0 {
            let mut fields = DescriptionList::new()
                .columns(1)
                .label_width(px(145.))
                .item(
                    crate::i18n::tr("Time (UTC)"),
                    chrono::DateTime::from_timestamp_nanos(row.unixnano).to_rfc3339(),
                    1,
                )
                .item(crate::i18n::tr("Node"), row.node.clone(), 1)
                .item(crate::i18n::tr("Action"), row.action.clone(), 1)
                .item(crate::i18n::tr("Reason"), row.reason.clone(), 1)
                .item(crate::i18n::tr("Executable"), row.process.clone(), 1);
            if let Some(c) = self.detail.connection() {
                fields = fields
                    .item(
                        crate::i18n::tr("Working directory"),
                        c.process_cwd.clone(),
                        1,
                    )
                    .item(crate::i18n::tr("User ID"), c.user_id.to_string(), 1)
                    .item(crate::i18n::tr("Process ID"), c.process_id.to_string(), 1)
                    .item(
                        crate::i18n::tr("Source"),
                        endpoint(&c.src_ip, c.src_port),
                        1,
                    )
                    .item(
                        crate::i18n::tr("Destination IP"),
                        endpoint(&c.dst_ip, c.dst_port),
                        1,
                    )
                    .item(crate::i18n::tr("Host"), c.dst_host.clone(), 1)
                    .item(crate::i18n::tr("Protocol"), c.protocol.clone(), 1);
                if let Some(event) = &self.detail.event
                    && !event.time.is_empty()
                {
                    fields = fields.item(crate::i18n::tr("Daemon time"), event.time.clone(), 1);
                }
            } else {
                fields = fields
                    .item(
                        crate::i18n::tr("Destination"),
                        endpoint(&row.destination, row.port as u32),
                        1,
                    )
                    .item(crate::i18n::tr("Protocol"), row.protocol.clone(), 1);
            }
            content = content.child(fields);
            content = content.child(div().text_sm().text_color(muted).child(crate::i18n::tr("Inspect process requests current data from this event's daemon. The process may have exited or its PID may have been reused; live data is not a historical snapshot.")
            ));
        } else if self.tab == 1 {
            if let Some(c) = self.detail.connection() {
                let mut fields = DescriptionList::new().columns(1).label_width(px(80.));
                for (index, arg) in c.process_args.iter().enumerate() {
                    fields = fields.item(
                        format!("argv[{index}]"),
                        serde_json::to_string(arg).unwrap(),
                        1,
                    );
                }
                content = content
                    .child(if c.process_args.is_empty() {
                        crate::i18n::tr("No arguments were reported")
                    } else {
                        crate::i18n::tr("Recorded arguments (quoted to preserve spaces)")
                    })
                    .child(fields)
                    .child(
                        Button::new("connection-copy-command")
                            .label(crate::i18n::tr("Copy command line"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(c) = this.detail.connection() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        c.process_args.join(" "),
                                    ));
                                }
                            })),
                    );
            } else {
                content = content.child(crate::i18n::tr(
                    "Arguments were not recorded for this older event",
                ));
            }
        } else if let Some(rule) = self.detail.matched_rule() {
            content=content.child(DescriptionList::new().columns(1).item(crate::i18n::tr("Name"),rule.name.clone(),1).item(crate::i18n::tr("Action"),rule.action.clone(),1).item(crate::i18n::tr("Duration"),rule.duration.clone(),1).item(crate::i18n::tr("Enabled"),rule.enabled.to_string(),1).item(crate::i18n::tr("Priority"),rule.precedence.to_string(),1).item(crate::i18n::tr("Logging"),(!rule.nolog).to_string(),1))
                .child(div().text_sm().text_color(muted).child(crate::i18n::tr("This is the rule reported with the event. Open the current rule to inspect or edit its current conditions.")))
                .child(Button::new("connection-edit-rule").label(crate::i18n::tr("Open current rule…")).disabled(!connected||self.busy||rule.name.is_empty()||self.detail.truncated&crate::history::details::MATCHED_RULE!=0).on_click(cx.listener(|this,_,window,cx|this.edit_current(window,cx))));
        } else {
            content = content.child(crate::i18n::tr("No matched rule snapshot was recorded"));
        }
        let can_create = connected
            && !self.busy
            && self
                .detail
                .connection()
                .is_some_and(|c| !c.process_path.is_empty())
            && self.detail.truncated & crate::history::details::PATH == 0;
        div().size_full().flex().flex_col().track_focus(&self.focus).bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().px_5().py_3().border_b_1().border_color(border).flex().items_center().justify_between().child(crate::i18n::tr_args("Connection %{arg0} · %{arg1}", &[("arg0", &(row.id).to_string()), ("arg1", &(row.action).to_string())])).child(Button::new("connection-copy").label(crate::i18n::tr("Copy details")).on_click(cx.listener(|this,_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(this.text()))))))
            .child(TabBar::new("connection-tabs").selected_index(self.tab).on_click(cx.listener(|this,index:&usize,_,cx|{this.tab = *index;cx.notify();})).child(Tab::new().label(crate::i18n::tr("Overview"))).child(Tab::new().label(crate::i18n::tr("Arguments"))).child(Tab::new().label(crate::i18n::tr("Matched rule"))))
            .when(self.detail.event.is_none(),|v|v.child(super::banner("This event predates detailed history; only its summary is available.".into(),cx)))
            .when(self.detail.truncated!=0,|v|v.child(super::banner(crate::i18n::tr_args("Recording limits shortened: %{arg0}. Rule creation requires complete match fields.", &[("arg0", &self.detail.omissions().join(", ").to_string())]),cx)))
            .when_some(self.error.clone(),|v,error|v.child(super::banner(error,cx)))
            .when_some(self.message.clone(),|v,message|v.child(div().px_5().py_2().text_sm().child(crate::i18n::presentation(&message))))
            .child(content)
            .child(div().px_5().py_3().border_t_1().border_color(border).flex().flex_wrap().gap_2()
                .child(Button::new("connection-create-application").label(crate::i18n::tr("Application rule…")).disabled(!can_create).on_click(cx.listener(|this,_,_,cx|this.create(DraftKind::Application,cx))))
                .child(Button::new("connection-create-destination").label(crate::i18n::tr("Application + destination…")).disabled(!can_create||self.detail.truncated&crate::history::details::DESTINATION!=0).on_click(cx.listener(|this,_,_,cx|this.create(DraftKind::Destination,cx))))
                .child(Button::new("connection-inspect-process").label(crate::i18n::tr("Inspect process…")).disabled(!process_available||self.busy).on_click(cx.listener(|this,_,_,cx|this.inspect_process(cx))))
                .child(Button::new("connection-close").label(crate::i18n::tr("Close")).on_click(cx.listener(|_,_,window,_|window.remove_window()))))
    }
}
