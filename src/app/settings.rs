//! One native settings window. Feature entities retain their drafts while pages change.
use crate::{backend::Handle, config::Config};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Collapsible, Disableable,
    button::{Button, ButtonVariants},
    sidebar::{Sidebar, SidebarGroup, SidebarItem, SidebarMenuItem},
};
use std::collections::HashMap;

/// All settings pages use the same fixed footer, with Save only for editable pages.
pub(super) fn footer(
    close_id: &'static str,
    busy: bool,
    save: impl Into<Option<Button>>,
    cx: &App,
) -> impl IntoElement {
    let footer = div()
        .id("settings-footer")
        .w_full()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_end()
        .gap_2()
        .px_5()
        .py_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            Button::new(close_id)
                .label(crate::i18n::tr("Close"))
                .w_24()
                .disabled(busy)
                .on_click(|_, window, _| window.remove_window()),
        )
        .when_some(save.into(), |footer, save| {
            footer.child(save.label(crate::i18n::tr("Save")).primary().w_24())
        });
    #[cfg(feature = "headless-tests")]
    let footer = footer.test_support();
    footer
}

pub(super) fn heading(title: &'static str) -> Div {
    div()
        .px_5()
        .py_4()
        .flex_shrink_0()
        .text_xl()
        .child(crate::i18n::tr(title))
}

actions!(
    rustleblower_settings,
    [CloseSettings, NextCategory, PreviousCategory]
);
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-w", CloseSettings, Some("RustleblowerSettings")),
        KeyBinding::new("ctrl-tab", NextCategory, Some("RustleblowerSettings")),
        KeyBinding::new(
            "ctrl-shift-tab",
            PreviousCategory,
            Some("RustleblowerSettings"),
        ),
    ]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum Page {
    Appearance,
    Popup,
    Startup,
    Updates,
    History,
    Catalogue,
    Reputation,
    Downloads,
    Highlighting,
}
impl Page {
    const ALL: [Self; 9] = [
        Self::Appearance,
        Self::Startup,
        Self::Updates,
        Self::Popup,
        Self::Reputation,
        Self::History,
        Self::Catalogue,
        Self::Downloads,
        Self::Highlighting,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Popup => "Popup preferences",
            Self::Startup => "Startup",
            Self::Updates => "Updates",
            Self::History => "History",
            Self::Catalogue => "Rule catalogue",
            Self::Reputation => "Reputation",
            Self::Downloads => "Raw downloads",
            Self::Highlighting => "Table highlighting",
        }
    }
    fn icon(self) -> IconName {
        match self {
            Self::Appearance => IconName::Palette,
            Self::Popup => IconName::Bell,
            Self::Startup => IconName::Play,
            Self::Updates => IconName::RefreshCw,
            Self::History => IconName::HardDrive,
            Self::Catalogue => IconName::FileText,
            Self::Reputation => IconName::Globe,
            Self::Downloads => IconName::Folder,
            Self::Highlighting => IconName::Eye,
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Appearance => "settings-appearance",
            Self::Popup => "settings-popup",
            Self::Startup => "settings-startup",
            Self::Updates => "settings-updates",
            Self::History => "settings-history",
            Self::Catalogue => "settings-catalogue",
            Self::Reputation => "settings-reputation",
            Self::Downloads => "settings-downloads",
            Self::Highlighting => "settings-highlighting",
        }
    }
}

#[derive(Clone)]
struct OpenSettings {
    window: AnyWindowHandle,
    view: WeakEntity<Settings>,
}
impl Global for OpenSettings {}

#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_window(cx: &App) -> AnyWindowHandle {
    cx.global::<OpenSettings>().window
}

// Keep navigation IDs independent of group positions and translated labels.
#[derive(Clone)]
struct MenuItem(Page, SidebarMenuItem);
impl Collapsible for MenuItem {
    fn is_collapsed(&self) -> bool {
        self.1.is_collapsed()
    }
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.1 = self.1.collapsed(collapsed);
        self
    }
}
impl SidebarItem for MenuItem {
    fn render(
        self,
        _: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        self.1.render(self.0.id(), window, cx)
    }
}

pub(super) fn open(
    backend: Handle,
    config: Config,
    page: Page,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    if let Some(open) = cx.try_global::<OpenSettings>().cloned()
        && open
            .window
            .update(cx, |_, window, cx| {
                open.view
                    .update(cx, |view, cx| view.select(page, window, cx))?;
                window.activate_window();
                anyhow::Ok(())
            })
            .is_ok_and(|result| result.is_ok())
    {
        return Ok(open.window);
    }
    super::appearance::init(&config.database, cx);
    let (window, view) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1180.), px(820.)),
                cx,
            ))),
            window_min_size: Some(size(px(900.), px(620.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Settings").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| {
                let mut view = Settings {
                    backend,
                    config,
                    page,
                    pages: HashMap::new(),
                    focus: cx.focus_handle(),
                };
                view.select(page, window, cx);
                view
            })
        },
    )?;
    crate::i18n::register_title(window, "Settings", cx);
    cx.set_global(OpenSettings {
        window,
        view: view.downgrade(),
    });
    Ok(window)
}

pub(super) struct Settings {
    backend: Handle,
    config: Config,
    page: Page,
    // At most nine lazily created pages, owned until this window closes.
    pages: HashMap<Page, AnyView>,
    focus: FocusHandle,
}
impl Settings {
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_reputation_result(&self, cx: &App) -> Option<crate::reputation::ResultSet> {
        self.pages
            .get(&Page::Reputation)?
            .clone()
            .downcast::<super::reputation_view::View>()
            .ok()?
            .read(cx)
            .test_result()
    }
    fn cycle(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let index = Page::ALL
            .iter()
            .position(|page| *page == self.page)
            .unwrap_or(0);
        let next = if backwards {
            index + Page::ALL.len() - 1
        } else {
            index + 1
        };
        self.select(Page::ALL[next % Page::ALL.len()], window, cx);
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_startup_state(&self, cx: &App) -> (bool, bool, bool) {
        self.pages[&Page::Startup]
            .clone()
            .downcast::<super::startup_view::Startup>()
            .unwrap()
            .read(cx)
            .test_state()
    }
    fn select(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        // Clear focus from a hidden page before mounting the next one. A newly
        // created feature can then focus its own initial control.
        self.focus.focus(window, cx);
        self.pages.entry(page).or_insert_with(|| match page {
            Page::Appearance => cx.new(super::appearance::Appearance::new).into(),
            Page::Popup => cx
                .new(|cx| {
                    super::preferences_view::Preferences::new(self.backend.clone(), window, cx)
                })
                .into(),
            Page::Startup => cx
                .new(|cx| super::startup_view::Startup::new(self.config.clone(), window, cx))
                .into(),
            Page::Updates => cx
                .new(|cx| super::updates_view::View::new(self.backend.updates.clone(), window, cx))
                .into(),
            Page::History => cx
                .new(|cx| {
                    super::history_preferences::Preferences::new(
                        self.backend.history.clone(),
                        window,
                        cx,
                    )
                })
                .into(),
            Page::Catalogue => cx
                .new(|cx| {
                    super::catalogue_preferences::View::new(
                        self.backend.catalogue_policy.clone(),
                        window,
                        cx,
                    )
                })
                .into(),
            Page::Reputation => cx
                .new(|cx| {
                    super::reputation_view::View::new(
                        self.backend.reputation.clone(),
                        None,
                        window,
                        cx,
                    )
                })
                .into(),
            Page::Downloads => cx
                .new(|cx| {
                    super::raw_downloads_view::View::new(
                        self.backend.raw_downloads.clone(),
                        window,
                        cx,
                    )
                })
                .into(),
            Page::Highlighting => cx
                .new(|_| Highlighting(self.backend.highlighting.clone()))
                .into(),
        });
        self.page = page;
        cx.notify();
    }
    fn group(
        &self,
        label: &'static str,
        pages: &[Page],
        cx: &mut Context<Self>,
    ) -> SidebarGroup<MenuItem> {
        SidebarGroup::new(crate::i18n::tr(label)).children(
            pages
                .iter()
                .copied()
                .map(|page| {
                    MenuItem(
                        page,
                        SidebarMenuItem::new(crate::i18n::tr(page.label()))
                            .icon(page.icon())
                            .active(self.page == page)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(page, window, cx)
                            })),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }
}
impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = Sidebar::new("settings-sidebar")
            .w(rems(16.))
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .header(
                div()
                    .px_4()
                    .py_5()
                    .text_xl()
                    .child(crate::i18n::tr("Settings")),
            )
            .child(self.group(
                "General",
                &[Page::Appearance, Page::Startup, Page::Updates],
                cx,
            ))
            .child(self.group("Permissions", &[Page::Popup, Page::Reputation], cx))
            .child(self.group(
                "Data",
                &[
                    Page::History,
                    Page::Catalogue,
                    Page::Downloads,
                    Page::Highlighting,
                ],
                cx,
            ));
        let content = self.pages[&self.page].clone();
        div()
            .id("settings-window")
            .size_full()
            .flex()
            .track_focus(&self.focus)
            .key_context("RustleblowerSettings")
            .on_action(|_: &CloseSettings, window, _| window.remove_window())
            .on_action(
                cx.listener(|this, _: &NextCategory, window, cx| this.cycle(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousCategory, window, cx| this.cycle(true, window, cx)),
            )
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sidebar)
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .overflow_hidden()
                    .child(content),
            )
    }
}

struct Highlighting(crate::highlighting::Handle);
impl Render for Highlighting {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col()
            .child(heading("Table highlighting"))
            .child(div().flex_1().min_h_0().px_5().pb_5().flex().flex_col().gap_4()
                .child(div().text_sm().child(crate::i18n::tr("Table styles are configured in a file. Reload applies the saved styles to open tables.")))
                .child(Button::new("highlighting-reload").label(crate::i18n::tr("Reload table styles"))
                    .disabled(super::highlighting::pending(cx))
                    .on_click(cx.listener(|this,_,_,cx|super::highlighting::reload(&this.0,cx))))
                .when_some(super::highlighting::message(cx), |view,message| view.child(crate::i18n::presentation(&message))))
            .child(footer("settings-close", false, None, cx))
    }
}
