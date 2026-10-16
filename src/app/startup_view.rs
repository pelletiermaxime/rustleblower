//! Staged startup controls; filesystem discovery and saves never run on GPUI.
use crate::{
    config::Config,
    startup::{Launch, Manager, Paths, Snapshot},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{ActiveTheme, Disableable, button::Button, checkbox::Checkbox};
use std::sync::Arc;
pub(super) fn open(config: Config, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    let (w, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(780.), px(580.)),
                cx,
            ))),
            window_min_size: Some(size(px(640.), px(480.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — startup preferences".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| {
                let mut v = Startup {
                    config,
                    manager: None,
                    snapshot: None,
                    enabled: false,
                    busy: false,
                    error: None,
                    message: String::new(),
                    focus: cx.focus_handle(),
                };
                v.reload(window, cx);
                v
            })
        },
    )?;
    Ok(w)
}
pub(super) struct Startup {
    config: Config,
    manager: Option<Arc<Manager>>,
    snapshot: Option<Snapshot>,
    enabled: bool,
    busy: bool,
    error: Option<String>,
    message: String,
    focus: FocusHandle,
}
impl Startup {
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let config = self.config.clone();
        let task = cx.background_executor().spawn(async move {
            let manager = Manager {
                paths: Paths::environment()?,
                launch: Launch::environment(&config)?,
            };
            let snapshot = manager.load()?;
            anyhow::Ok((manager, snapshot))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((manager, snapshot)) => {
                        this.enabled = snapshot.enabled;
                        this.manager = Some(Arc::new(manager));
                        this.snapshot = Some(snapshot);
                        this.message =
                            "Loaded current startup entry. Changes apply when saved.".into();
                    }
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (Some(manager), Some(snapshot)) = (self.manager.clone(), self.snapshot.clone()) else {
            return;
        };
        self.busy = true;
        self.error = None;
        let enabled = self.enabled;
        let task = cx
            .background_executor()
            .spawn(async move { manager.save(&snapshot, enabled) });
        cx.spawn_in(window,async move|this,cx|{let result=task.await;let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(snapshot)=>{this.snapshot=Some(snapshot);this.message=if enabled{"Saved: Rustleblower will start at login."}else{"Saved: automatic startup is disabled."}.into();},Err(e)=>{this.error=Some(format!("{e:#}"));this.message="Startup change was not confirmed. Your selection is retained; reload to inspect the current entry.".into();}}cx.notify();});}).detach();
        cx.notify();
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_state(&self) -> (bool, bool, bool) {
        (self.busy, self.enabled, self.error.is_some())
    }
}
impl Render for Startup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let disabled = self.busy || self.snapshot.is_none();
        div().size_full().flex().flex_col().track_focus(&self.focus).bg(cx.theme().background).text_color(cx.theme().foreground)
  .child(div().px_5().py_3().border_b_1().border_color(border).text_lg().child("Startup preferences"))
  .child(div().id("startup-scroll").flex_1().min_h_0().overflow_y_scroll().px_5().py_3().flex().flex_col().gap_3()
   .child(Checkbox::new("startup-enabled").label("Start Rustleblower when I log in").checked(self.enabled).disabled(disabled).on_change(cx.listener(|this,value:&bool,_,cx|{this.enabled = *value;cx.notify();})))
   .child("The desktop starts this copy of Rustleblower with its current socket, history location and permission fallback. Saved retention settings continue to apply.")
   .when_some(self.snapshot.as_ref(),|v,s|v.child(format!("Current entry: {}",s.origin.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"No startup entry".into()))))
   .when_some(self.manager.as_ref(),|v,m|v.child(format!("User entry: {}",m.paths.user.display())).child(format!("Application: {}",m.launch.executable.display())))
   .child("Save applies the selection. Reload reads the current entry again and discards the staged selection.")
   .when_some(self.error.clone(),|v,e|v.child(super::banner(e,cx)))
   .child(self.message.clone()))
  .child(div().px_5().py_3().border_t_1().border_color(border).flex().gap_2()
   .child(Button::new("startup-close").label("Close").on_click(|_,window,_|window.remove_window()))
   .child(Button::new("startup-reload").label("Reload").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.reload(window,cx))))
   .child(Button::new("startup-save").label("Save startup preference").disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.save(window,cx)))))
    }
}
