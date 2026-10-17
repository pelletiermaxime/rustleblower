//! Reviewed socket columns/preferences and a captured, off-thread export.
use crate::live::{
    presentation::{self, Command, Settings},
    table::{Export, Format},
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectState},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone, Copy)]
struct OpenWindow(AnyWindowHandle);
impl Global for OpenWindow {}
pub(super) fn open(
    handle: presentation::Handle,
    settings: Settings,
    revision: Option<String>,
    export: Option<Export>,
    database: PathBuf,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    if let Some(previous) = cx.try_global::<OpenWindow>().copied() {
        if export.is_none()
            && previous
                .0
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return Ok(previous.0);
        }
        let _ = previous.0.update(cx, |_, window, _| window.remove_window());
    }
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(850.), px(740.)),
                cx,
            ))),
            window_min_size: Some(size(px(680.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — live table tools").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| Tools::new(handle, settings, revision, export, database, window, cx))
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — live table tools", cx);
    cx.set_global(OpenWindow(window));
    Ok(window)
}
struct Tools {
    handle: presentation::Handle,
    settings: Settings,
    revision: Option<String>,
    visible: [bool; 13],
    widths: Vec<Entity<InputState>>,
    export: Option<Export>,
    database: PathBuf,
    path: Entity<InputState>,
    format: Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<Format>>>>,
    replace: bool,
    busy: bool,
    stop: Arc<AtomicBool>,
    message: Option<String>,
}
impl Tools {
    fn new(
        handle: presentation::Handle,
        settings: Settings,
        revision: Option<String>,
        export: Option<Export>,
        database: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let visible = std::array::from_fn(|index| {
            settings
                .columns
                .iter()
                .any(|column| column.field == presentation::FIELDS[index])
        });
        let widths = presentation::FIELDS
            .iter()
            .map(|field| {
                let width = settings
                    .columns
                    .iter()
                    .find(|column| column.field == *field)
                    .map_or(field.width(), |column| column.width);
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(width.to_string())
                        .validate(|text, _| {
                            text.len() <= 4 && text.bytes().all(|b| b.is_ascii_digit())
                        })
                })
            })
            .collect();
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Absolute export file path",
                    window,
                    cx,
                ))
                .validate(|text, _| text.len() <= 4096 && !text.contains('\0'))
        });
        let format = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    crate::i18n::LocalizedOption::new(Format::Csv, "CSV"),
                    crate::i18n::LocalizedOption::new(Format::Json, "Socket JSON"),
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        Self {
            handle,
            settings,
            revision,
            visible,
            widths,
            export,
            database,
            path,
            format,
            replace: false,
            busy: false,
            stop: Arc::new(AtomicBool::new(false)),
            message: None,
        }
    }
    fn candidate(&self, cx: &App) -> anyhow::Result<Settings> {
        let mut settings = self.settings.clone();
        settings.columns = presentation::FIELDS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.visible[*i])
            .map(|(index, field)| {
                Ok(presentation::Column {
                    field: *field,
                    width: self.widths[index].read(cx).value().parse()?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        settings.validate()?;
        Ok(settings)
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let settings = match self.candidate(cx) {
            Ok(settings) => settings,
            Err(_) => {
                self.message = Some(crate::i18n::tr(
                    "Choose at least one column and widths from 40 to 2000 pixels.",
                ));
                cx.notify();
                return;
            }
        };
        self.busy = true;
        let reply = self.handle.request(Command::Save {
            expected_revision: self.revision.clone(),
            settings,
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = reply.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(saved)) => {
                        this.revision = saved.revision;
                        this.message = Some(crate::i18n::tr(
                            "Live view saved. Restoring it leaves monitoring stopped.",
                        ));
                    }
                    _ => {
                        this.message = Some(crate::i18n::tr(
                            "Live view save refused; reload preferences and reopen these tools.",
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn output(&mut self, copy: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(mut export) = self.export.clone() else {
            return;
        };
        let settings = match self.candidate(cx) {
            Ok(settings) => settings,
            Err(_) => {
                self.message = Some(crate::i18n::tr(
                    "Choose at least one column and widths from 40 to 2000 pixels.",
                ));
                cx.notify();
                return;
            }
        };
        export.columns = settings.columns.iter().map(|column| column.field).collect();
        let format = self
            .format
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Format::Csv);
        let database = self.database.clone();
        let target = PathBuf::from(self.path.read(cx).value().as_str());
        let replace = self.replace;
        self.busy = true;
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let task = cx.background_executor().spawn(async move {
            if copy {
                export.copy(format, &stop).map(Some)
            } else {
                export
                    .save(&database, &target, replace, format, &stop)
                    .map(|_| None)
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(Some(text)) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.message = Some(crate::i18n::tr("Captured socket rows copied."));
                    }
                    Ok(None) => {
                        this.message = Some(crate::i18n::tr("Captured socket rows exported."))
                    }
                    Err(error) => {
                        this.message = Some(crate::i18n::tr_args(
                            "Socket output refused: %{error}",
                            &[("error", &crate::history::bounded(&error.to_string(), 1024))],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Drop for Tools {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Render for Tools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content=div().id("live-tools-scroll").flex_1().min_h_0().overflow_y_scroll().p_4().flex().flex_col().gap_3().child(div().text_sm().child(crate::i18n::tr("Columns and widths apply to the saved view. Protocol, family, state, interval, search and sort are captured from Live. Saved views never start monitoring automatically.")));
        #[cfg(feature = "headless-tests")]
        let mut content = content.test_support();
        #[cfg(not(feature = "headless-tests"))]
        let mut content = content;
        for (index, field) in presentation::FIELDS.into_iter().enumerate() {
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .flex_none()
                    .gap_3()
                    .child(
                        Checkbox::new(("live-column", index as u64))
                            .label(crate::i18n::tr(field.label()))
                            .checked(self.visible[index])
                            .disabled(self.busy)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                this.visible[index] = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Input::new(&self.widths[index])
                            .id(("live-width", index as u64))
                            .w(px(100.))
                            .disabled(self.busy),
                    ),
            );
        }
        content = content.child(
            Button::new("live-view-save")
                .label(crate::i18n::tr("Save live view"))
                .disabled(self.busy)
                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
        );
        if let Some(export) = &self.export {
            content=content.child(div().text_sm().child(crate::i18n::tr_args("Frozen socket snapshot: session %{session}, revision %{revision}, %{rows} rows. Incoming updates cannot change this output.",&[("session",&export.capture.session.to_string()),("revision",&export.capture.revision.to_string()),("rows",&export.order.len().to_string())]))).child(div().h_8().flex_none().child(Select::new(&self.format).id("live-output-format").disabled(self.busy))).child(div().text_sm().child(crate::i18n::tr("CSV uses the chosen columns. Socket JSON keeps the original socket objects."))).child(Input::new(&self.path).id("live-output-path").disabled(self.busy)).child(Checkbox::new("live-output-replace").label(crate::i18n::tr("Allow replacing this export file")).checked(self.replace).disabled(self.busy).on_change(cx.listener(|this,checked,_,cx|{this.replace = *checked;cx.notify();}))).child(div().flex().gap_2().child(Button::new("live-output-copy").label(crate::i18n::tr("Copy captured rows")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.output(true,window,cx)))).child(Button::new("live-output-save").label(crate::i18n::tr("Export captured rows")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.output(false,window,cx)))).child(Button::new("live-output-cancel").label(crate::i18n::tr("Cancel output")).disabled(!self.busy).on_click(cx.listener(|this,_,_,cx|{this.stop.store(true,Ordering::Release);cx.notify();}))));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().tokens.background)
            .text_color(cx.theme().tokens.foreground)
            .child(content)
            .when_some(self.message.clone(), |view, message| {
                view.child(super::banner(message, cx))
            })
            .child(
                div().p_3().child(
                    Button::new("live-tools-close")
                        .label(crate::i18n::tr("Close"))
                        .on_click(|_, window, _| window.remove_window()),
                ),
            )
    }
}
