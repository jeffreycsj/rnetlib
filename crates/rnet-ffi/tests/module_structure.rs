use std::fs;
use std::path::Path;

#[test]
fn ffi_implementation_is_split_by_public_capability() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let lib = fs::read_to_string(source.join("lib.rs")).expect("read ffi lib.rs");
    assert!(
        lib.lines().count() <= 200,
        "ffi lib.rs has {} lines",
        lib.lines().count()
    );
    for module in [
        "abi.rs",
        "endpoint.rs",
        "events.rs",
        "observe.rs",
        "registry.rs",
        "runtime.rs",
        "session.rs",
    ] {
        assert!(source.join(module).is_file(), "missing {module}");
    }

    for entry in fs::read_dir(&source).expect("read FFI source directory") {
        let path = entry.expect("read FFI source entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read FFI module");
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
