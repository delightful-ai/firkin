//! Product pod route smoke coverage for runtime adapters.

#![cfg(any())]
// Scaffolding: this runtime integration test depends on firkin-single-node.
// It is disabled during the crates.io bootstrap to keep the publish graph acyclic.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use firkin_single_node::AppleVzLocalRuntimeDriver;
use {
    firkin_e2b_server::{LocalRuntimeBackend, PodRoutes},
    firkin_e2b_wire::{
        ControlPlaneMethod, ControlPlaneRequest, PodContainerCreateRequest, PodContainerOutput,
        PodCreateRequest, PodEmptyDir, PodInfo, PodStoreImageFormat, PodStoreOptions,
        PodVolumeMountRequest,
    },
};

#[test]
fn pod_wait_container_route_targets_container_output() {
    assert_eq!(
        PodRoutes::wait_container("pod/id", "agent sidecar"),
        "/pods/pod%2Fid/containers/agent%20sidecar/wait"
    );
}

#[test]
fn pod_container_capture_output_is_explicit_wire_state() {
    let request = PodContainerCreateRequest {
        name: "cli".to_owned(),
        template_id: "base".to_owned(),
        command: vec!["printf".to_owned(), "ready".to_owned()],
        env_vars: BTreeMap::new(),
        empty_dir_mounts: Vec::new(),
        capture_output: true,
    };
    let value = serde_json::to_value(&request).expect("encode pod container request");

    assert_eq!(value["captureOutput"], true);
    assert_eq!(
        serde_json::from_value::<PodContainerOutput>(serde_json::json!({
            "stdout": [114, 101, 97, 100, 121],
            "stderr": [],
            "exitCode": 0,
        }))
        .expect("decode pod container output"),
        PodContainerOutput::new(b"ready".to_vec(), Vec::new(), 0)
    );
}

#[tokio::test]
async fn apple_vz_product_pod_route_reports_asif_support() {
    let driver = AppleVzLocalRuntimeDriver::new("busybox");
    let backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");

    let capabilities = backend
        .preflight()
        .await
        .expect("Apple/VZ preflight should succeed without booting");

    assert!(
        capabilities
            .supported
            .contains(&"pod-store-asif".to_owned())
    );
    assert!(
        !capabilities
            .unsupported
            .iter()
            .any(|(name, _reason)| name == "pod-store-asif")
    );
}

#[tokio::test]
#[ignore = "signed live Apple/VZ pod route smoke; boots a VM"]
async fn live_apple_vz_product_pod_route_creates_adds_and_deletes() {
    live_product_pod_route_lifecycle("pod-live", PodStoreOptions::default()).await;
}

#[tokio::test]
#[ignore = "signed live Apple/VZ ASIF pod route smoke; boots a VM"]
async fn live_apple_vz_product_pod_route_uses_asif_pod_store() {
    live_product_pod_route_lifecycle(
        "pod-live-asif",
        PodStoreOptions {
            size_bytes: 512 * 1024 * 1024,
            image_format: PodStoreImageFormat::Asif,
            ..PodStoreOptions::default()
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "signed live Apple/VZ pod route emptyDir law smoke; boots a VM"]
async fn live_apple_vz_product_pod_added_writer_is_visible_to_later_reader() {
    let pod_id = format!("pod-live-added-writer-{}", uuid::Uuid::new_v4().simple());
    let driver = AppleVzLocalRuntimeDriver::new("busybox");
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");

    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::new(),
            empty_dirs: vec![PodEmptyDir {
                name: "work".to_owned(),
            }],
            pod_store: PodStoreOptions::default(),
            containers: vec![PodContainerCreateRequest {
                name: "keeper".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "sleep 30".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "work".to_owned(),
                    path: "/work".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            }],
        })
        .await
        .expect("create product pod for added-writer emptyDir law");

    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "writer".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "printf added-writer-ok >/work/marker; sleep 30".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "work".to_owned(),
                    path: "/work".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            },
        )
        .await
        .expect("add product pod writer");
    tokio::time::sleep(Duration::from_millis(500)).await;

    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "reader".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "cat /work/marker".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "work".to_owned(),
                    path: "/work".to_owned(),
                    read_only: true,
                }],
                capture_output: true,
            },
        )
        .await
        .expect("add product pod reader");
    let output = backend
        .wait_pod_container(&pod_id, "reader")
        .await
        .expect("wait product pod reader");

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete product pod");
    assert_eq!(
        output,
        PodContainerOutput::new(b"added-writer-ok".to_vec(), Vec::new(), 0)
    );
}

async fn live_product_pod_route_lifecycle(pod_prefix: &str, pod_store: PodStoreOptions) {
    let pod_id = format!("{pod_prefix}-{}", uuid::Uuid::new_v4().simple());
    let image_format = pod_store.image_format;
    let size_bytes = pod_store.size_bytes;
    let driver = AppleVzLocalRuntimeDriver::new("busybox");
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");

    let create_start = Instant::now();
    let create = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, PodRoutes::create())
                .with_json(&PodCreateRequest {
                    pod_id: Some(pod_id.clone()),
                    timeout: Some(300),
                    metadata: BTreeMap::new(),
                    empty_dirs: vec![PodEmptyDir {
                        name: "work".to_owned(),
                    }],
                    pod_store,
                    containers: vec![PodContainerCreateRequest {
                        name: "agent".to_owned(),
                        template_id: "base".to_owned(),
                        command: vec![
                            "/bin/sh".to_owned(),
                            "-lc".to_owned(),
                            "printf product-pod-ok >/work/marker; sleep 30".to_owned(),
                        ],
                        env_vars: BTreeMap::new(),
                        empty_dir_mounts: vec![PodVolumeMountRequest {
                            name: "work".to_owned(),
                            path: "/work".to_owned(),
                            read_only: false,
                        }],
                        capture_output: false,
                    }],
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let create_elapsed = create_start.elapsed();
    eprintln!(
        "product pod create image_format={image_format:?} size_bytes={size_bytes} elapsed_ms={}",
        create_elapsed.as_millis()
    );
    assert_eq!(create.status, 200);
    assert_eq!(create.decode_json::<PodInfo>().unwrap().pod_id, pod_id);
    let usage_before_sidecar = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before sidecar proof");

    let added = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, PodRoutes::add_container(&pod_id))
                .with_json(&PodContainerCreateRequest {
                    name: "sidecar".to_owned(),
                    template_id: "base".to_owned(),
                    command: vec![
                        "/bin/sh".to_owned(),
                        "-lc".to_owned(),
                        "test \"$(cat /shared/marker)\" = product-pod-ok && dd if=/dev/zero of=/shared/sidecar-proof bs=1024 count=4096 2>/dev/null && sleep 30"
                            .to_owned(),
                    ],
                    env_vars: BTreeMap::new(),
                    empty_dir_mounts: vec![PodVolumeMountRequest {
                        name: "work".to_owned(),
                        path: "/shared".to_owned(),
                        read_only: true,
                    }],
                    capture_output: false,
                })
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(added.status, 200);
    let usage_after_sidecar =
        wait_for_pod_store_growth(&metrics_driver, &pod_id, usage_before_sidecar).await;
    assert!(
        usage_after_sidecar > usage_before_sidecar,
        "sidecar did not prove marker readability through pod-store growth: before={usage_before_sidecar} after={usage_after_sidecar}"
    );

    let removed = backend
        .handle_control_plane(ControlPlaneRequest::new(
            ControlPlaneMethod::Delete,
            PodRoutes::delete_container(&pod_id, "sidecar"),
        ))
        .await
        .unwrap();
    assert_eq!(removed.status, 204);

    let deleted = backend
        .handle_control_plane(ControlPlaneRequest::new(
            ControlPlaneMethod::Delete,
            PodRoutes::delete(&pod_id),
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status, 204);
}

async fn wait_for_pod_store_growth(
    driver: &AppleVzLocalRuntimeDriver,
    pod_id: &str,
    baseline: u64,
) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let usage = driver
            .pod_store_used_bytes(pod_id)
            .await
            .expect("read pod-store usage during sidecar proof");
        if usage > baseline || Instant::now() >= deadline {
            return usage;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
