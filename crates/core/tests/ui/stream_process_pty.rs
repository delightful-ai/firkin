use firkin_core::{Container, ExecConfig, Result, Rootfs};

async fn compile_stream_process_has_no_pty() -> Result<()> {
    let mut container = Container::builder("stream-process-pty")?
        .command(["/bin/sleep", "1"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .spawn()
        .await?;
    let mut process = container
        .exec(
            "stream-process",
            ExecConfig::builder().command(["/bin/sh"]).build(),
        )
        .await?;
    let _ = process.pty();
    Ok(())
}

fn main() {}
