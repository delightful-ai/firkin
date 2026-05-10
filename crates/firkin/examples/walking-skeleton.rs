//! Pull busybox and run one command through the full firkin stack.

use firkin::oci::{Client, Reference};
use firkin::{Container, Rootfs};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bundle = Client::default()
        .pull(&Reference::parse("docker.io/library/busybox:latest")?)
        .await?;

    let output = Container::builder("hello")?
        .image_config(bundle.config())
        .rootfs(Rootfs::oci_bundle(bundle))
        .command(["/bin/echo", "hello from a real rust-rewrite container"])
        .output()
        .await?;

    assert!(output.status.success());
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}
