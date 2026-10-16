//! Native Kit appearance state/settings, following FerryDock's Theme::change and
//! SettingPage pattern. Persistence always runs in background.
use crate::appearance::{ColorMode, Snapshot, Store};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Theme, ThemeMode,
    button::Button,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use std::path::Path;
struct State {
    store: Store,
    snapshot: Option<Snapshot>,
    mode: ColorMode,
    native: ThemeMode,
    busy: bool,
    message: Option<String>,
}
impl Global for State {}
fn apply(cx: &mut App) {
    let state = cx.global::<State>();
    let mode = match state.mode {
        ColorMode::System => state.native,
        ColorMode::Light => ThemeMode::Light,
        ColorMode::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, None, cx);
}
fn system(appearance: WindowAppearance, cx: &mut App) {
    if !cx.has_global::<State>() {
        return;
    }
    let native = ThemeMode::from(appearance);
    let state = cx.global_mut::<State>();
    let changed = state.native != native;
    state.native = native;
    if changed && state.mode == ColorMode::System {
        apply(cx);
    }
}
struct Monitor {
    _subscription: Subscription,
}
impl gpui_kit::base::RootPlugin for Monitor {}
impl Render for Monitor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
pub(super) fn init(database: &Path, cx: &mut App) {
    if cx.has_global::<State>() {
        return;
    }
    cx.set_global(State {
        store: Store::new(database),
        snapshot: None,
        mode: ColorMode::System,
        native: ThemeMode::from(cx.window_appearance()),
        busy: false,
        message: None,
    });
    gpui_kit::base::Root::register_plugin::<Monitor>(cx, |window, cx| {
        let appearance = window.appearance();
        cx.defer(move |cx| system(appearance, cx));
        Monitor {
            _subscription: window
                .observe_window_appearance(|window, cx| system(window.appearance(), cx)),
        }
    });
    apply(cx);
    reload(cx);
}
fn reload(cx: &mut App) {
    let s = cx.global_mut::<State>();
    if s.busy {
        return;
    }
    s.busy = true;
    s.message = Some("Loading appearance…".into());
    let store = s.store.clone();
    let task = cx.background_executor().spawn(async move { store.load() });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            state.busy = false;
            match result {
                Ok(snapshot) => {
                    state.mode = snapshot.mode;
                    state.message = snapshot.warning.clone();
                    state.snapshot = Some(snapshot);
                }
                Err(e) => {
                    state.message = Some(format!("Cannot load appearance preferences: {e:#}"));
                }
            }
            apply(cx);
            cx.refresh_windows();
        });
    })
    .detach();
    cx.refresh_windows();
}
fn choose(mode: ColorMode, cx: &mut App) {
    let state = cx.global_mut::<State>();
    if state.busy {
        return;
    }
    let Some(snapshot) = state.snapshot.clone() else {
        return;
    };
    let store = state.store.clone();
    state.mode = mode;
    state.busy = true;
    state.message = Some("Saving appearance…".into());
    apply(cx);
    let task = cx
        .background_executor()
        .spawn(async move { store.save(&snapshot, mode) });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            state.busy = false;
            match result {
                Ok(snapshot) => {
                    state.snapshot = Some(snapshot);
                    state.message = Some("Appearance saved.".into());
                }
                Err(e) => {
                    state.message = Some(format!(
                        "The color mode changed for this session but could not be saved: {e:#}"
                    ))
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
}
pub(super) fn open(database: &Path, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    init(database, cx);
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(620.)),
                cx,
            ))),
            window_min_size: Some(size(px(700.), px(500.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — appearance".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        |_, cx| cx.new(|_| Appearance),
    )?;
    Ok(window)
}
struct Appearance;
impl Render for Appearance {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.global::<State>();
        let message = state.message.clone();
        let busy = state.busy;
        let border = cx.theme().border;
        let color = SettingItem::new(
            "Color mode",
            SettingField::render(|_, _, cx| {
                let state = cx.global::<State>();
                let disabled = state.busy || state.snapshot.is_none();
                let selected = state.mode;
                div().flex().flex_wrap().gap_2().children(
                    [ColorMode::System, ColorMode::Light, ColorMode::Dark].map(|mode| {
                        Button::new(match mode {
                            ColorMode::System => "appearance-system",
                            ColorMode::Light => "appearance-light",
                            ColorMode::Dark => "appearance-dark",
                        })
                        .label(mode.label())
                        .selected(selected == mode)
                        .disabled(disabled)
                        .on_click(move |_, _, cx| choose(mode, cx))
                    }),
                )
            }),
        )
        .description("System follows the desktop's light or dark appearance.");
        let reset = SettingItem::new(
            "Default",
            SettingField::render(|_, _, cx| {
                Button::new("appearance-reset")
                    .label("Use System appearance")
                    .disabled(cx.global::<State>().busy || cx.global::<State>().snapshot.is_none())
                    .on_click(|_, _, cx| choose(ColorMode::System, cx))
            }),
        );
        let settings = Settings::new("appearance-settings").sidebar_width(px(170.)).pages(vec![
            SettingPage::new("Appearance").default_open(true).group(
                SettingGroup::new().title("Color theme")
                    .description("Changes apply immediately to every Rustleblower window. Your current page and edits stay open.")
                    .item(color).item(reset)
            )
        ]);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .px_5()
                    .py_3()
                    .text_lg()
                    .border_b_1()
                    .border_color(border)
                    .child("Appearance"),
            )
            .child(div().flex_1().min_h_0().child(settings))
            .when_some(message, |view, message| {
                view.child(div().px_5().py_2().text_sm().child(message))
            })
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(border)
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("appearance-close")
                            .label("Close")
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("appearance-reload")
                            .label("Reload saved appearance")
                            .disabled(busy)
                            .on_click(|_, _, cx| reload(cx)),
                    ),
            )
    }
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_state(cx: &App) -> (bool, ColorMode) {
    let s = cx.global::<State>();
    (s.busy, s.mode)
}
