use prost::Message;
use rustleblower::{
    history::{
        Query, Worker,
        details::{ARGUMENTS, DESTINATION, DraftKind, MAX_DETAIL_BYTES, PATH},
    },
    protocol::{Connection, Event, Operator, Rule, StringInt},
};
fn event() -> Event {
    Event {
        time: "reported UTC time".into(),
        unixnano: 1234567890000000000,
        connection: Some(Connection {
            protocol: "tcp".into(),
            src_ip: "192.0.2.10".into(),
            src_port: 50000,
            dst_ip: "198.51.100.20".into(),
            dst_host: "api.example.test".into(),
            dst_port: 443,
            user_id: 1000,
            process_id: 4567,
            process_path: "/usr/bin/example".into(),
            process_cwd: "/tmp/working directory".into(),
            process_args: vec!["/usr/bin/example".into(), "argument with spaces".into()],
            process_env: [
                ("SECRET".into(), "never stored".into()),
                ("OTHER".into(), "value".into()),
            ]
            .into(),
            process_checksums: [("sha1".into(), "not historical".into())].into(),
            process_tree: vec![StringInt {
                key: "parent".into(),
                value: 1,
            }],
        }),
        rule: Some(Rule {
            name: "original.rule".into(),
            enabled: true,
            precedence: true,
            nolog: true,
            action: "deny".into(),
            duration: "always".into(),
            operator: Some(Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: "/usr/bin/example".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    }
}
#[tokio::test]
async fn details_roundtrip_across_restart_and_create_reviewable_rules() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("history.sqlite3");
    let worker = Worker::start(&database, 7, 16).unwrap();
    worker
        .history
        .ingest("node".into(), vec![event()], "observed");
    worker.history.flush().await.unwrap();
    let row = worker
        .history
        .query(Query::default())
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .rows
        .first()
        .unwrap()
        .clone();
    worker.join();
    let worker = Worker::start(&database, 7, 16).unwrap();
    let detail = worker
        .history
        .detail(row.id)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(detail.row.node, "node");
    assert_eq!(detail.truncated, 0);
    let connection = detail.connection().unwrap();
    assert_eq!(
        connection.process_args,
        event().connection.unwrap().process_args
    );
    assert_eq!(connection.process_cwd, "/tmp/working directory");
    assert_eq!(connection.user_id, 1000);
    assert_eq!(connection.process_id, 4567);
    assert_eq!(connection.src_port, 50000);
    assert_eq!(connection.dst_ip, "198.51.100.20");
    assert!(
        connection.process_env.is_empty()
            && connection.process_checksums.is_empty()
            && connection.process_tree.is_empty()
    );
    assert!(detail.matched_rule().unwrap().operator.is_none());
    let application = detail.draft(DraftKind::Application).unwrap();
    assert_eq!(application.action, "allow");
    assert_eq!(application.duration, "always");
    assert!(application.enabled && !application.precedence && !application.nolog);
    assert_eq!(application.operator.unwrap().data, "/usr/bin/example");
    let destination = detail.draft(DraftKind::Destination).unwrap();
    let fields = destination.operator.unwrap().list;
    assert_eq!(
        fields
            .iter()
            .map(|op| op.operand.as_str())
            .collect::<Vec<_>>(),
        ["process.path", "dest.host", "dest.port", "protocol"]
    );
    assert_eq!(fields[1].data, "api.example.test");
    assert_eq!(fields[2].data, "443");
    let mut ipv6 = detail.clone();
    let connection = ipv6.event.as_mut().unwrap().connection.as_mut().unwrap();
    connection.dst_host.clear();
    connection.dst_ip = "2001:db8::42".into();
    assert_eq!(
        ipv6.draft(DraftKind::Destination)
            .unwrap()
            .operator
            .unwrap()
            .list[1]
            .data,
        "2001:db8::42"
    );
    assert_eq!(
        rustleblower::history::details::endpoint("2001:db8::42", 443),
        "[2001:db8::42]:443"
    );
    let db = rusqlite::Connection::open(&database).unwrap();
    let bytes: i64 = db
        .query_row("SELECT length(payload) FROM event_details", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(bytes <= MAX_DETAIL_BYTES as i64);
    let payload: Vec<u8> = db
        .query_row("SELECT payload FROM event_details", [], |row| row.get(0))
        .unwrap();
    assert!(!String::from_utf8_lossy(&payload).contains("never stored"));
    drop(db);
    worker.join();
}

#[tokio::test]
async fn replay_deduplication_ignores_hashmap_iteration_order() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::start(&dir.path().join("history.sqlite3"), 7, 16).unwrap();
    let wire = event().encode_to_vec();
    for _ in 0..10 {
        worker.history.ingest(
            "node".into(),
            vec![Event::decode(wire.as_slice()).unwrap()],
            "observed",
        );
    }
    worker.history.flush().await.unwrap();
    assert_eq!(
        worker
            .history
            .query(Query::default())
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .rows
            .len(),
        1
    );
    let db = rusqlite::Connection::open(worker.history.database_path()).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM event_details", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(db);
    worker.join();
}

#[tokio::test]
async fn oversized_fields_are_marked_and_cannot_seed_incomplete_match_rules() {
    let dir = tempfile::tempdir().unwrap();
    let worker = Worker::start(&dir.path().join("history.sqlite3"), 7, 16).unwrap();
    let mut event = event();
    let con = event.connection.as_mut().unwrap();
    con.process_path = "é".repeat(3000);
    con.dst_host = "h".repeat(2000);
    con.process_args = vec!["a".repeat(2000); 100];
    worker
        .history
        .ingest("node".into(), vec![event], "observed");
    worker.history.flush().await.unwrap();
    let row = worker
        .history
        .query(Query::default())
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .rows
        .first()
        .unwrap()
        .clone();
    let detail = worker
        .history
        .detail(row.id)
        .unwrap()
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        detail.truncated & (PATH | DESTINATION | ARGUMENTS),
        PATH | DESTINATION | ARGUMENTS
    );
    assert!(detail.event.as_ref().unwrap().encoded_len() <= MAX_DETAIL_BYTES);
    assert_eq!(detail.connection().unwrap().process_path.len(), 4096);
    assert!(
        detail
            .connection()
            .unwrap()
            .process_args
            .iter()
            .map(String::len)
            .sum::<usize>()
            <= 8192
    );
    assert!(
        detail.draft(DraftKind::Application).is_err()
            && detail.draft(DraftKind::Destination).is_err()
    );
    worker.join();
}
