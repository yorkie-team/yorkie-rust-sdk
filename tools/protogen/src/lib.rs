//! Regenerates `crates/yorkie-protocol/src/generated/yorkie.v1.rs` from the
//! vendored protos. Run with `cargo run -p protogen`.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// Root of the workspace, derived from this crate's manifest directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Path of the vendored proto include directory.
pub fn proto_dir() -> PathBuf {
    workspace_root().join("crates/yorkie-protocol/proto")
}

/// Path of the checked-in generated file.
pub fn generated_file() -> PathBuf {
    workspace_root().join("crates/yorkie-protocol/src/generated/yorkie.v1.rs")
}

/// Generates `yorkie.v1.rs` into `out_dir`.
pub fn generate(out_dir: &Path) -> std::io::Result<()> {
    let proto = proto_dir();
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.btree_map(["."]);
    config.out_dir(out_dir);
    config.compile_protos(
        &[
            proto.join("yorkie/v1/resources.proto"),
            proto.join("yorkie/v1/yorkie.proto"),
        ],
        &[proto],
    )
}
