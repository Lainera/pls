
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .compile(&["proto/runner.proto"], &["proto"])?;
    Ok(())
}
