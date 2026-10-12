//! Übersetzt den gRPC-Vertrag `proto/railit/v1/railit.proto` (ohne `protoc`, mit protox).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../proto");
    let fds = protox::compile(["railit/v1/railit.proto"], ["../../proto"])?;
    tonic_prost_build::configure()
        .build_client(true)
        // Clients wählen ihren Transport selbst (Tests, CLI).
        .build_transport(false)
        .build_server(true)
        .compile_fds(fds)?;
    Ok(())
}
