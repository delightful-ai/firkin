use firkin_core::{Container, Result, Rootfs};

async fn compile_stream_container_has_no_pty() -> Result<()> {
    let mut container = Container::builder("stream-container-pty")?
        .command(["/bin/sh"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .spawn()
        .await?;
    let _ = container.pty();
    Ok(())
}

fn main() {}
