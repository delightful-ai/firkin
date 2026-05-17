use std::fs::OpenOptions;
use std::net::Ipv4Addr;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::path::Path;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, ClassType, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSData, NSError, NSFileHandle, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_virtualization::{
    VZBootLoader, VZDirectoryShare, VZDirectorySharingDeviceConfiguration,
    VZDiskImageStorageDeviceAttachment, VZFileHandleSerialPortAttachment,
    VZGenericMachineIdentifier, VZGenericPlatformConfiguration, VZLinuxBootLoader,
    VZLinuxRosettaAvailability, VZLinuxRosettaDirectoryShare, VZMACAddress,
    VZNATNetworkDeviceAttachment, VZNetworkDeviceAttachment, VZNetworkDeviceConfiguration,
    VZPlatformConfiguration, VZSerialPortAttachment, VZSerialPortConfiguration,
    VZSocketDeviceConfiguration, VZStorageDeviceAttachment, VZStorageDeviceConfiguration,
    VZVirtioBlockDeviceConfiguration, VZVirtioConsoleDeviceSerialPortConfiguration,
    VZVirtioFileSystemDeviceConfiguration, VZVirtioNetworkDeviceConfiguration,
    VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtioSocketDeviceConfiguration,
    VZVirtioSocketListener, VZVirtioSocketListenerDelegate, VZVirtualMachine,
    VZVirtualMachineConfiguration, VZVirtualMachineState, VZVmnetNetworkDeviceAttachment,
};
use tokio::sync::{mpsc, oneshot};

use crate::{
    BlockDevice, BootLog, Error, KernelImage, Network, NetworkInterface, Result, VmConfig, VmPhase,
    VsockListener, VsockPort, VsockStream, invalid_config,
};

const BASE_KERNEL_COMMAND_LINE: &str =
    "console=hvc0 root=/dev/vda rootfstype=ext4 ro init=/sbin/vminitd panic=-1";

type SerialDeviceParts = (
    Option<Retained<NSFileHandle>>,
    Option<Retained<VZFileHandleSerialPortAttachment>>,
    Option<Retained<VZVirtioConsoleDeviceSerialPortConfiguration>>,
);

type StorageDeviceParts = (
    Vec<Retained<VZDiskImageStorageDeviceAttachment>>,
    Vec<Retained<VZVirtioBlockDeviceConfiguration>>,
);

type RosettaDeviceParts = (
    Option<Retained<VZLinuxRosettaDirectoryShare>>,
    Option<Retained<VZVirtioFileSystemDeviceConfiguration>>,
);

pub(crate) fn new_machine_identifier_data() -> Vec<u8> {
    // SAFETY: `new` creates a fresh Virtualization.framework identifier.
    let identifier = unsafe { VZGenericMachineIdentifier::new() };
    // SAFETY: This is a read-only copy of the opaque identifier bytes.
    unsafe { identifier.dataRepresentation() }.to_vec()
}

pub(crate) fn new_mac_address_string() -> String {
    // SAFETY: Virtualization.framework creates and returns a retained MAC
    // address object, and `string` copies its textual representation.
    let mac = unsafe { VZMACAddress::randomLocallyAdministeredAddress() };
    unsafe { mac.string() }.to_string()
}

#[derive(Debug)]
struct VmnetAllocation {
    _configuration: VmnetNetworkConfigurationRef,
    _network: VmnetNetworkRef,
}

#[derive(Debug)]
#[allow(dead_code)]
enum NetworkAttachment {
    Nat(Retained<VZNATNetworkDeviceAttachment>),
    Vmnet(Retained<VZVmnetNetworkDeviceAttachment>),
}

type NetworkDeviceParts = (
    Vec<VmnetAllocation>,
    Vec<NetworkAttachment>,
    Vec<Retained<VZMACAddress>>,
    Vec<Retained<VZVirtioNetworkDeviceConfiguration>>,
    Vec<NetworkInterface>,
);

type StartCompletion = RcBlock<dyn Fn(*mut NSError)>;
type ConnectCompletion = RcBlock<dyn Fn(*mut VZVirtioSocketConnection, *mut NSError)>;
type ListenerSender = mpsc::Sender<firkin_vsock::Result<(OwnedFd, crate::VsockPeer)>>;

#[repr(C)]
#[derive(Debug)]
struct VmnetNetwork {
    _priv: [u8; 0],
}

unsafe impl RefEncode for VmnetNetwork {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("vmnet_network", &[]));
}

#[repr(C)]
#[derive(Debug)]
struct VmnetNetworkConfiguration {
    _priv: [u8; 0],
}

unsafe impl RefEncode for VmnetNetworkConfiguration {
    const ENCODING_REF: Encoding =
        Encoding::Pointer(&Encoding::Struct("vmnet_network_configuration", &[]));
}

type VmnetNetworkRef = *mut VmnetNetwork;
type VmnetNetworkConfigurationRef = *mut VmnetNetworkConfiguration;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct InAddr {
    s_addr: u32,
}

unsafe impl RefEncode for InAddr {
    const ENCODING_REF: Encoding =
        Encoding::Pointer(&Encoding::Struct("in_addr", &[Encoding::UInt]));
}

const VMNET_SHARED_MODE: u32 = 1001;
const VMNET_SUCCESS: u32 = 1000;

#[link(name = "vmnet", kind = "framework")]
unsafe extern "C" {
    fn vmnet_network_configuration_create(
        mode: u32,
        status: *mut u32,
    ) -> VmnetNetworkConfigurationRef;
    fn vmnet_network_configuration_disable_dhcp(cfg: VmnetNetworkConfigurationRef);
    fn vmnet_network_configuration_set_ipv4_subnet(
        cfg: VmnetNetworkConfigurationRef,
        subnet: *const InAddr,
        mask: *const InAddr,
    ) -> u32;
    fn vmnet_network_create(cfg: VmnetNetworkConfigurationRef, status: *mut u32)
    -> VmnetNetworkRef;
    fn vmnet_network_get_ipv4_subnet(net: VmnetNetworkRef, subnet: *mut InAddr, mask: *mut InAddr);
}

#[derive(Clone, Copy, Debug)]
enum VmOperation {
    Start,
    Stop,
    Pause,
    Resume,
    #[cfg(feature = "snapshot")]
    SaveSnapshot,
    #[cfg(feature = "snapshot")]
    RestoreSnapshot,
}

impl VmOperation {
    fn error(self, reason: String) -> Error {
        match self {
            Self::Start => Error::Start { reason },
            Self::Stop => Error::Stop { reason },
            Self::Pause => Error::Pause { reason },
            Self::Resume => Error::Resume { reason },
            #[cfg(feature = "snapshot")]
            Self::SaveSnapshot => Error::Snapshot {
                operation: "save",
                reason,
            },
            #[cfg(feature = "snapshot")]
            Self::RestoreSnapshot => Error::Snapshot {
                operation: "restore",
                reason,
            },
        }
    }
}

#[derive(Debug)]
struct VzSend<T>(T);

// SAFETY: Values wrapped in VzSend are retained Objective-C objects or blocks
// that are created for one VZ runtime and only operated on through that
// runtime's serial dispatch queue. The wrapper is never exposed publicly.
unsafe impl<T> Send for VzSend<T> {}

// SAFETY: Shared references are used only to retain/drop handles or enqueue
// operations onto the serial VZ queue; direct framework calls stay inside
// queue closures.
unsafe impl<T> Sync for VzSend<T> {}

impl<T: Clone> Clone for VzSend<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl VzSend<Retained<VZVirtualMachine>> {
    fn phase(&self) -> VmPhase {
        // SAFETY: This is a read-only query. Callers invoke it synchronously on
        // the VM queue or before any asynchronous operation has started.
        let state = unsafe { self.0.state() };
        vm_phase(state)
    }

    fn can_start(&self) -> bool {
        // SAFETY: This is a read-only query on a VM owned by this private
        // runtime wrapper.
        unsafe { self.0.canStart() }
    }

    fn can_stop(&self) -> bool {
        // SAFETY: This is a read-only query run on the VM queue.
        unsafe { self.0.canStop() }
    }

    fn can_pause(&self) -> bool {
        // SAFETY: This is a read-only query run on the VM queue.
        unsafe { self.0.canPause() }
    }

    fn can_resume(&self) -> bool {
        // SAFETY: This is a read-only query run on the VM queue.
        unsafe { self.0.canResume() }
    }

    fn can_request_stop(&self) -> bool {
        // SAFETY: This is a read-only query run on the VM queue.
        unsafe { self.0.canRequestStop() }
    }

    fn start_with_completion(&self, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            self.0.startWithCompletionHandler(&completion.0);
        }
    }

    fn stop_with_completion(&self, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            self.0.stopWithCompletionHandler(&completion.0);
        }
    }

    fn pause_with_completion(&self, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            self.0.pauseWithCompletionHandler(&completion.0);
        }
    }

    fn resume_with_completion(&self, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            self.0.resumeWithCompletionHandler(&completion.0);
        }
    }

    #[cfg(feature = "snapshot")]
    fn save_snapshot_with_completion(&self, url: &NSURL, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            let _: () = msg_send![
                &self.0,
                saveMachineStateToURL: url,
                completionHandler: &*completion.0,
            ];
        }
    }

    #[cfg(feature = "snapshot")]
    fn restore_snapshot_with_completion(&self, url: &NSURL, completion: &VzSend<StartCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            let _: () = msg_send![
                &self.0,
                restoreMachineStateFromURL: url,
                completionHandler: &*completion.0,
            ];
        }
    }

    fn request_stop(&self) -> Result<()> {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue.
        unsafe {
            self.0.requestStopWithError().map_err(|error| Error::Stop {
                reason: nserror_desc(&error),
            })
        }
    }
}

impl VzSend<Retained<VZVirtioSocketDevice>> {
    fn connect_to_port(&self, port: VsockPort, completion: &VzSend<ConnectCompletion>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue, and the completion block is retained until VZ replies.
        unsafe {
            self.0
                .connectToPort_completionHandler(port.get(), &completion.0);
        }
    }

    fn set_listener(&self, port: VsockPort, listener: &VzSend<Retained<VZVirtioSocketListener>>) {
        // SAFETY: Callers invoke this method only from the VM's private serial
        // queue. The listener and delegate are retained in VmRuntime.
        unsafe {
            self.0.setSocketListener_forPort(&listener.0, port.get());
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct VmRuntime {
    inner: Arc<VmRuntimeInner>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct VmRuntimeInner {
    vm: VzSend<Retained<VZVirtualMachine>>,
    socket_device: VzSend<Retained<VZVirtioSocketDevice>>,
    queue: DispatchRetained<DispatchQueue>,
    listeners: Mutex<Vec<VzSend<RegisteredListener>>>,
    _configuration: VzSend<VzConfiguration>,
}

impl VmRuntime {
    pub(crate) fn listen(&self, port: VsockPort) -> VsockListener {
        let (sender, receiver) = mpsc::channel(16);
        let delegate = ListenerDelegate::new(port, sender);
        let listener = VzSend(new_socket_listener());
        let proto: &ProtocolObject<dyn VZVirtioSocketListenerDelegate> =
            ProtocolObject::from_ref(&*delegate);
        // SAFETY: The delegate object is retained in VmRuntime because
        // VZVirtioSocketListener stores it weakly.
        unsafe {
            listener.0.setDelegate(Some(proto));
        }

        let socket_device = self.inner.socket_device.clone();
        let listener_for_queue = listener.clone();
        self.inner.queue.exec_sync(move || {
            socket_device.set_listener(port, &listener_for_queue);
        });

        self.inner
            .listeners
            .lock()
            .expect("listeners mutex")
            .push(VzSend(RegisteredListener {
                _listener: listener,
                _delegate: VzSend(delegate),
            }));

        VsockListener::from_receiver(receiver)
    }

    pub(crate) async fn dial(&self, port: VsockPort) -> Result<VsockStream> {
        let (sender, receiver) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        let completion_sender = Arc::clone(&sender);
        let completion = VzSend(RcBlock::new(
            move |connection: *mut VZVirtioSocketConnection, error: *mut NSError| {
                let result = dial_completion_result(port, connection, error);
                if let Some(sender) = completion_sender.lock().expect("completion mutex").take() {
                    let _ = sender.send(result);
                }
            },
        ));
        let completion_for_dispatch = completion.clone();
        let socket_device = self.inner.socket_device.clone();

        self.inner.queue.exec_async(move || {
            socket_device.connect_to_port(port, &completion_for_dispatch);
        });

        let fd = receiver.await.map_err(|_| Error::Dial {
            port,
            reason: "connect completion channel closed before VZ replied".into(),
        })?;
        drop(completion);
        VsockStream::from_owned_fd(fd?).map_err(|error| Error::Dial {
            port,
            reason: error.to_string(),
        })
    }

    pub(crate) async fn stop(&self) -> Result<()> {
        if !self.vm_bool(VzSend::can_stop).await? {
            return Ok(());
        }
        self.completion_operation(VmOperation::Stop, |vm, completion| {
            vm.stop_with_completion(&completion);
        })
        .await
    }

    pub(crate) async fn request_stop(&self) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        let vm = self.inner.vm.clone();
        self.inner.queue.exec_async(move || {
            let result = if vm.can_request_stop() {
                vm.request_stop()
            } else {
                Ok(())
            };
            let _ = sender.send(result);
        });
        receiver.await.map_err(|_| Error::Stop {
            reason: "request-stop completion channel closed before VZ replied".into(),
        })?
    }

    pub(crate) async fn pause(&self) -> Result<()> {
        if !self.vm_bool(VzSend::can_pause).await? {
            return Err(Error::Pause {
                reason: "VM is not in a pausable state".into(),
            });
        }
        self.completion_operation(VmOperation::Pause, |vm, completion| {
            vm.pause_with_completion(&completion);
        })
        .await
    }

    pub(crate) async fn resume(&self) -> Result<()> {
        if !self.vm_bool(VzSend::can_resume).await? {
            return Err(Error::Resume {
                reason: "VM is not in a resumable state".into(),
            });
        }
        self.completion_operation(VmOperation::Resume, |vm, completion| {
            vm.resume_with_completion(&completion);
        })
        .await
    }

    #[cfg(feature = "snapshot")]
    pub(crate) async fn save_snapshot(&self, path: &Path) -> Result<()> {
        let was_paused = self.phase_blocking() == VmPhase::Paused;
        if !was_paused {
            self.pause().await?;
        }

        let url = ns_url_file_for_create(path)?;
        let save_result = self
            .completion_operation(VmOperation::SaveSnapshot, move |vm, completion| {
                vm.save_snapshot_with_completion(&url, &completion);
            })
            .await;

        if !was_paused {
            let resume_result = self.resume().await;
            save_result?;
            resume_result?;
            return Ok(());
        }

        save_result
    }

    pub(crate) fn phase_blocking(&self) -> VmPhase {
        let vm = self.inner.vm.clone();
        let phase = Arc::new(Mutex::new(VmPhase::Stopping));
        let phase_for_queue = Arc::clone(&phase);
        self.inner.queue.exec_sync(move || {
            *phase_for_queue.lock().expect("phase mutex") = vm.phase();
        });
        *phase.lock().expect("phase mutex")
    }

    async fn vm_bool(&self, f: fn(&VzSend<Retained<VZVirtualMachine>>) -> bool) -> Result<bool> {
        let (sender, receiver) = oneshot::channel();
        let vm = self.inner.vm.clone();
        self.inner.queue.exec_async(move || {
            let _ = sender.send(f(&vm));
        });
        receiver.await.map_err(|_| Error::UnclassifiedVz {
            reason: "VM queue closed before state query completed".into(),
        })
    }

    async fn completion_operation<F>(&self, operation: VmOperation, dispatch: F) -> Result<()>
    where
        F: FnOnce(VzSend<Retained<VZVirtualMachine>>, VzSend<StartCompletion>) + Send + 'static,
    {
        let vm = self.inner.vm.clone();
        let (sender, receiver) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        let completion_sender = Arc::clone(&sender);
        let completion = VzSend(RcBlock::new(move |error: *mut NSError| {
            let result = completion_result(operation, error);
            if let Some(sender) = completion_sender.lock().expect("completion mutex").take() {
                let _ = sender.send(result);
            }
        }));
        let completion_for_dispatch = completion.clone();

        self.inner.queue.exec_async(move || {
            dispatch(vm, completion_for_dispatch);
        });

        let result = receiver
            .await
            .map_err(|_| operation.error("completion channel closed before VZ replied".into()))?;
        drop(completion);
        result
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct VzConfiguration {
    configuration: Retained<VZVirtualMachineConfiguration>,
    _boot_loader: Retained<VZLinuxBootLoader>,
    _platform: Retained<VZGenericPlatformConfiguration>,
    _storage_attachments: Vec<Retained<VZDiskImageStorageDeviceAttachment>>,
    _storage_devices: Vec<Retained<VZVirtioBlockDeviceConfiguration>>,
    _vmnet_allocations: Vec<VmnetAllocation>,
    _network_attachments: Vec<NetworkAttachment>,
    _network_macs: Vec<Retained<VZMACAddress>>,
    _network_devices: Vec<Retained<VZVirtioNetworkDeviceConfiguration>>,
    network_interfaces: Vec<NetworkInterface>,
    _rosetta_share: Option<Retained<VZLinuxRosettaDirectoryShare>>,
    _rosetta_device: Option<Retained<VZVirtioFileSystemDeviceConfiguration>>,
    _socket_device: Retained<VZVirtioSocketDeviceConfiguration>,
    _serial_file: Option<Retained<NSFileHandle>>,
    _serial_attachment: Option<Retained<VZFileHandleSerialPortAttachment>>,
    _serial_device: Option<Retained<VZVirtioConsoleDeviceSerialPortConfiguration>>,
}

impl VzConfiguration {
    #[allow(dead_code)]
    pub(crate) fn configuration(&self) -> &VZVirtualMachineConfiguration {
        &self.configuration
    }

    pub(crate) fn network_interfaces(&self) -> &[NetworkInterface] {
        &self.network_interfaces
    }
}

#[derive(Debug)]
pub(crate) struct VzStart {
    configuration: VzSend<VzConfiguration>,
    network_interfaces: Vec<NetworkInterface>,
}

impl VzStart {
    pub(crate) fn network_interfaces(&self) -> &[NetworkInterface] {
        &self.network_interfaces
    }
}

pub(crate) fn prepare_start(config: &VmConfig) -> Result<VzStart> {
    let configuration = VzSend(build_configuration(config)?);
    let network_interfaces = configuration.0.network_interfaces().to_vec();
    Ok(VzStart {
        configuration,
        network_interfaces,
    })
}

#[allow(dead_code)]
pub(crate) fn build_configuration(config: &VmConfig) -> Result<VzConfiguration> {
    let kernel = match config.kernel() {
        KernelImage::File(path) => path,
        KernelImage::Bundled => return invalid_config("VZ boot requires a resolved kernel file"),
    };
    let init_block = config.init_block().ok_or_else(|| Error::InvalidConfig {
        reason: "VZ boot requires a resolved init block".into(),
    })?;

    let configuration = new_configuration();
    let boot_loader = linux_boot_loader(kernel, config.cmdline_extra())?;
    let platform = generic_platform(config)?;
    let (serial_file, serial_attachment, serial_device) = serial_device(config.boot_log())?;
    let (storage_attachments, storage_devices) =
        storage_devices(init_block, config.block_devices())?;
    let (vmnet_allocations, network_attachments, network_macs, network_devices, network_interfaces) =
        network_devices(config.networks(), config.network_macs())?;
    let (rosetta_share, rosetta_device) = rosetta_device(config.rosetta_enabled())?;
    let socket_device = socket_device();

    // SAFETY: All objects passed to the Objective-C setters are freshly
    // initialized Virtualization.framework objects, and the arrays are only
    // borrowed for the duration of the setter calls, which copy them.
    unsafe {
        let boot_loader_parent: &VZBootLoader = (*boot_loader).as_super();
        configuration.setBootLoader(Some(boot_loader_parent));

        let platform_parent: &VZPlatformConfiguration = (*platform).as_super();
        configuration.setPlatform(platform_parent);
        configuration.setCPUCount(config.cpus().get() as usize);
        configuration.setMemorySize(config.memory().as_bytes());

        if let Some(serial_device) = serial_device.as_ref() {
            let serial_parent: &VZSerialPortConfiguration = (**serial_device).as_super();
            configuration.setSerialPorts(&NSArray::from_slice(&[serial_parent]));
        }

        let storage_refs: Vec<&VZStorageDeviceConfiguration> = storage_devices
            .iter()
            .map(|device| {
                let parent: &VZStorageDeviceConfiguration = (**device).as_super();
                parent
            })
            .collect();
        configuration.setStorageDevices(&NSArray::from_slice(&storage_refs));

        if !network_devices.is_empty() {
            let network_refs: Vec<&VZNetworkDeviceConfiguration> = network_devices
                .iter()
                .map(|device| {
                    let parent: &VZNetworkDeviceConfiguration = (**device).as_super();
                    parent
                })
                .collect();
            configuration.setNetworkDevices(&NSArray::from_slice(&network_refs));
        }

        if let Some(rosetta_device) = rosetta_device.as_ref() {
            let rosetta_parent: &VZDirectorySharingDeviceConfiguration =
                (**rosetta_device).as_super();
            configuration.setDirectorySharingDevices(&NSArray::from_slice(&[rosetta_parent]));
        }

        let socket_parent: &VZSocketDeviceConfiguration = (*socket_device).as_super();
        configuration.setSocketDevices(&NSArray::from_slice(&[socket_parent]));

        configuration
            .validateWithError()
            .map_err(|error| Error::UnclassifiedVz {
                reason: format!("configuration validation failed: {}", nserror_desc(&error)),
            })?;
    }

    Ok(VzConfiguration {
        configuration,
        _boot_loader: boot_loader,
        _platform: platform,
        _storage_attachments: storage_attachments,
        _storage_devices: storage_devices,
        _vmnet_allocations: vmnet_allocations,
        _network_attachments: network_attachments,
        _network_macs: network_macs,
        _network_devices: network_devices,
        network_interfaces,
        _rosetta_share: rosetta_share,
        _rosetta_device: rosetta_device,
        _socket_device: socket_device,
        _serial_file: serial_file,
        _serial_attachment: serial_attachment,
        _serial_device: serial_device,
    })
}

#[allow(dead_code)]
pub(crate) async fn start(start: VzStart) -> Result<VmRuntime> {
    let configuration = start.configuration;
    let queue = DispatchQueue::new("dev.firkin.vmm.vm", DispatchQueueAttr::SERIAL);

    // SAFETY: The configuration has already passed VZ validation, and the
    // queue is a private serial queue retained by VmRuntime for the VM life.
    let vm = VzSend(unsafe {
        VZVirtualMachine::initWithConfiguration_queue(
            VZVirtualMachine::alloc(),
            configuration.0.configuration(),
            &queue,
        )
    });
    let socket_device_slot = Arc::new(Mutex::new(None));
    let socket_device_for_queue = Arc::clone(&socket_device_slot);
    let vm_for_socket = vm.clone();
    queue.exec_sync(move || {
        let result = socket_device_for_vm(&vm_for_socket).and_then(|socket_device| {
            if vm_for_socket.can_start() {
                Ok(socket_device)
            } else {
                Err(Error::Start {
                    reason: "new VZVirtualMachine cannot start".into(),
                })
            }
        });
        *socket_device_for_queue.lock().expect("socket device mutex") = Some(result);
    });
    let socket_device = socket_device_slot
        .lock()
        .expect("socket device mutex")
        .take()
        .expect("socket device lookup completed")?;

    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let completion_sender = Arc::clone(&sender);
    let completion = VzSend(RcBlock::new(move |error: *mut NSError| {
        let result = completion_result(VmOperation::Start, error);
        if let Some(sender) = completion_sender.lock().expect("completion mutex").take() {
            let _ = sender.send(result);
        }
    }));
    let completion_for_start = completion.clone();
    let vm_for_start = vm.clone();
    queue.exec_async(move || {
        vm_for_start.start_with_completion(&completion_for_start);
    });

    let start_result = receiver.await.map_err(|_| Error::Start {
        reason: "start completion channel closed before VZ replied".into(),
    })?;
    drop(completion);
    start_result?;

    Ok(VmRuntime {
        inner: Arc::new(VmRuntimeInner {
            vm,
            socket_device,
            queue,
            listeners: Mutex::new(Vec::new()),
            _configuration: configuration,
        }),
    })
}

#[allow(dead_code)]
#[cfg(feature = "snapshot")]
pub(crate) async fn restore(prepared: VzStart, snapshot_path: &Path) -> Result<VmRuntime> {
    let configuration = prepared.configuration;
    let queue = DispatchQueue::new("dev.firkin.vmm.vm", DispatchQueueAttr::SERIAL);
    let snapshot_url = ns_url_file(snapshot_path)?;

    // SAFETY: The configuration has already passed VZ validation, and the
    // queue is a private serial queue retained by VmRuntime for the VM life.
    let vm = VzSend(unsafe {
        VZVirtualMachine::initWithConfiguration_queue(
            VZVirtualMachine::alloc(),
            configuration.0.configuration(),
            &queue,
        )
    });
    let socket_device_slot = Arc::new(Mutex::new(None));
    let socket_device_for_queue = Arc::clone(&socket_device_slot);
    let vm_for_socket = vm.clone();
    queue.exec_sync(move || {
        let result = socket_device_for_vm(&vm_for_socket);
        *socket_device_for_queue.lock().expect("socket device mutex") = Some(result);
    });
    let socket_device = socket_device_slot
        .lock()
        .expect("socket device mutex")
        .take()
        .expect("socket device lookup completed")?;

    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let completion_sender = Arc::clone(&sender);
    let completion = VzSend(RcBlock::new(move |error: *mut NSError| {
        let result = completion_result(VmOperation::RestoreSnapshot, error);
        if let Some(sender) = completion_sender.lock().expect("completion mutex").take() {
            let _ = sender.send(result);
        }
    }));
    let completion_for_restore = completion.clone();
    let vm_for_restore = vm.clone();
    queue.exec_async(move || {
        vm_for_restore.restore_snapshot_with_completion(&snapshot_url, &completion_for_restore);
    });

    let restore_result = receiver.await.map_err(|_| Error::Snapshot {
        operation: "restore",
        reason: "restore completion channel closed before VZ replied".into(),
    })?;
    drop(completion);
    restore_result?;

    let runtime = VmRuntime {
        inner: Arc::new(VmRuntimeInner {
            vm,
            socket_device,
            queue,
            listeners: Mutex::new(Vec::new()),
            _configuration: configuration,
        }),
    };
    runtime.resume().await?;
    Ok(runtime)
}

fn new_socket_listener() -> Retained<VZVirtioSocketListener> {
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    unsafe { VZVirtioSocketListener::init(VZVirtioSocketListener::alloc()) }
}

#[derive(Debug)]
struct RegisteredListener {
    _listener: VzSend<Retained<VZVirtioSocketListener>>,
    _delegate: VzSend<Retained<ListenerDelegate>>,
}

#[derive(Debug)]
struct ListenerDelegateIvars {
    port: u32,
    sender: ListenerSender,
}

define_class!(
    // SAFETY: The delegate only duplicates an fd and pushes it through a
    // tokio channel. All Objective-C references are callback-borrowed.
    #[unsafe(super(NSObject))]
    #[ivars = ListenerDelegateIvars]
    #[derive(Debug)]
    struct ListenerDelegate;

    unsafe impl NSObjectProtocol for ListenerDelegate {}

    unsafe impl VZVirtioSocketListenerDelegate for ListenerDelegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:fromSocketDevice:))]
        #[allow(non_snake_case)]
        fn listener_shouldAcceptNewConnection_fromSocketDevice(
            &self,
            _listener: &VZVirtioSocketListener,
            connection: &VZVirtioSocketConnection,
            _socket_device: &VZVirtioSocketDevice,
        ) -> bool {
            let ivars = self.ivars();
            let port = VsockPort::new(ivars.port);
            match accepted_fd(port, connection) {
                Ok(fd) => {
                    let peer = crate::VsockPeer::new(3, port);
                    ivars.sender.try_send(Ok((fd, peer))).is_ok()
                }
                Err(error) => ivars.sender.try_send(Err(error)).is_ok(),
            }
        }
    }
);

impl ListenerDelegate {
    fn new(port: VsockPort, sender: ListenerSender) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ListenerDelegateIvars {
            port: port.get(),
            sender,
        });
        // SAFETY: `set_ivars` initialized all ivars before NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

fn accepted_fd(
    port: VsockPort,
    connection: &VZVirtioSocketConnection,
) -> firkin_vsock::Result<OwnedFd> {
    // SAFETY: The callback-borrowed connection is valid for the duration of
    // this delegate method. We duplicate the fd before returning because VZ
    // owns the original.
    let fd = unsafe { connection.fileDescriptor() };
    if fd < 0 {
        return Err(firkin_vsock::Error::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            format!("VZ listener on port {} returned a closed fd", port.get()),
        )));
    }
    // SAFETY: `dup` does not take ownership of `fd`; it returns a new fd on
    // success which becomes owned below.
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        return Err(firkin_vsock::Error::Io(std::io::Error::last_os_error()));
    }
    // SAFETY: `duplicated` is a fresh fd returned by `dup`, so this transfers
    // ownership into OwnedFd exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

fn socket_device_for_vm(
    vm: &VzSend<Retained<VZVirtualMachine>>,
) -> Result<VzSend<Retained<VZVirtioSocketDevice>>> {
    // SAFETY: The VM configuration always installs exactly one virtio socket
    // device before constructing VZVirtualMachine, so the returned first socket
    // device has the concrete VZVirtioSocketDevice class.
    let socket_devices = unsafe { vm.0.socketDevices() };
    let Some(socket_device) = socket_devices.to_vec().into_iter().next() else {
        return Err(Error::UnclassifiedVz {
            reason: "VZ VM did not expose a socket device".into(),
        });
    };
    Ok(VzSend(unsafe { Retained::cast_unchecked(socket_device) }))
}

fn new_configuration() -> Retained<VZVirtualMachineConfiguration> {
    // SAFETY: `alloc` is immediately consumed by the designated `init` for
    // VZVirtualMachineConfiguration.
    unsafe { VZVirtualMachineConfiguration::init(VZVirtualMachineConfiguration::alloc()) }
}

fn linux_boot_loader(kernel: &Path, extra: &[String]) -> Result<Retained<VZLinuxBootLoader>> {
    let kernel_url = ns_url_file(kernel)?;
    let command_line = kernel_command_line(extra);

    // SAFETY: The URL is a canonical local file URL, and `alloc` is consumed
    // by the designated Linux boot-loader initializer.
    let boot_loader =
        unsafe { VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url) };
    let command_line = NSString::from_str(&command_line);

    // SAFETY: Virtualization.framework copies the NSString command line.
    unsafe {
        boot_loader.setCommandLine(&command_line);
    }

    Ok(boot_loader)
}

fn generic_platform(config: &VmConfig) -> Result<Retained<VZGenericPlatformConfiguration>> {
    if config.nested_virtualization() {
        // SAFETY: This is a static Virtualization.framework capability query.
        let supported =
            unsafe { VZGenericPlatformConfiguration::isNestedVirtualizationSupported() };
        if !supported {
            return Err(Error::NestedVirtNotSupported);
        }
    }

    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    let platform =
        unsafe { VZGenericPlatformConfiguration::init(VZGenericPlatformConfiguration::alloc()) };
    let data = NSData::with_bytes(config.machine_identifier());
    // SAFETY: The config stores bytes previously produced by
    // VZGenericMachineIdentifier, unless the caller supplied invalid data; VZ
    // returns nil in that case and we map it to InvalidConfig.
    let machine_identifier = unsafe {
        VZGenericMachineIdentifier::initWithDataRepresentation(
            VZGenericMachineIdentifier::alloc(),
            &data,
        )
    }
    .ok_or_else(|| Error::InvalidConfig {
        reason: "machine identifier data is invalid".into(),
    })?;
    // SAFETY: The setter copies the identifier into this newly-created platform
    // configuration before VM construction.
    unsafe {
        platform.setMachineIdentifier(&machine_identifier);
    }

    if config.nested_virtualization() {
        // SAFETY: The setter mutates only this newly-created platform object.
        unsafe {
            platform.setNestedVirtualizationEnabled(true);
        }
    }

    Ok(platform)
}

fn serial_device(boot_log: &BootLog) -> Result<SerialDeviceParts> {
    let file = match boot_log {
        BootLog::File(path) => OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|source| Error::InvalidConfig {
                reason: format!("boot_log: {} not writable: {source}", path.display()),
            })?,
        BootLog::None => OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .map_err(|source| Error::InvalidConfig {
                reason: format!("boot_log: /dev/null not writable: {source}"),
            })?,
    };
    let fd = file.into_raw_fd();
    let file_handle =
        NSFileHandle::initWithFileDescriptor_closeOnDealloc(NSFileHandle::alloc(), fd, true);

    // SAFETY: The file handle owns a valid fd, and `alloc` is immediately
    // consumed by the designated serial attachment initializer.
    let attachment = unsafe {
        VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
            VZFileHandleSerialPortAttachment::alloc(),
            None,
            Some(&file_handle),
        )
    };
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    let serial = unsafe {
        VZVirtioConsoleDeviceSerialPortConfiguration::init(
            VZVirtioConsoleDeviceSerialPortConfiguration::alloc(),
        )
    };

    // SAFETY: The attachment object remains retained by the returned
    // VzConfiguration for at least as long as the serial device.
    unsafe {
        let attachment_parent: &VZSerialPortAttachment = (*attachment).as_super();
        serial.setAttachment(Some(attachment_parent));
    }

    Ok((Some(file_handle), Some(attachment), Some(serial)))
}

fn storage_devices(init_block: &Path, block_devices: &[BlockDevice]) -> Result<StorageDeviceParts> {
    let mut attachments = Vec::with_capacity(block_devices.len() + 1);
    let mut devices = Vec::with_capacity(block_devices.len() + 1);

    push_block_device(&mut attachments, &mut devices, init_block, true)?;
    for block_device in block_devices {
        push_block_device(
            &mut attachments,
            &mut devices,
            block_device.path(),
            block_device.read_only(),
        )?;
    }

    Ok((attachments, devices))
}

fn push_block_device(
    attachments: &mut Vec<Retained<VZDiskImageStorageDeviceAttachment>>,
    devices: &mut Vec<Retained<VZVirtioBlockDeviceConfiguration>>,
    path: &Path,
    read_only: bool,
) -> Result<()> {
    let url = ns_url_file(path)?;

    // SAFETY: The URL is a canonical local file URL, and `alloc` is consumed
    // by the designated disk-image attachment initializer.
    let attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &url,
            read_only,
        )
        .map_err(|error| Error::UnclassifiedVz {
            reason: format!(
                "disk attachment failed for {}: {}",
                path.display(),
                nserror_desc(&error)
            ),
        })?
    };

    // SAFETY: The attachment remains retained by VzConfiguration for at least
    // as long as the block-device configuration.
    let device = unsafe {
        let parent: &VZStorageDeviceAttachment = (*attachment).as_super();
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            parent,
        )
    };

    attachments.push(attachment);
    devices.push(device);
    Ok(())
}

fn network_devices(networks: &[Network], macs: &[String]) -> Result<NetworkDeviceParts> {
    let mut vmnet_allocations = Vec::new();
    let mut attachments = Vec::new();
    let mut retained_macs = Vec::new();
    let mut devices = Vec::new();
    let mut interfaces = Vec::new();

    for (index, network) in networks.iter().enumerate() {
        let device = network_device();
        let mac = mac_address(&macs[index])?;
        let device_parent: &VZNetworkDeviceConfiguration = (*device).as_super();
        // SAFETY: The retained MAC lives in `VzConfiguration` for the lifetime
        // of the VM, and the device copies the address.
        unsafe {
            device_parent.setMACAddress(&mac);
        }
        match network {
            Network::Nat => {
                let attachment = nat_attachment();
                let attachment_parent: &VZNetworkDeviceAttachment = (*attachment).as_super();
                let device_parent: &VZNetworkDeviceConfiguration = (*device).as_super();
                // SAFETY: Both Objective-C objects are retained in
                // `VzConfiguration` for the lifetime of the VM.
                unsafe {
                    device_parent.setAttachment(Some(attachment_parent));
                }
                attachments.push(NetworkAttachment::Nat(attachment));
            }
            Network::VmnetShared { subnet } => {
                let setup = vmnet_setup(subnet.as_deref(), index)?;
                let attachment_parent: &VZNetworkDeviceAttachment = (*setup.attachment).as_super();
                let device_parent: &VZNetworkDeviceConfiguration = (*device).as_super();
                // SAFETY: The vmnet attachment lives in `VzConfiguration`.
                unsafe {
                    device_parent.setAttachment(Some(attachment_parent));
                }
                vmnet_allocations.push(VmnetAllocation {
                    _configuration: setup.configuration,
                    _network: setup.network,
                });
                attachments.push(NetworkAttachment::Vmnet(setup.attachment));
                interfaces.push(setup.interface);
            }
        }
        retained_macs.push(mac);
        devices.push(device);
    }

    Ok((
        vmnet_allocations,
        attachments,
        retained_macs,
        devices,
        interfaces,
    ))
}

fn network_device() -> Retained<VZVirtioNetworkDeviceConfiguration> {
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    unsafe { VZVirtioNetworkDeviceConfiguration::init(VZVirtioNetworkDeviceConfiguration::alloc()) }
}

fn nat_attachment() -> Retained<VZNATNetworkDeviceAttachment> {
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    unsafe { VZNATNetworkDeviceAttachment::init(VZNATNetworkDeviceAttachment::alloc()) }
}

fn mac_address(value: &str) -> Result<Retained<VZMACAddress>> {
    let value = NSString::from_str(value);
    // SAFETY: The NSString is valid for the duration of the initializer call.
    unsafe { VZMACAddress::initWithString(VZMACAddress::alloc(), &value) }.ok_or_else(|| {
        Error::InvalidConfig {
            reason: "network MAC address data is invalid".into(),
        }
    })
}

#[derive(Debug)]
struct VmnetSetup {
    configuration: VmnetNetworkConfigurationRef,
    network: VmnetNetworkRef,
    attachment: Retained<VZVmnetNetworkDeviceAttachment>,
    interface: NetworkInterface,
}

fn vmnet_setup(subnet: Option<&str>, index: usize) -> Result<VmnetSetup> {
    let mut status = 0_u32;
    // SAFETY: vmnet returns an owned opaque configuration reference or null
    // with a status code.
    let configuration =
        unsafe { vmnet_network_configuration_create(VMNET_SHARED_MODE, &raw mut status) };
    if configuration.is_null() {
        return Err(Error::UnclassifiedVz {
            reason: format!(
                "vmnet_network_configuration_create failed: status={} ({})",
                status,
                vmnet_return_str(status)
            ),
        });
    }

    // SAFETY: The configuration reference is valid and owned by this setup.
    unsafe {
        vmnet_network_configuration_disable_dhcp(configuration);
    }

    if let Some(subnet) = subnet {
        let (gateway, mask) = explicit_vmnet_subnet(subnet)?;
        // SAFETY: vmnet reads the two in_addr values during this call only.
        let status = unsafe {
            vmnet_network_configuration_set_ipv4_subnet(
                configuration,
                &raw const gateway,
                &raw const mask,
            )
        };
        if status != VMNET_SUCCESS {
            return Err(Error::UnclassifiedVz {
                reason: format!(
                    "vmnet_network_configuration_set_ipv4_subnet failed for {subnet}: status={} ({})",
                    status,
                    vmnet_return_str(status)
                ),
            });
        }
    }

    let mut status = 0_u32;
    // SAFETY: vmnet returns an owned opaque network reference or null with a
    // status code. The reference must outlive the VZ attachment.
    let network = unsafe { vmnet_network_create(configuration, &raw mut status) };
    if network.is_null() {
        return Err(Error::UnclassifiedVz {
            reason: format!(
                "vmnet_network_create failed: status={} ({})",
                status,
                vmnet_return_str(status)
            ),
        });
    }

    let mut subnet_addr = InAddr::default();
    let mut mask_addr = InAddr::default();
    // SAFETY: The network reference is valid and vmnet writes both in_addr
    // outputs synchronously.
    unsafe {
        vmnet_network_get_ipv4_subnet(network, &raw mut subnet_addr, &raw mut mask_addr);
    }
    let (lower, _mask, prefix) = subnet_host_order(subnet_addr, mask_addr);
    let gateway = lower | 1;
    let guest = lower | 2;

    // SAFETY: objc2 does not expose this unavailable initializer, but S6/S9
    // verified that passing `vmnet_network_ref` with the exact RefEncode shape
    // is the correct VZ API boundary.
    let attachment: Option<Retained<VZVmnetNetworkDeviceAttachment>> = unsafe {
        msg_send![
            VZVmnetNetworkDeviceAttachment::alloc(),
            initWithNetwork: network,
        ]
    };
    let attachment = attachment.ok_or_else(|| Error::UnclassifiedVz {
        reason: "-[VZVmnetNetworkDeviceAttachment initWithNetwork:] returned nil".into(),
    })?;

    let interface = NetworkInterface::new(
        format!("eth{index}"),
        ipv4_from_u32(guest),
        prefix,
        ipv4_from_u32(gateway),
    );

    Ok(VmnetSetup {
        configuration,
        network,
        attachment,
        interface,
    })
}

fn explicit_vmnet_subnet(subnet: &str) -> Result<(InAddr, InAddr)> {
    let Some((address, prefix)) = subnet.split_once('/') else {
        return invalid_config(format!("vmnet subnet `{subnet}` is not CIDR notation"));
    };
    let address = address
        .parse::<Ipv4Addr>()
        .map_err(|_| Error::InvalidConfig {
            reason: format!("vmnet subnet `{subnet}` has an invalid IPv4 address"),
        })?;
    let prefix = prefix.parse::<u8>().map_err(|_| Error::InvalidConfig {
        reason: format!("vmnet subnet `{subnet}` has an invalid prefix"),
    })?;
    if prefix > 30 {
        return invalid_config(format!(
            "vmnet subnet `{subnet}` must leave room for gateway and guest addresses"
        ));
    }
    let mask = prefix_mask(prefix);
    let lower = u32::from(address) & mask;
    Ok((in_addr(lower | 1), in_addr(mask)))
}

fn subnet_host_order(subnet: InAddr, mask: InAddr) -> (u32, u32, u8) {
    let subnet = u32::from_be(subnet.s_addr);
    let mask = u32::from_be(mask.s_addr);
    let prefix = u8::try_from(mask.count_ones()).expect("IPv4 prefix has at most 32 bits");
    (subnet & mask, mask, prefix)
}

fn in_addr(value: u32) -> InAddr {
    InAddr {
        s_addr: value.to_be(),
    }
}

fn prefix_mask(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn ipv4_from_u32(value: u32) -> Ipv4Addr {
    Ipv4Addr::from(value)
}

fn vmnet_return_str(code: u32) -> &'static str {
    match code {
        1000 => "VMNET_SUCCESS",
        1001 => "VMNET_FAILURE",
        1002 => "VMNET_MEM_FAILURE",
        1003 => "VMNET_INVALID_ARGUMENT",
        1004 => "VMNET_SETUP_INCOMPLETE",
        1005 => "VMNET_INVALID_ACCESS",
        1006 => "VMNET_PACKET_TOO_BIG",
        1007 => "VMNET_BUFFER_EXHAUSTED",
        1008 => "VMNET_TOO_MANY_PACKETS",
        1009 => "VMNET_SHARING_SERVICE_BUSY",
        1010 => "VMNET_NOT_AUTHORIZED",
        _ => "VMNET_UNKNOWN",
    }
}

fn rosetta_device(enabled: bool) -> Result<RosettaDeviceParts> {
    if !enabled {
        return Ok((None, None));
    }

    let availability = unsafe { VZLinuxRosettaDirectoryShare::availability() };
    match availability {
        VZLinuxRosettaAvailability::NotSupported => {
            return invalid_config("rosetta was requested but is not supported on this host");
        }
        VZLinuxRosettaAvailability::NotInstalled => {
            return invalid_config("rosetta was requested but is not installed on this host");
        }
        VZLinuxRosettaAvailability::Installed => {}
        other => {
            return invalid_config(format!(
                "unknown rosetta availability encountered: {:?}",
                other.0
            ));
        }
    }

    // SAFETY: Rosetta availability is installed, and the returned directory
    // share is retained by VzConfiguration for at least as long as the device.
    let share = unsafe {
        VZLinuxRosettaDirectoryShare::initWithError(VZLinuxRosettaDirectoryShare::alloc()).map_err(
            |error| Error::UnclassifiedVz {
                reason: format!("rosetta directory share failed: {}", nserror_desc(&error)),
            },
        )?
    };
    let tag = NSString::from_str("rosetta");
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    let device = unsafe {
        VZVirtioFileSystemDeviceConfiguration::initWithTag(
            VZVirtioFileSystemDeviceConfiguration::alloc(),
            &tag,
        )
    };
    // SAFETY: The share remains retained by VzConfiguration for at least as
    // long as the filesystem device configuration.
    unsafe {
        let share_parent: &VZDirectoryShare = (*share).as_super();
        device.setShare(Some(share_parent));
    }

    Ok((Some(share), Some(device)))
}

fn socket_device() -> Retained<VZVirtioSocketDeviceConfiguration> {
    // SAFETY: `alloc` is immediately consumed by the designated initializer.
    unsafe { VZVirtioSocketDeviceConfiguration::init(VZVirtioSocketDeviceConfiguration::alloc()) }
}

fn ns_url_file(path: &Path) -> Result<Retained<NSURL>> {
    let path = path.canonicalize().map_err(|source| Error::InvalidConfig {
        reason: format!("{} not accessible: {source}", path.display()),
    })?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(
        &path.to_string_lossy(),
    )))
}

#[cfg(feature = "snapshot")]
fn ns_url_file_for_create(path: &Path) -> Result<Retained<NSURL>> {
    let parent = path.parent().ok_or_else(|| Error::InvalidConfig {
        reason: format!("snapshot path {} has no parent", path.display()),
    })?;
    let parent = parent
        .canonicalize()
        .map_err(|error| Error::InvalidConfig {
            reason: format!(
                "snapshot parent {} is not accessible: {error}",
                parent.display()
            ),
        })?;
    let file_name = path.file_name().ok_or_else(|| Error::InvalidConfig {
        reason: format!("snapshot path {} has no file name", path.display()),
    })?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(
        &parent.join(file_name).to_string_lossy(),
    )))
}

fn kernel_command_line(extra: &[String]) -> String {
    if extra.is_empty() {
        BASE_KERNEL_COMMAND_LINE.to_owned()
    } else {
        format!("{} {}", BASE_KERNEL_COMMAND_LINE, extra.join(" "))
    }
}

fn nserror_desc(error: &NSError) -> String {
    format!("NSError code={} desc={error}", error.code())
}

fn completion_result(operation: VmOperation, error: *mut NSError) -> Result<()> {
    if error.is_null() {
        Ok(())
    } else {
        // SAFETY: VZ passes a non-null NSError pointer that is valid for the
        // duration of the completion callback. We copy it to String before
        // returning from the block.
        let reason = unsafe { nserror_desc(&*error) };
        Err(operation.error(reason))
    }
}

fn dial_completion_result(
    port: VsockPort,
    connection: *mut VZVirtioSocketConnection,
    error: *mut NSError,
) -> Result<OwnedFd> {
    if !error.is_null() {
        // SAFETY: VZ passes a non-null NSError pointer that is valid for the
        // duration of the completion callback. We copy it to String before
        // returning from the block.
        let reason = unsafe { nserror_desc(&*error) };
        return Err(Error::Dial { port, reason });
    }
    if connection.is_null() {
        return Err(Error::Dial {
            port,
            reason: "VZ returned no connection and no error".into(),
        });
    }

    // SAFETY: VZ passes a non-null connection pointer that is valid for the
    // duration of the completion callback. We duplicate the fd immediately
    // because VZ owns and closes the original with the connection object.
    let fd = unsafe { (*connection).fileDescriptor() };
    if fd < 0 {
        return Err(Error::Dial {
            port,
            reason: "VZ returned a closed connection fd".into(),
        });
    }
    // SAFETY: `dup` does not take ownership of `fd`; it returns a new fd on
    // success which becomes owned below.
    let duplicated = unsafe { libc::dup(fd) };
    if duplicated < 0 {
        return Err(Error::Dial {
            port,
            reason: format!("dup(vsock fd) failed: {}", std::io::Error::last_os_error()),
        });
    }
    // SAFETY: `duplicated` is a fresh fd returned by `dup`, so this transfers
    // ownership into OwnedFd exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicated) })
}

fn vm_phase(state: VZVirtualMachineState) -> VmPhase {
    if state == VZVirtualMachineState::Paused {
        VmPhase::Paused
    } else if state == VZVirtualMachineState::Stopping
        || state == VZVirtualMachineState::Stopped
        || state == VZVirtualMachineState::Error
    {
        VmPhase::Stopping
    } else {
        VmPhase::Running
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, KernelImage, VmConfig};

    #[test]
    fn vz_configuration_requires_resolved_kernel_file() {
        let init_block = tempfile::NamedTempFile::new().expect("init block");
        let config = VmConfig::builder()
            .init_block(init_block.path())
            .build()
            .expect("config");

        assert_eq!(
            super::build_configuration(&config).expect_err("kernel must be resolved"),
            Error::InvalidConfig {
                reason: "VZ boot requires a resolved kernel file".into(),
            }
        );
    }

    #[test]
    fn vz_configuration_requires_resolved_init_block() {
        let kernel = tempfile::NamedTempFile::new().expect("kernel");
        let config = VmConfig::builder()
            .kernel(KernelImage::from_file(kernel.path()))
            .build()
            .expect("config");

        assert_eq!(
            super::build_configuration(&config).expect_err("init block must be resolved"),
            Error::InvalidConfig {
                reason: "VZ boot requires a resolved init block".into(),
            }
        );
    }
}
