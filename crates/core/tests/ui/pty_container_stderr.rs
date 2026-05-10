use firkin_core::{Container, Rootfs, Stdio};

fn main() {
    let _container = Container::builder("pty-container-stderr")
        .unwrap()
        .command(["/bin/sh"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .pty((80, 24))
        .stderr(Stdio::piped())
        .spawn();
}
