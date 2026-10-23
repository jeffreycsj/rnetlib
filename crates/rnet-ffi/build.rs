use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=Cargo.toml");
    let config = cbindgen::Config::from_file(root.join("cbindgen.toml"))
        .expect("read C header generation configuration");
    let bindings = cbindgen::Builder::new()
        .with_crate(root)
        .with_config(config)
        .generate()
        .expect("generate C header from Rust FFI declarations");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("rnet.h");
    bindings.write_to_file(output);
}
