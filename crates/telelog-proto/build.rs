use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc so contributors don't need it installed.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let well_known = protoc_bin_vendored::include_path()?;
    // SAFETY: build scripts are single-threaded.
    unsafe { std::env::set_var("PROTOC", protoc) };

    tonic_prost_build::configure().compile_protos(
        &[PathBuf::from("../../proto/telelog/v1/logs.proto")],
        &[PathBuf::from("../../proto"), well_known],
    )?;
    println!("cargo:rerun-if-changed=../../proto");
    Ok(())
}
