use std::fs;
use std::path::Path;

#[test]
fn legacy_secure_execution_modules_are_removed() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for module in ["secure_tcp", "secure_datagram", "secure_kcp", "tcp", "udp"] {
        assert!(
            !source.join(format!("{module}.rs")).exists(),
            "obsolete execution path: {module}"
        );
    }
    let config = fs::read_to_string(source.join("config.rs")).unwrap();
    for name in [
        "secure_tcp_listener",
        "secure_tcp_client",
        "secure_udp_listener",
        "secure_udp_client",
        "secure_kcp_listener",
        "secure_kcp_client",
        "pub fn tcp_listener",
        "pub fn tcp_client",
        "pub fn udp(",
        "pub fn kcp(",
        "allow_legacy_unauthenticated_endpoints",
    ] {
        assert!(!config.contains(name), "obsolete constructor: {name}");
    }
}

#[test]
fn transport_implementation_is_split_by_responsibility() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let lib = fs::read_to_string(source.join("lib.rs")).expect("read transport lib.rs");
    assert!(
        lib.lines().count() <= 200,
        "transport lib.rs has {} lines",
        lib.lines().count()
    );
    for module in [
        "config.rs",
        "kcp.rs",
        "lifecycle.rs",
        "metrics.rs",
        "runtime.rs",
        "adaptive_datagram.rs",
        "adaptive_tcp.rs",
        "adaptive_wire.rs",
        "state.rs",
    ] {
        assert!(source.join(module).is_file(), "missing {module}");
    }

    for entry in fs::read_dir(&source).expect("read transport source directory") {
        let path = entry.expect("read transport source entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read transport module");
        assert!(
            contents.lines().count() <= 600,
            "{} exceeds 600 lines",
            path.display()
        );
        assert!(
            !contents.contains("use super::*;"),
            "{} hides dependencies behind a wildcard import",
            path.display()
        );
    }
}
