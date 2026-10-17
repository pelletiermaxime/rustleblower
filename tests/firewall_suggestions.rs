use rustleblower::firewall::{
    helpers::{self, Kind},
    suggestions::{Catalogue, MAX_BYTES, MAX_SERVICES},
};
use std::os::unix::fs::symlink;

#[test]
fn service_aliases_are_protocol_qualified_unambiguous_and_feed_the_real_constructor() {
    let (services, limited) = Catalogue::parse_services(
        "https 443/tcp www-ssl # comment\nhttps 443/udp\nftp-data 20/tcp\nambiguous 80/tcp\nambiguous 81/tcp\ninvalid 70000/tcp\nzero 0/tcp\nbad 42/raw\ncontrol\\name 42/tcp\n",
    );
    assert!(!limited);
    let catalogue = Catalogue {
        services,
        ..Default::default()
    };
    assert_eq!(catalogue.port("https", "tcp,udp").unwrap(), Some(443));
    assert_eq!(catalogue.port("www-ssl", "tcp").unwrap(), Some(443));
    assert_eq!(catalogue.port("ftp-data", "tcp").unwrap(), Some(20));
    assert!(catalogue.port("www-ssl", "tcp,udp").is_err());
    assert!(catalogue.port("ambiguous", "tcp").is_err());
    for value in ["80", "80-90", "80,443"] {
        assert_eq!(catalogue.port(value, "tcp").unwrap(), None);
    }
    for value in ["", "missing", "zero", "invalid", "bad", "control\\name"] {
        assert!(catalogue.port(value, "tcp").is_err());
    }
    let port = catalogue
        .port("https", "tcp,udp")
        .unwrap()
        .unwrap()
        .to_string();
    let expression = helpers::statement(Kind::DestinationPort, "tcp,udp", "==", &port).unwrap();
    let statement = expression.statement.unwrap();
    assert_eq!(statement.name, "meta");
    assert_eq!(statement.values[1].value, "443");
}

#[test]
fn discovery_uses_owned_files_bounds_records_and_refuses_symlink_services() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let services = dir.path().join("services");
    let interfaces = dir.path().join("net");
    std::fs::create_dir(&interfaces)?;
    for value in ["lo", "owned0", "invalid name", "interface-name-too-long"] {
        std::fs::create_dir(interfaces.join(value))?;
    }
    std::fs::write(&services, "https 443/tcp\n")?;
    let catalogue = Catalogue::load(&services, &interfaces);
    assert_eq!(catalogue.interfaces, ["lo", "owned0"]);
    assert_eq!(catalogue.services.len(), 2);
    assert_eq!(catalogue.port("wireguard", "udp").unwrap(), Some(51820));
    assert_eq!(catalogue.warning, None);
    symlink(&services, dir.path().join("linked-services"))?;
    assert!(
        Catalogue::load(&dir.path().join("linked-services"), &interfaces)
            .services
            .is_empty()
    );
    assert!(
        Catalogue::load(&dir.path().join("missing"), &interfaces)
            .warning
            .is_some()
    );
    let records: String = (0..MAX_SERVICES + 5)
        .map(|i| format!("service{i} 443/tcp\n"))
        .collect();
    let (parsed, limited) = Catalogue::parse_services(&records);
    assert!(limited);
    assert_eq!(parsed.len(), MAX_SERVICES);
    let mut huge = "#".to_owned();
    huge.push_str(&"x".repeat(MAX_BYTES));
    huge.push_str("\ninvisible 123/tcp\n");
    std::fs::write(&services, &huge)?;
    let catalogue = Catalogue::load(&services, &interfaces);
    assert_eq!(catalogue.services.len(), 1);
    assert_eq!(catalogue.services[0].name, "wireguard");
    assert!(catalogue.warning.is_some());
    assert_eq!(Catalogue::parse_services(&huge).0.len(), 0);
    Ok(())
}
