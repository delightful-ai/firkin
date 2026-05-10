use firkin_core::{ExecConfig, Stdio};

fn main() {
    let _config = ExecConfig::builder()
        .command(["/bin/sh"])
        .pty((80, 24))
        .stdout(Stdio::piped())
        .build();
}
