// SPDX-License-Identifier: MIT
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/protocol-generator"));
    fs::create_dir_all(&output)?;
    tonic_prost_build::configure()
        .out_dir(&output)
        .emit_rerun_if_changed(false)
        .compile_protos(
            &[root.join("proto/rustleblower.proto")],
            &[root.join("proto")],
        )?;
    let generated = fs::read_to_string(output.join("protocol.rs"))?;
    let header = "// SPDX-License-Identifier: MIT\n// Generated from Rustleblower's proto/rustleblower.proto.\n// Generator: tonic-prost-build 0.14.6 / prost-build 0.14.4.\n// Regenerate with the locked tools/protocol-generator crate; see proto/README.md.\n";
    fs::write(root.join("src/protocol.rs"), format!("{header}{generated}"))?;
    Ok(())
}
