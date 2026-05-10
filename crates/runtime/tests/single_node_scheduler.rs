//! Scheduler tests for the single-node runtime.

#![cfg(any())]
// Scaffolding: this runtime integration test depends on firkin-single-node.
// It is disabled during the crates.io bootstrap to keep the publish graph acyclic.

use firkin_single_node::{Error, SandboxResources, SingleNodeScheduler, SingleNodeSchedulerConfig};
use firkin_types::Size;

#[test]
fn scheduler_admits_idempotently_and_releases_capacity() {
    let scheduler = SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        2,
        SandboxResources::new(4, Size::gib(8)),
    ));
    let resources = SandboxResources::new(2, Size::gib(2));

    scheduler.admit("sbx-one", resources).unwrap();
    scheduler.admit("sbx-one", resources).unwrap();

    assert_eq!(scheduler.admitted_len().unwrap(), 1);
    assert_eq!(
        scheduler.available_resources().unwrap(),
        SandboxResources::new(2, Size::gib(6))
    );

    scheduler.release("sbx-one").unwrap();

    assert_eq!(scheduler.admitted_len().unwrap(), 0);
    assert_eq!(
        scheduler.available_resources().unwrap(),
        SandboxResources::new(4, Size::gib(8))
    );
}

#[test]
fn scheduler_rejects_max_sessions_before_runtime_boot() {
    let scheduler = SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        1,
        SandboxResources::new(4, Size::gib(8)),
    ));

    scheduler
        .admit("sbx-one", SandboxResources::new(2, Size::gib(1)))
        .unwrap();

    let result = scheduler.admit("sbx-two", SandboxResources::new(1, Size::gib(1)));

    assert!(matches!(
        result,
        Err(Error::CapacityRejected(message))
            if message.contains("max sessions")
                && message.contains("sbx-two")
                && !message.contains("cube")
                && !message.contains("firkin-apple-vz")
    ));
}

#[test]
fn scheduler_rejects_cpu_and_memory_capacity() {
    let cpu_scheduler = SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        4,
        SandboxResources::new(4, Size::gib(8)),
    ));
    cpu_scheduler
        .admit("sbx-one", SandboxResources::new(3, Size::gib(1)))
        .unwrap();

    assert!(matches!(
        cpu_scheduler.admit("sbx-two", SandboxResources::new(2, Size::gib(1))),
        Err(Error::CapacityRejected(message))
            if message.contains("CPU") && message.contains("requested 2")
    ));

    let memory_scheduler = SingleNodeScheduler::new(SingleNodeSchedulerConfig::new(
        4,
        SandboxResources::new(4, Size::gib(8)),
    ));
    memory_scheduler
        .admit("sbx-one", SandboxResources::new(1, Size::gib(7)))
        .unwrap();

    assert!(matches!(
        memory_scheduler.admit("sbx-two", SandboxResources::new(1, Size::gib(2))),
        Err(Error::CapacityRejected(message))
            if message.contains("memory") && message.contains("sbx-two")
    ));
}
