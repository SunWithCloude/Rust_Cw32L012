//! 构建脚本：让链接器找到 memory.x，并在其变更时重新链接。

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-search=native={}", manifest.display());
}
