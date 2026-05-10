//! command — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::build::TemplateBuildError;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::process::Command;
pub(crate) fn run(
    command: &mut Command,
    operation: &'static str,
) -> Result<(), TemplateBuildError> {
    let status = command
        .status()
        .map_err(|source| TemplateBuildError::Io { operation, source })?;
    if status.success() {
        Ok(())
    } else {
        Err(TemplateBuildError::Command { operation, status })
    }
}
pub(crate) fn run_shell(
    command: &str,
    current_dir: &Path,
    operation: &'static str,
) -> Result<(), TemplateBuildError> {
    run(
        Command::new("/bin/sh")
            .arg("-lc")
            .arg(command)
            .current_dir(current_dir),
        operation,
    )
}
