use firkin_core::{Container, Pty, Result, Rootfs};

async fn compile_container_pty_accessor() -> Result<()> {
    let mut container = Container::builder("pty-container-accessor")?
        .command(["/bin/sh"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .pty((80, 24))
        .spawn()
        .await?;
    let _: &mut Pty = container.pty();
    Ok(())
}

fn main() {}
