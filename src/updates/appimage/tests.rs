use super::*;
use crate::{
    backend::Backend,
    config::{Config, Decision},
};
use std::sync::Arc;

fn image(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination)?;
    let mut file = OpenOptions::new().write(true).open(destination)?;
    file.seek(SeekFrom::Start(8))?;
    file.write_all(b"AI\x02")?;
    Ok(())
}

fn stage(installed: PathBuf, update: &Path) -> Result<Restart> {
    let installation = Arc::new(Installation::open(installed)?);
    let workspace = Workspace::acquire(&installation)?;
    let mut file = tempfile::NamedTempFile::new_in(&workspace.path)?;
    let bytes = fs::read(update)?;
    file.write_all(&bytes)?;
    Pending {
        file,
        version: "99.0.0".into(),
        size: bytes.len() as u64,
        checksum: Sha256::digest(&bytes).into(),
        _workspace: workspace,
    }
    .prepare(installation)
}

fn config(dir: &Path) -> Config {
    Config {
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        transport: Default::default(),
        history_days: 7,
        history_mib: 32,
        headless: true,
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = stop_failed_child(&mut self.0);
    }
}

#[test]
fn restart_probe() -> Result<()> {
    let Some(dir) = std::env::var_os("RUSTLEBLOWER_TEST_RESTART_DIR") else {
        return Ok(());
    };
    let dir = PathBuf::from(dir);
    let config = config(&dir);
    // This runs in the new process, before its backend can bind.
    assert!(
        !config.socket.exists(),
        "The old backend still owns the socket"
    );
    let backend = Backend::start(config.clone())?;
    fs::write(dir.join("rebound"), b"ready")?;
    signal_ready();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.join("stop").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    backend.join();
    assert!(!config.socket.exists());
    Ok(())
}

#[test]
fn restart_waits_for_socket_release_and_rolls_back_failed_launch() -> Result<()> {
    for rollback in [false, true] {
        let dir = tempfile::tempdir()?;
        let installed = dir.path().join("installed.AppImage");
        let update = dir.path().join("new.AppImage");
        image(Path::new("/bin/sh"), &installed)?;
        image(
            Path::new(if rollback { "/bin/false" } else { "/bin/sh" }),
            &update,
        )?;
        let old_inode = fs::metadata(&installed)?.ino();
        let config = config(dir.path());
        let backend = Backend::start(config.clone())?;
        assert!(config.socket.exists());
        let args = vec![
            "-c".into(),
            "RUSTLEBLOWER_TEST_RESTART_DIR=\"$2\" exec \"$1\" --exact updates::appimage::tests::restart_probe".into(),
            "restart-probe".into(),
            std::env::current_exe()?.into_os_string(),
            dir.path().as_os_str().to_owned(),
        ];
        let restart = stage(installed.clone(), &update)?;
        let mut child =
            OwnedChild(restart.after_shutdown_with_args(backend, &config.socket, &args)?);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !dir.path().join("rebound").exists() {
            ensure!(
                Instant::now() < deadline && child.0.try_wait()?.is_none(),
                "Replacement did not rebind the socket"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(std::os::unix::net::UnixStream::connect(&config.socket).is_ok());
        assert_eq!(fs::metadata(&installed)?.ino() == old_inode, rollback);
        fs::write(dir.path().join("stop"), b"stop")?;
        while child.0.try_wait()?.is_none() {
            ensure!(Instant::now() < deadline, "Replacement did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.0.wait()?.success());
        assert!(!config.socket.exists());
    }
    Ok(())
}

#[test]
fn restart_preserves_a_socket_owned_by_another_process() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path());
    let installed = dir.path().join("installed.AppImage");
    let update = dir.path().join("new.AppImage");
    image(Path::new("/bin/sh"), &installed)?;
    image(Path::new("/bin/false"), &update)?;
    let old_inode = fs::metadata(&installed)?.ino();
    let listener = std::os::unix::net::UnixListener::bind(&config.socket)?;
    let socket_inode = fs::symlink_metadata(&config.socket)?.ino();
    let restart = stage(installed.clone(), &update)?;
    assert!(
        restart
            .install(
                &crate::transport::Endpoint::Unix(config.socket.clone()),
                &[]
            )
            .is_err()
    );
    assert_eq!(fs::symlink_metadata(&config.socket)?.ino(), socket_inode);
    assert_eq!(fs::metadata(&installed)?.ino(), old_inode);
    drop(listener);
    Ok(())
}

#[test]
#[ignore = "requires an owned AppImage containing the updated Rustleblower binary"]
fn packaged_appimage_rebinds_after_shutdown() -> Result<()> {
    let update =
        std::env::var_os("RUSTLEBLOWER_TEST_APPIMAGE").context("Set RUSTLEBLOWER_TEST_APPIMAGE")?;
    let dir = tempfile::tempdir()?;
    let installed = dir.path().join("installed.AppImage");
    image(Path::new("/bin/sh"), &installed)?;
    let config = config(dir.path());
    let backend = Backend::start(config.clone())?;
    assert!(config.socket.exists());
    let args = vec![
        "--headless".into(),
        "--socket".into(),
        config.socket.clone().into_os_string(),
        "--database".into(),
        config.database.into_os_string(),
    ];
    let restart = stage(installed.clone(), Path::new(&update))?;
    let mut child = OwnedChild(restart.after_shutdown_with_args(backend, &config.socket, &args)?);
    let socket = config.socket.clone();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                let channel = tonic::transport::Endpoint::from_static("http://localhost")
                    .connect_with_connector(tower::service_fn(move |_| {
                        let socket = socket.clone();
                        async move {
                            tokio::net::UnixStream::connect(socket)
                                .await
                                .map(hyper_util::rt::TokioIo::new)
                        }
                    }))
                    .await?;
                let mut daemon = crate::protocol::ui_client::UiClient::new(channel);
                let reply = daemon
                    .ping(crate::protocol::PingRequest {
                        id: 73,
                        ..Default::default()
                    })
                    .await?;
                assert_eq!(reply.into_inner().id, 73);
                Ok::<_, anyhow::Error>(())
            })
            .await?
        })?;
    assert!(stop_failed_child(&mut child.0));
    assert!(!config.socket.exists());
    Ok(())
}
