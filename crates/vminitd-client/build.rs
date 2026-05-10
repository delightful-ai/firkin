//! Build-time `SandboxContext` protobuf generation.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_client(true)
        .build_server(false)
        .compile_protos(&["proto/SandboxContext.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto/SandboxContext.proto");
    Ok(())
}
