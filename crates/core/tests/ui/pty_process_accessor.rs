use firkin_core::{Container, ExecConfig, Pty, PtyControl, PtyInput, PtyOutput, Result, Rootfs};

async fn compile_process_pty_accessor() -> Result<()> {
    let mut container = Container::builder("pty-process-accessor")?
        .command(["/bin/sleep", "1"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .spawn()
        .await?;
    let mut process = container
        .exec(
            "pty-process",
            ExecConfig::builder()
                .command(["/bin/sh"])
                .pty((80, 24))
                .build(),
        )
        .await?;
    let _: &mut Pty = process.pty();
    let pty = process.take_pty().await?.unwrap();
    let (_input, _output, _control): (PtyInput, PtyOutput, PtyControl) = pty.split();
    Ok(())
}

fn main() {}
