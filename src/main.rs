use anyhow::Result;
use rustleblower::{backend::Backend, config::Config};

fn main() -> Result<()> {
    #[cfg(all(feature = "hot-reload", debug_assertions))]
    env_logger::init();

    #[cfg(feature = "desktop")]
    graphics_diagnostics();
    let (config, storage_overrides) = Config::parse_launch();
    #[cfg(feature = "desktop")]
    if !config.headless {
        anyhow::ensure!(
            ["WAYLAND_DISPLAY", "DISPLAY"]
                .iter()
                .any(|k| std::env::var_os(k).is_some_and(|s| !s.is_empty())),
            "Launch from a Wayland/X11 desktop, or pass --headless"
        );
    }
    #[cfg(not(feature = "desktop"))]
    anyhow::ensure!(
        config.headless,
        "This build requires --headless; enable the desktop feature for GPUI"
    );
    let backend = Backend::start_with_storage(config.clone(), storage_overrides)?;
    let (popup_preferences, timeout, fallback) = backend.handle.popup_policy();
    eprintln!(
        "Rustleblower listening on {}. History: {}. Unanswered prompts: {} once {}.",
        config.socket.display(),
        config.database.display(),
        fallback.action(),
        if popup_preferences.popups {
            format!("after {timeout}s")
        } else {
            "immediately (popups disabled)".into()
        }
    );
    if config.headless {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async { backend.handle.cancel.cancelled().await });
    } else {
        #[cfg(feature = "desktop")]
        rustleblower::app::run(backend.handle.clone(), config);
    }
    backend.join();
    Ok(())
}

#[cfg(feature = "desktop")]
fn graphics_diagnostics() {
    struct GraphicsLog;
    impl log::Log for GraphicsLog {
        fn enabled(&self, record: &log::Metadata) -> bool {
            record.level() <= log::Level::Warn
                && ["gpui", "blade", "wgpu"]
                    .iter()
                    .any(|prefix| record.target().starts_with(prefix))
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("{} {}: {}", record.level(), record.target(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: GraphicsLog = GraphicsLog;
    if std::env::var_os("RUSTLEBLOWER_GRAPHICS_DIAGNOSTICS").is_some()
        && log::set_logger(&LOGGER).is_ok()
    {
        log::set_max_level(log::LevelFilter::Warn);
    }
}
