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

pub(super) fn open(
    backend: Handle,
    detail: Detail,
    actions: SyncSender<OpenRule>,
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
                title: Some("Rustleblower — connection details".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| ConnectionDetails::new(backend, detail, actions, window, cx)),
    )?;
    Ok(window)
}

struct ConnectionDetails {
    backend: Handle,
    detail: Detail,
    actions: SyncSender<OpenRule>,
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
                self.message = Some("Opening the rule editor for review…".into());
                self.busy = true;
                cx.spawn(async move |this, cx| {
                    let result = pending.await.unwrap_or_else(|_| {
                        Err("The rule editor request stopped before opening".into())
                    });
                    let _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        match result {
                            Ok(()) => this.message = Some("Rule editor opened for review".into()),
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
                self.error = Some("The rule editor request could not be queued; try again".into())
            }
        }
        cx.notify();
    }
    fn create(&mut self, kind: DraftKind, cx: &mut Context<Self>) {
        let Some(session) = self.session() else {
            self.error = Some(
                "Connect this event's daemon and reload its rules before creating a rule".into(),
            );
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
        cx.spawn_in(window,async move |this,cx|{let result=task.await;let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(Some(rule))=>this.send(rule,true,session,cx),Ok(None)=>this.error=Some("The matched rule is no longer in the current inventory; it may have expired or been deleted".into()),Err(error)=>this.error=Some(format!("Cannot read the current rule: {error:#}"))}cx.notify();});}).detach();
    }
    fn text(&self) -> String {
        let row = &self.detail.row;
        let mut text = format!(
            "Connection {}\nTime (UTC): {}\nNode: {}\nAction: {}\nReason: {}\nExecutable: {}\nDestination: {}\nProtocol: {}\n",
            row.id,
            chrono::DateTime::from_timestamp_nanos(row.unixnano).to_rfc3339(),
            row.node,
            row.action,
            row.reason,
            row.process,
            endpoint(&row.destination, row.port as u32),
            row.protocol
        );
        if let Some(c) = self.detail.connection() {
            text.push_str(&format!("Working directory: {}\nUID: {}\nPID: {}\nSource: {}\nDestination IP: {}\nHost: {}\nCommand line: {}\n",c.process_cwd,c.user_id,c.process_id,endpoint(&c.src_ip,c.src_port),endpoint(&c.dst_ip,c.dst_port),c.dst_host,c.process_args.join(" ")));
        }
        if let Some(rule) = self.detail.matched_rule() {
            text.push_str(&format!(
                "Matched rule: {}\nDuration: {}\n",
                rule.name, rule.duration
            ));
        }
        if self.detail.event.is_none() {
            text.push_str("Detailed metadata was not recorded for this older event.\n");
        }
        if self.detail.truncated != 0 {
            text.push_str(&format!(
                "Limited fields: {}\n",
                self.detail.omissions().join(", ")
            ));
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
                    "Time (UTC)",
                    chrono::DateTime::from_timestamp_nanos(row.unixnano).to_rfc3339(),
                    1,
                )
                .item("Node", row.node.clone(), 1)
                .item("Action", row.action.clone(), 1)
                .item("Reason", row.reason.clone(), 1)
                .item("Executable", row.process.clone(), 1);
            if let Some(c) = self.detail.connection() {
                fields = fields
                    .item("Working directory", c.process_cwd.clone(), 1)
                    .item("User ID", c.user_id.to_string(), 1)
                    .item("Process ID", c.process_id.to_string(), 1)
                    .item("Source", endpoint(&c.src_ip, c.src_port), 1)
                    .item("Destination IP", endpoint(&c.dst_ip, c.dst_port), 1)
                    .item("Host", c.dst_host.clone(), 1)
                    .item("Protocol", c.protocol.clone(), 1);
                if let Some(event) = &self.detail.event
                    && !event.time.is_empty()
                {
                    fields = fields.item("Daemon time", event.time.clone(), 1);
                }
            } else {
                fields = fields
                    .item(
                        "Destination",
                        endpoint(&row.destination, row.port as u32),
                        1,
                    )
                    .item("Protocol", row.protocol.clone(), 1);
            }
            content = content.child(fields);
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
                        "No arguments were reported"
                    } else {
                        "Recorded arguments (quoted to preserve spaces)"
                    })
                    .child(fields)
                    .child(
                        Button::new("connection-copy-command")
                            .label("Copy command line")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(c) = this.detail.connection() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        c.process_args.join(" "),
                                    ));
                                }
                            })),
                    );
            } else {
                content = content.child("Arguments were not recorded for this older event");
            }
        } else if let Some(rule) = self.detail.matched_rule() {
            content=content.child(DescriptionList::new().columns(1).item("Name",rule.name.clone(),1).item("Action",rule.action.clone(),1).item("Duration",rule.duration.clone(),1).item("Enabled",rule.enabled.to_string(),1).item("Priority",rule.precedence.to_string(),1).item("Logging",(!rule.nolog).to_string(),1))
                .child(div().text_sm().text_color(muted).child("This is the rule reported with the event. Open the current rule to inspect or edit its current conditions."))
                .child(Button::new("connection-edit-rule").label("Open current rule…").disabled(!connected||self.busy||rule.name.is_empty()||self.detail.truncated&crate::history::details::MATCHED_RULE!=0).on_click(cx.listener(|this,_,window,cx|this.edit_current(window,cx))));
        } else {
            content = content.child("No matched rule snapshot was recorded");
        }
        let can_create = connected
            && !self.busy
            && self
                .detail
                .connection()
                .is_some_and(|c| !c.process_path.is_empty())
            && self.detail.truncated & crate::history::details::PATH == 0;
        div().size_full().flex().flex_col().track_focus(&self.focus).bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().px_5().py_3().border_b_1().border_color(border).flex().items_center().justify_between().child(format!("Connection {} · {}",row.id,row.action)).child(Button::new("connection-copy").label("Copy details").on_click(cx.listener(|this,_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(this.text()))))))
            .child(TabBar::new("connection-tabs").selected_index(self.tab).on_click(cx.listener(|this,index:&usize,_,cx|{this.tab = *index;cx.notify();})).child(Tab::new().label("Overview")).child(Tab::new().label("Arguments")).child(Tab::new().label("Matched rule")))
            .when(self.detail.event.is_none(),|v|v.child(super::banner("This event predates detailed history; only its summary is available.".into(),cx)))
            .when(self.detail.truncated!=0,|v|v.child(super::banner(format!("Recording limits shortened: {}. Rule creation requires complete match fields.",self.detail.omissions().join(", ")),cx)))
            .when_some(self.error.clone(),|v,error|v.child(super::banner(error,cx)))
            .when_some(self.message.clone(),|v,message|v.child(div().px_5().py_2().text_sm().child(message)))
            .child(content)
            .child(div().px_5().py_3().border_t_1().border_color(border).flex().flex_wrap().gap_2()
                .child(Button::new("connection-create-application").label("Application rule…").disabled(!can_create).on_click(cx.listener(|this,_,_,cx|this.create(DraftKind::Application,cx))))
                .child(Button::new("connection-create-destination").label("Application + destination…").disabled(!can_create||self.detail.truncated&crate::history::details::DESTINATION!=0).on_click(cx.listener(|this,_,_,cx|this.create(DraftKind::Destination,cx))))
                .child(Button::new("connection-close").label("Close").on_click(cx.listener(|_,_,window,_|window.remove_window()))))
    }
}
