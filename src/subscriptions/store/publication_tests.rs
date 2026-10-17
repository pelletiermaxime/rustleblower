use super::*;
use crate::subscriptions::{collections, model::Spec};
use anyhow::Result;
use std::io::Write;

fn temp(store: &Store, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("download-")
        .tempfile_in(store.root.join("tmp"))?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}
fn setup(root: &Path, old: bool) -> Result<(Store, Row)> {
    let mut store = Store::open(root).map_err(anyhow::Error::msg)?;
    let id = store
        .save(
            None,
            &Spec {
                name: "Owned recovery fixture".into(),
                url: "https://example.test/list".into(),
                ..Default::default()
            },
        )
        .map_err(anyhow::Error::msg)?;
    let row = store.get(id).map_err(anyhow::Error::msg)?;
    if old {
        let file = temp(&store, b"old.example\n")?;
        assert!(
            store
                .publish(&row, file, 12, ("old-etag", "old-modified"), 60)
                .map_err(anyhow::Error::msg)?
        );
    }
    Ok((store, row))
}
fn pending(store: &Store) -> Result<usize> {
    Ok(store
        .db
        .query_row("SELECT count(*) FROM publication_intent", [], |r| r.get(0))?)
}
fn no_backup(store: &Store) -> Result<()> {
    assert_eq!(pending(store)?, 0);
    assert!(!store.root.join("tmp").join(BACKUP).exists());
    assert!(!store.root.join("tmp").join(RESTORE).exists());
    Ok(())
}

#[test]
fn late_sql_failure_restores_old_bytes_metadata_and_collection_generation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut store, row) = setup(dir.path(), true)?;
    let before = store.get(row.id).map_err(anyhow::Error::msg)?;
    collections::synchronize(&store).map_err(anyhow::Error::msg)?;
    let pointer = fs::read_link(store.root.join("collections/current"))?;
    let epoch: i64 = store
        .db
        .query_row("SELECT value FROM collection_epoch", [], |r| r.get(0))?;
    store.db.execute_batch("CREATE TRIGGER fail_publication_log BEFORE INSERT ON logs WHEN new.message='Download validated and published' BEGIN SELECT RAISE(ABORT,'owned late publication fault'); END;")?;
    let file = temp(&store, b"new-and-longer.example\n")?;
    assert!(
        !store
            .publish(&row, file, 23, ("new-etag", "new-modified"), 60)
            .map_err(anyhow::Error::msg)?
    );
    let after = store.get(row.id).map_err(anyhow::Error::msg)?;
    assert_eq!(after.status, "error");
    assert!(after.error.contains("owned late publication fault"));
    assert_eq!(after.bytes, before.bytes);
    assert_eq!(after.file, before.file);
    assert_eq!(after.etag, before.etag);
    assert_eq!(after.modified, before.modified);
    assert_eq!(after.publication.unwrap().bytes, 12);
    assert_eq!(store.used().map_err(anyhow::Error::msg)?, 12);
    assert_eq!(fs::read(store.data_path(&row))?, b"old.example\n");
    assert_eq!(
        store
            .db
            .query_row("SELECT value FROM collection_epoch", [], |r| r
                .get::<_, i64>(0))?,
        epoch
    );
    collections::synchronize(&store).map_err(anyhow::Error::msg)?;
    assert_eq!(
        fs::read_link(store.root.join("collections/current"))?,
        pointer
    );
    no_backup(&store)?;
    drop(store);
    let store = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
    assert_eq!(fs::read(store.data_path(&row))?, b"old.example\n");
    no_backup(&store)?;
    Ok(())
}

#[test]
fn first_publication_sql_failure_removes_uncommitted_file_and_publication_row() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (mut store, row) = setup(dir.path(), false)?;
    store.db.execute_batch("CREATE TRIGGER fail_first_publication BEFORE UPDATE ON subscriptions WHEN new.status='updated' BEGIN SELECT RAISE(ABORT,'owned first publication fault'); END;")?;
    let file = temp(&store, b"first.example\n")?;
    assert!(
        !store
            .publish(&row, file, 14, ("new", "new"), 60)
            .map_err(anyhow::Error::msg)?
    );
    assert!(!store.data_path(&row).exists());
    let current = store.get(row.id).map_err(anyhow::Error::msg)?;
    assert_eq!(current.status, "error");
    assert_eq!(current.bytes, 0);
    assert!(current.file.is_none() && current.publication.is_none());
    assert_eq!(store.used().map_err(anyhow::Error::msg)?, 0);
    no_backup(&store)?;
    Ok(())
}

#[test]
fn startup_recovers_before_rename_after_rename_and_repeated_rollback() -> Result<()> {
    for stage in 0..3 {
        let dir = tempfile::tempdir()?;
        let (store, row) = setup(dir.path(), true)?;
        store.db.execute(
            "UPDATE subscriptions SET status='downloading' WHERE id=?",
            [row.id],
        )?;
        store.begin_publication(&row).map_err(anyhow::Error::msg)?;
        assert_eq!(pending(&store)?, 1);
        if stage > 0 {
            temp(&store, b"new.example\n")?.persist(store.data_path(&row))?;
            sync(store.data_path(&row).parent().unwrap()).map_err(anyhow::Error::msg)?;
        }
        if stage == 2 {
            // Simulate another interruption after the rollback rename but before
            // SQLite records its completion. The retained backup must survive.
            let restore = store.root.join("tmp").join(RESTORE);
            fs::hard_link(store.root.join("tmp").join(BACKUP), &restore)?;
            fs::rename(&restore, store.data_path(&row))?;
            sync(store.data_path(&row).parent().unwrap()).map_err(anyhow::Error::msg)?;
        }
        drop(store);
        let store = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
        let current = store.get(row.id).map_err(anyhow::Error::msg)?;
        assert_eq!(current.status, "interrupted");
        assert_eq!(current.bytes, 12);
        assert_eq!(fs::read(store.data_path(&row))?, b"old.example\n");
        assert_eq!(current.publication.unwrap().bytes, 12);
        no_backup(&store)?;
    }
    Ok(())
}

#[test]
fn committed_metadata_survives_backup_cleanup_interruption_and_untrusted_paths_refuse() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let (mut store, row) = setup(dir.path(), true)?;
    store.begin_publication(&row).map_err(anyhow::Error::msg)?;
    let destination = store.data_path(&row);
    temp(&store, b"committed.example\n")?.persist(&destination)?;
    sync(destination.parent().unwrap()).map_err(anyhow::Error::msg)?;
    store
        .success(
            &row,
            Some(&destination),
            18,
            ("committed", "committed"),
            60,
            false,
        )
        .map_err(anyhow::Error::msg)?;
    assert_eq!(pending(&store)?, 0);
    assert!(store.root.join("tmp").join(BACKUP).exists());
    drop(store);
    let store = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
    assert_eq!(fs::read(&destination)?, b"committed.example\n");
    assert_eq!(store.get(row.id).map_err(anyhow::Error::msg)?.bytes, 18);
    no_backup(&store)?;
    let external = dir.path().join("unrelated");
    fs::write(&external, b"keep me")?;
    store.db.execute(
        "INSERT INTO publication_intent VALUES(1,?,?,?,0)",
        params![row.id, row.revision, external.to_string_lossy()],
    )?;
    assert!(store.recover_publication().is_err());
    assert_eq!(fs::read(&external)?, b"keep me");
    assert_eq!(pending(&store)?, 1);
    store.db.execute("DELETE FROM publication_intent", [])?;
    fs::remove_file(&destination)?;
    std::os::unix::fs::symlink(&external, &destination)?;
    assert!(store.begin_publication(&row).is_err());
    assert_eq!(fs::read(&external)?, b"keep me");
    assert_eq!(pending(&store)?, 0);
    Ok(())
}

#[test]
fn obsolete_filename_cleanup_survives_restart_and_preserves_other_format_publications() -> Result<()>
{
    use crate::subscriptions::model::Format;
    for interrupted_after_unlink in [false, true] {
        let dir = tempfile::tempdir()?;
        let (mut store, row) = setup(dir.path(), true)?;
        let old = store.data_path(&row);
        collections::synchronize(&store).map_err(anyhow::Error::msg)?;
        let mut spec = row.spec.clone();
        spec.filename = "new-name.txt".into();
        store
            .save(Some((row.id, row.revision)), &spec)
            .map_err(anyhow::Error::msg)?;
        let row = store.get(row.id).map_err(anyhow::Error::msg)?;
        let bytes = b"renamed.example\n";
        let file = temp(&store, bytes)?;
        assert!(
            store
                .publish(&row, file, bytes.len() as u64, ("renamed", "renamed"), 60)
                .map_err(anyhow::Error::msg)?
        );
        let new = store.data_path(&row);
        assert!(old.exists() && new.exists());
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM publication_cleanup", [], |r| r
                    .get::<_, usize>(0))?,
            1
        );
        collections::synchronize(&store).map_err(anyhow::Error::msg)?;
        if interrupted_after_unlink {
            fs::remove_file(&old)?;
        }
        drop(store);
        let mut store = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
        collections::synchronize(&store).map_err(anyhow::Error::msg)?;
        store.cleanup_publications().map_err(anyhow::Error::msg)?;
        assert!(!old.exists());
        assert_eq!(fs::read(&new)?, bytes);
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM publication_cleanup", [], |r| r
                    .get::<_, usize>(0))?,
            0
        );
        let row = store.get(row.id).map_err(anyhow::Error::msg)?;
        let mut spec = row.spec.clone();
        spec.format = Format::Ips;
        store
            .save(Some((row.id, row.revision)), &spec)
            .map_err(anyhow::Error::msg)?;
        let row = store.get(row.id).map_err(anyhow::Error::msg)?;
        let ips = b"192.0.2.4\n";
        let file = temp(&store, ips)?;
        assert!(
            store
                .publish(&row, file, ips.len() as u64, ("ips", "ips"), 60)
                .map_err(anyhow::Error::msg)?
        );
        collections::synchronize(&store).map_err(anyhow::Error::msg)?;
        store.cleanup_publications().map_err(anyhow::Error::msg)?;
        assert_eq!(fs::read(&new)?, bytes, "previous format remains referenced");
        assert_eq!(fs::read(store.data_path(&row))?, ips);
        assert_eq!(
            store.used().map_err(anyhow::Error::msg)?,
            (bytes.len() + ips.len()) as u64
        );
        assert_eq!(
            store
                .db
                .query_row("SELECT count(*) FROM subscription_publications", [], |r| {
                    r.get::<_, usize>(0)
                })?,
            2
        );
    }
    Ok(())
}

#[test]
fn closed_owner_releases_lock_even_if_a_child_inherits_its_open_file_description() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
    // fork() can briefly inherit the same open file description before CLOEXEC.
    // dup() retains exactly that kernel lock ownership without process timing.
    let inherited = store._lock.0.try_clone()?;
    assert!(
        Store::open(dir.path()).is_err(),
        "the live owner must remain exclusive"
    );
    drop(store);
    let reopened = Store::open(dir.path()).map_err(anyhow::Error::msg)?;
    drop(inherited);
    assert!(
        Store::open(dir.path()).is_err(),
        "closing the former child's descriptor cannot unlock the new owner"
    );
    drop(reopened);
    Store::open(dir.path()).map_err(anyhow::Error::msg)?;
    Ok(())
}
