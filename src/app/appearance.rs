//! Native Kit appearance state/settings, following FerryDock's Theme::change and
//! SettingPage pattern. Persistence always runs in background.
use crate::appearance::{Catalogue, ColorMode, DensityScale, PaletteId, Snapshot, Store};
use crate::i18n::{self, LanguageChoice};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Theme, ThemeMode,
    button::Button,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};
use std::path::Path;
#[derive(Clone)]
struct Selection {
    mode: ColorMode,
    density: DensityScale,
    palette: Option<PaletteId>,
    language: LanguageChoice,
}
struct State {
    draft: Option<Selection>,
    store: Store,
    snapshot: Option<Snapshot>,
    mode: ColorMode,
    density: DensityScale,
    palette: Option<PaletteId>,
    catalogue: Catalogue,
    languages: i18n::Catalogue,
    language: LanguageChoice,
    language_initialized: bool,
    base_font_size: Pixels,
    base_mono_font_size: Pixels,
    native: ThemeMode,
    busy: bool,
    message: Option<String>,
}
impl Global for State {}
impl State {
    fn selection(&self) -> Selection {
        self.draft.clone().unwrap_or_else(|| Selection {
            mode: self.mode,
            density: self.density,
            palette: self.palette.clone(),
            language: self.language.clone(),
        })
    }
}
fn apply(cx: &mut App) {
    let state = cx.global::<State>();
    let mode = match state.mode {
        ColorMode::System => state.native,
        ColorMode::Light => ThemeMode::Light,
        ColorMode::Dark => ThemeMode::Dark,
    };
    let font_size = state.base_font_size * state.density.factor();
    let mono_font_size = state.base_mono_font_size * state.density.factor();
    let palette = state
        .palette
        .as_ref()
        .and_then(|id| state.catalogue.palettes.iter().find(|p| p.id == *id))
        .cloned();
    Theme::change(mode, None, cx);
    // Kit's Root WindowState sets each window's rem basis from font_size before
    // layout. Theme::update also refreshes the Base projection and every window.
    // Always derive from the unscaled baseline so repeated choices never drift.
    Theme::update(cx, |theme| {
        theme.font_size = font_size;
        theme.mono_font_size = mono_font_size;
        if let Some(palette) = palette {
            let colors = palette.document.colors;
            theme.background = rgb(colors.background.rgb()).into();
            theme.foreground = rgb(colors.foreground.rgb()).into();
            theme.popover = theme.background;
            theme.popover_foreground = theme.foreground;
            theme.primary = rgb(colors.primary.rgb()).into();
            theme.primary_foreground = rgb(colors.primary_foreground.rgb()).into();
            theme.secondary = rgb(colors.secondary.rgb()).into();
            theme.secondary_foreground = rgb(colors.secondary_foreground.rgb()).into();
            theme.muted = rgb(colors.muted.rgb()).into();
            theme.muted_foreground = rgb(colors.muted_foreground.rgb()).into();
            theme.accent = rgb(colors.accent.rgb()).into();
            theme.accent_foreground = rgb(colors.accent_foreground.rgb()).into();
            theme.border = rgb(colors.border.rgb()).into();
            theme.danger = rgb(colors.danger.rgb()).into();
            theme.danger_foreground = rgb(colors.danger_foreground.rgb()).into();
            theme.ring = theme.primary;
        }
    });
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
        draft: None,
        store: Store::new(database),
        snapshot: None,
        mode: ColorMode::System,
        density: DensityScale::default(),
        palette: None,
        catalogue: Catalogue::default(),
        languages: i18n::Catalogue::bundled(),
        language: LanguageChoice::System,
        language_initialized: false,
        base_font_size: cx.theme().font_size,
        base_mono_font_size: cx.theme().mono_font_size,
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
    s.message = Some(crate::i18n::tr("Loading appearance…"));
    let store = s.store.clone();
    let task = cx.background_executor().spawn(async move {
        let result = store.load();
        let palettes = store.catalogue();
        let languages = i18n::Catalogue::discover(&store.languages_path());
        let localizer = result.as_ref().ok().map(|snapshot| {
            languages
                .as_ref()
                .map(|catalogue| {
                    catalogue.activate(&snapshot.language, &i18n::environment_locale())
                })
                .unwrap_or_else(|e| {
                    i18n::Localizer::fallback(crate::i18n::tr_args(
                        "Cannot load languages: %{e}",
                        &[("e", &format!("{:#}", e))],
                    ))
                })
        });
        (result, palettes, languages, localizer)
    });
    cx.spawn(async move |cx| {
        let (result, catalogue, languages, localizer) = task.await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            state.busy = false;
            state.languages = languages.unwrap_or_else(|e| { let mut fallback = i18n::Catalogue::bundled(); fallback.warning = Some(crate::i18n::tr_args("Cannot load languages: %{e}", &[("e", &format!("{:#}", e))])); fallback });
            if !state.language_initialized && let Some(localizer) = localizer {
                i18n::install(localizer); state.language_initialized = true;
            }
            state.catalogue = catalogue.unwrap_or_else(|e| Catalogue { palettes: Vec::new(), warning: Some(crate::i18n::tr_args("Cannot load native palettes: %{e}", &[("e", &format!("{:#}", e))])) });
            state.draft = None;
            match result {
                Ok(snapshot) => {
                    state.language = snapshot.language.clone();
                    state.mode = snapshot.mode;
                    state.density = snapshot.density;
                    let palette = snapshot.palette.as_ref().and_then(|id| state.catalogue.palettes.iter().find(|p| p.id == *id));
                    state.palette = snapshot.palette.clone();
                    if let Some(palette) = palette { state.mode = palette.document.mode; }
                    let missing = snapshot.palette.is_some() && palette.is_none();
                    state.message = snapshot.warning.clone().or_else(|| missing.then(|| "Saved native palette is unavailable. Using the saved built-in color mode; reload after restoring its file.".into())).or_else(|| state.catalogue.warning.clone()).or_else(|| i18n::current().warning).or_else(|| state.languages.warning.clone());
                    state.snapshot = Some(snapshot);
                }
                Err(e) => {
                    state.message = Some(crate::i18n::tr_args("Cannot load appearance preferences: %{e}", &[("e", &format!("{:#}", e))]));
                }
            }
            i18n::refresh_inputs(cx);
            i18n::refresh_titles(cx);
            apply(cx);
            cx.refresh_windows();
        });
    })
    .detach();
    cx.refresh_windows();
}
fn stage(cx: &mut App, change: impl FnOnce(&mut Selection)) {
    let state = cx.global_mut::<State>();
    if state.busy || state.snapshot.is_none() {
        return;
    }
    let mut draft = state.selection();
    change(&mut draft);
    state.draft = Some(draft);
    state.message = None;
    cx.refresh_windows();
}
fn choose(mode: ColorMode, cx: &mut App) {
    stage(cx, |draft| {
        draft.mode = mode;
        draft.palette = None;
    });
}
fn choose_density(density: DensityScale, cx: &mut App) {
    stage(cx, |draft| draft.density = density);
}
fn choose_palette(id: PaletteId, cx: &mut App) {
    let Some(mode) = cx
        .global::<State>()
        .catalogue
        .palettes
        .iter()
        .find(|palette| palette.id == id)
        .map(|palette| palette.document.mode)
    else {
        return;
    };
    stage(cx, |draft| {
        draft.mode = mode;
        draft.palette = Some(id);
    });
}
fn choose_language(language: LanguageChoice, cx: &mut App) {
    stage(cx, |draft| draft.language = language);
}
fn save(cx: &mut App) {
    let state = cx.global_mut::<State>();
    if state.busy {
        return;
    }
    let Some(snapshot) = state.snapshot.clone() else {
        return;
    };
    let draft = state.selection();
    let store = state.store.clone();
    state.busy = true;
    state.message = None;
    let task = cx.background_executor().spawn(async move {
        store.save_all(
            &snapshot,
            draft.mode,
            draft.density,
            draft.palette,
            draft.language,
        )
    });
    cx.spawn(async move |cx| {
        let result = task.await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            state.busy = false;
            match result {
                Ok(snapshot) => {
                    let language_changed = state.language != snapshot.language;
                    state.mode = snapshot.mode;
                    state.density = snapshot.density;
                    state.palette = snapshot.palette.clone();
                    state.language = snapshot.language.clone();
                    state.snapshot = Some(snapshot);
                    state.draft = None;
                    state.message = Some(if language_changed {
                        i18n::text("language.restart")
                    } else {
                        crate::i18n::tr("Appearance saved.")
                    });
                    apply(cx);
                }
                Err(error) => {
                    state.message = Some(crate::i18n::tr_args(
                        "Could not save appearance: %{error}",
                        &[("error", &format!("{error:#}"))],
                    ))
                }
            }
            cx.refresh_windows();
        });
    })
    .detach();
    cx.refresh_windows();
}
pub(super) struct Appearance;
impl Appearance {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|_, cx| {
            if cx.has_global::<State>() {
                cx.global_mut::<State>().draft = None;
            }
        })
        .detach();
        reload(cx);
        Self
    }
}
impl Render for Appearance {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.global::<State>();
        let message = state.message.clone();
        let busy = state.busy;
        let color = SettingItem::new(
            crate::i18n::tr("Color mode"),
            SettingField::render(|_, _, cx| {
                let state = cx.global::<State>();
                let disabled = state.busy || state.snapshot.is_none();
                let selected = state
                    .selection()
                    .palette
                    .is_none()
                    .then_some(state.selection().mode);
                div().flex().flex_wrap().gap_2().children(
                    [ColorMode::System, ColorMode::Light, ColorMode::Dark].map(|mode| {
                        Button::new(match mode {
                            ColorMode::System => "appearance-system",
                            ColorMode::Light => "appearance-light",
                            ColorMode::Dark => "appearance-dark",
                        })
                        .label(crate::i18n::tr(mode.label()))
                        .selected(selected == Some(mode))
                        .disabled(disabled)
                        .on_click(move |_, _, cx| choose(mode, cx))
                    }),
                )
            }),
        )
        .layout(Axis::Vertical)
        .description(crate::i18n::tr(
            "System follows the desktop's light or dark appearance.",
        ));
        let reset = SettingItem::new(
            crate::i18n::tr("Default"),
            SettingField::render(|_, _, cx| {
                div().flex().child(
                    Button::new("appearance-reset")
                        .label(crate::i18n::tr("Use System appearance"))
                        .disabled(
                            cx.global::<State>().busy || cx.global::<State>().snapshot.is_none(),
                        )
                        .on_click(|_, _, cx| choose(ColorMode::System, cx)),
                )
            }),
        );
        let reset = reset.layout(Axis::Vertical);
        let palettes = SettingItem::new(crate::i18n::tr("Installed native palettes"),
            SettingField::render(|_, _, cx| {
                let state = cx.global::<State>();
                let disabled = state.busy || state.snapshot.is_none();
                let selected = state.selection().palette;
                if state.catalogue.palettes.is_empty() {
                    return div().w_full().min_w_0().whitespace_normal().child(crate::i18n::tr("No native palettes installed."));
                }
                div().flex().flex_wrap().gap_2().children(state.catalogue.palettes.iter().map(|palette| {
                    let id = palette.id.clone();
                    Button::new(SharedString::from(format!("appearance-palette-{}", id.as_str())))
                        .label(palette.document.name.clone())
                        .selected(selected.as_ref() == Some(&id)).disabled(disabled)
                        .on_click(move |_, _, cx| choose_palette(id.clone(), cx))
                }))
            }),
        ).layout(Axis::Vertical).description(crate::i18n::tr("Choose an installed custom palette. Reload to discover changes; built-in color buttons restore standard colors."));
        let density = SettingItem::new(crate::i18n::tr("Display density"),
            SettingField::render(|_, _, cx| {
                let state = cx.global::<State>();
                let disabled = state.busy || state.snapshot.is_none();
                let value = state.selection().density;
                let down = DensityScale::new(value.value() - 1);
                let up = DensityScale::new(value.value() + 1);
                div().flex().items_center().flex_wrap().gap_2()
                    .child(Button::new("appearance-density-down").label("−")
                        .tooltip(crate::i18n::tr("Decrease display density scale"))
                        .disabled(disabled || down.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(value) = down { choose_density(value, cx); }
                        }))
                    .child(div().id("appearance-density-value").min_w(px(85.))
                        .child(format!("{} ({}%)", value.value(), value.percent())))
                    .child(Button::new("appearance-density-up").label("+")
                        .tooltip(crate::i18n::tr("Increase display density scale"))
                        .disabled(disabled || up.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(value) = up { choose_density(value, cx); }
                        }))
                    .child(Button::new("appearance-density-reset").label(crate::i18n::tr("Default density"))
                        .disabled(disabled)
                        .on_click(|_, _, cx| choose_density(DensityScale::default(), cx)))
            }),
        ).layout(Axis::Vertical).description(crate::i18n::tr("Adjust native text and spacing from 60% to 140%. Zero uses the default size; fixed table and window dimensions stay unchanged."));
        let language = SettingItem::new(
            i18n::text("language.title"),
            SettingField::render(|_, _, cx| {
                let state = cx.global::<State>();
                let disabled = state.busy || state.snapshot.is_none();
                let missing = match &state.selection().language {
                    LanguageChoice::Locale(id) => {
                        !state.languages.languages.iter().any(|l| &l.id == id)
                    }
                    LanguageChoice::System => false,
                };
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("appearance-language-system")
                                    .label(i18n::text("language.system"))
                                    .selected(state.selection().language == LanguageChoice::System)
                                    .disabled(disabled)
                                    .on_click(|_, _, cx| {
                                        choose_language(LanguageChoice::System, cx)
                                    }),
                            )
                            .children(state.languages.languages.iter().map(|language| {
                                let id = language.id.clone();
                                let choice = LanguageChoice::Locale(id.clone());
                                Button::new(SharedString::from(format!(
                                    "appearance-language-{}",
                                    id.as_str()
                                )))
                                .label(
                                    i18n::format(
                                        "language.choice_coverage",
                                        &[
                                            ("name", &language.name),
                                            ("translated", &language.translated.to_string()),
                                            ("total", &language.total.to_string()),
                                        ],
                                    )
                                    .unwrap_or_else(|_| language.name.clone()),
                                )
                                .selected(state.selection().language == choice)
                                .disabled(disabled)
                                .on_click(move |_, _, cx| choose_language(choice.clone(), cx))
                            })),
                    )
                    .child(
                        i18n::format(
                            "language.active",
                            &[("language", i18n::current().locale.as_str())],
                        )
                        .unwrap_or_default(),
                    )
                    .child({
                        let (translated, total) = i18n::current().coverage();
                        i18n::format(
                            "language.coverage",
                            &[
                                ("translated", &translated.to_string()),
                                ("total", &total.to_string()),
                            ],
                        )
                        .unwrap_or_default()
                    })
                    .when(missing, |view| {
                        view.child(i18n::text("language.unavailable"))
                    })
            }),
        )
        .description(i18n::text("language.description"))
        .layout(Axis::Vertical);
        let settings = Settings::new("appearance-settings")
            .sidebar_width(px(0.))
            .sidebar_size_range(px(0.)..px(0.))
            .sidebar_style(&StyleRefinement { display: Some(Display::None), ..Default::default() })
            .pages(vec![
                SettingPage::new(crate::i18n::tr("Appearance"))
                    .header_style(&StyleRefinement { display: Some(Display::None), ..Default::default() })
                    .default_open(true)
                    .group(
                        SettingGroup::new()
                            .title(crate::i18n::tr("Size"))
                            .item(density),
                    )
                    .group(
                        SettingGroup::new()
                            .title(crate::i18n::tr("Colors"))
                            .description(crate::i18n::tr(
                                "Save applies appearance to every open window while keeping current edits.",
                            ))
                            .item(color)
                            .item(reset),
                    )
                    .group(
                        SettingGroup::new()
                            .title(crate::i18n::tr("Palettes"))
                            .item(palettes)
                            .item(SettingItem::render(|_, _, cx| div().flex().child(Button::new("appearance-reload").label(crate::i18n::tr("Reload")).disabled(cx.global::<State>().busy).on_click(|_, _, cx| reload(cx))))),
                    )
                    .group(
                        SettingGroup::new()
                            .title(i18n::text("language.title"))
                            .item(language),
                    ),
            ]);
        let content = div()
            .id("appearance-content")
            .flex_1()
            .min_h_0()
            .px_1()
            .child(settings);
        #[cfg(feature = "headless-tests")]
        let content = content.test_support();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(super::settings::heading("Appearance"))
            .child(content)
            .when_some(message, |view, message| {
                view.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .child(crate::i18n::presentation(&message)),
                )
            })
            .child(super::settings::footer(
                "appearance-close",
                busy,
                Button::new("appearance-save")
                    .disabled(busy || state.snapshot.is_none())
                    .on_click(|_, _, cx| save(cx)),
                cx,
            ))
    }
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_state(cx: &App) -> (bool, ColorMode) {
    let s = cx.global::<State>();
    (s.busy, s.mode)
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_density(cx: &App) -> DensityScale {
    cx.global::<State>().density
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_palette(cx: &App) -> Option<PaletteId> {
    cx.global::<State>().palette.clone()
}
