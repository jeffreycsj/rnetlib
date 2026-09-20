use std::fs;
use std::path::Path;

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
        "secure_datagram.rs",
        "secure_tcp.rs",
        "state.rs",
        "tcp.rs",
        "udp.rs",
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
