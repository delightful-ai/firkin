//! Spike <N> — <one-line description of the scary question>.
//!
//! This is the template boot skeleton lifted from s1-boot. It boots a Linux
//! VM to verify the harness works, then powers off. **Extend this** for your
//! spike's actual question. Look for `// TODO(spike):` markers.
//!
//! Read `docs/specs/rust_rewrite/PRO_TIPS.md` before touching threads,
//! `define_class!`, or codesigning. Read SPIKE_RUNBOOK.md for conventions.

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use dispatch2::dispatch_main;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, ClassType, Ivars};
use objc2_foundation::{NSArray, NSError, NSFileHandle, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_virtualization::{
    VZFileHandleSerialPortAttachment, VZGenericPlatformConfiguration, VZLinuxBootLoader,
    VZSerialPortAttachment, VZSerialPortConfiguration, VZVirtioConsoleDeviceSerialPortConfiguration,
    VZVirtualMachine, VZVirtualMachineConfiguration, VZVirtualMachineDelegate,
};

// ---------------------------------------------------------------------------
// Delegate: fires when the guest stops. Extend with more methods if your
// spike needs them (e.g. network disconnect).
// ---------------------------------------------------------------------------

define_class!(
    // SAFETY:
    // - NSObject superclass imposes no subclassing constraints.
    // - VZVirtualMachineDelegate does not require MainThreadOnly; callbacks
    //   fire on the VM's queue (main queue here).
    // - Ivars are Send + Sync, so the class is Send + Sync.
    // - No Drop impl.
    #[unsafe(super(NSObject))]
    struct StopDelegate {
        fired: AtomicBool,
    }

    unsafe impl NSObjectProtocol for StopDelegate {}

    unsafe impl VZVirtualMachineDelegate for StopDelegate {
        #[unsafe(method(guestDidStopVirtualMachine:))]
        fn guest_did_stop(&self, _vm: &VZVirtualMachine) {
            eprintln!("[delegate] guestDidStopVirtualMachine");
            self.finish(0);
        }

        #[unsafe(method(virtualMachine:didStopWithError:))]
        fn did_stop_with_error(&self, _vm: &VZVirtualMachine, error: &NSError) {
            eprintln!(
                "[delegate] virtualMachine:didStopWithError: {}",
                nserror_desc(error)
            );
            self.finish(6);
        }
    }
);

impl StopDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars::<Self> {
            fired: AtomicBool::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, code: i32) {
        if !self.fired().swap(true, Ordering::SeqCst) {
            eprintln!("[host] exiting with code {code}");
            std::process::exit(code);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ns_url_file(path: &str) -> Retained<NSURL> {
    let abs = std::fs::canonicalize(path)
        .unwrap_or_else(|e| panic!("canonicalize({path}): {e}"))
        .to_string_lossy()
        .into_owned();
    NSURL::fileURLWithPath(&NSString::from_str(&abs))
}

fn nserror_desc(err: &NSError) -> String {
    let code: isize = unsafe { msg_send![err, code] };
    let desc: Retained<NSString> = unsafe { msg_send![err, localizedDescription] };
    format!("NSError code={code} desc={}", desc.to_string())
}

fn asset_path(name: &str) -> String {
    let here = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    here.ancestors()
        .find(|a| a.join("assets").is_dir())
        .map(|a| a.join("assets").join(name).to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("assets/{name}"))
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let kernel = std::env::var("SPIKE_KERNEL").unwrap_or_else(|_| asset_path("vmlinux"));
    let initrd = std::env::var("SPIKE_INITRD").unwrap_or_else(|_| asset_path("initrd.cpio"));

    eprintln!("[host] kernel = {kernel}");
    eprintln!("[host] initrd = {initrd}");

    if !unsafe { VZVirtualMachine::isSupported() } {
        eprintln!("[host] VZVirtualMachine::isSupported() == false");
        std::process::exit(2);
    }

    // ---- VM config ---------------------------------------------------------
    let config: Retained<VZVirtualMachineConfiguration> =
        unsafe { VZVirtualMachineConfiguration::init(VZVirtualMachineConfiguration::alloc()) };

    let boot_loader: Retained<VZLinuxBootLoader> = unsafe {
        VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &ns_url_file(&kernel))
    };
    unsafe {
        boot_loader.setInitialRamdiskURL(Some(&ns_url_file(&initrd)));
        boot_loader.setCommandLine(&NSString::from_str("console=hvc0 panic=-1"));
        config.setBootLoader(Some(&boot_loader));
    }

    let platform: Retained<VZGenericPlatformConfiguration> =
        unsafe { VZGenericPlatformConfiguration::init(VZGenericPlatformConfiguration::alloc()) };
    unsafe { config.setPlatform(&platform) };
    unsafe {
        config.setCPUCount(1);
        config.setMemorySize(128 * 1024 * 1024);
    }

    // Serial port -> host stdout.
    let stdout_fh: Retained<NSFileHandle> = {
        let fd = std::io::stdout().as_raw_fd();
        NSFileHandle::initWithFileDescriptor(NSFileHandle::alloc(), fd)
    };
    let serial_attachment: Retained<VZFileHandleSerialPortAttachment> = unsafe {
        VZFileHandleSerialPortAttachment::initWithFileHandleForReading_fileHandleForWriting(
            VZFileHandleSerialPortAttachment::alloc(),
            None,
            Some(&stdout_fh),
        )
    };
    let serial: Retained<VZVirtioConsoleDeviceSerialPortConfiguration> = unsafe {
        VZVirtioConsoleDeviceSerialPortConfiguration::init(
            VZVirtioConsoleDeviceSerialPortConfiguration::alloc(),
        )
    };
    {
        let attach_parent: &VZSerialPortAttachment = (&*serial_attachment).as_super();
        unsafe { serial.setAttachment(Some(attach_parent)) };
    }
    let serial_parent: &VZSerialPortConfiguration = (&*serial).as_super();
    unsafe {
        config.setSerialPorts(&NSArray::<VZSerialPortConfiguration>::from_slice(&[serial_parent]));
    }

    // TODO(spike): attach the devices your spike needs. Examples:
    //
    //   // Block device (for S3/S4):
    //   let url = ns_url_file("assets/init.block");
    //   let attach = unsafe {
    //       VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_error(
    //           VZDiskImageStorageDeviceAttachment::alloc(), &url, false,
    //       )?
    //   };
    //   let dev = unsafe {
    //       VZVirtioBlockDeviceConfiguration::initWithAttachment(
    //           VZVirtioBlockDeviceConfiguration::alloc(), &attach.into_super())
    //   };
    //   unsafe {
    //       config.setStorageDevices(
    //           &NSArray::from_slice(&[(&*dev).as_super()])
    //       )
    //   };
    //
    //   // Vsock (for S2):
    //   let vsock: Retained<VZVirtioSocketDeviceConfiguration> =
    //       unsafe { VZVirtioSocketDeviceConfiguration::new() };
    //   unsafe {
    //       config.setSocketDevices(
    //           &NSArray::from_slice(&[(&*vsock).as_super()])
    //       )
    //   };

    if let Err(err) = unsafe { config.validateWithError() } {
        eprintln!("[host] configuration validation failed: {}", nserror_desc(&err));
        std::process::exit(3);
    }
    eprintln!("[host] configuration validated");

    // ---- VM + delegate -----------------------------------------------------
    let delegate = StopDelegate::new();
    let vm: Retained<VZVirtualMachine> = unsafe {
        VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &config)
    };
    let proto: &ProtocolObject<dyn VZVirtualMachineDelegate> = ProtocolObject::from_ref(&*delegate);
    unsafe { vm.setDelegate(Some(proto)) };

    // TODO(spike): if your spike needs to interact with the VM while it's
    // running (e.g. open vsock connections), grab the needed handles from
    // `vm` *here* — before startWithCompletionHandler — and stash them in
    // something like `Box::leak(Box::new(handles))`. VZ accessors are safe
    // to call on main thread before start.

    // ---- Start -------------------------------------------------------------
    let start_block = RcBlock::new(|err: *mut NSError| {
        if err.is_null() {
            eprintln!("[host] VM started");
            // TODO(spike): post-start work (e.g. spawn tokio runtime, issue
            // RPCs, etc.). Keep it non-blocking so the main queue can pump.
        } else {
            let nserr: &NSError = unsafe { &*err };
            eprintln!("[host] start failed: {}", nserror_desc(nserr));
            std::process::exit(4);
        }
    });
    unsafe { vm.startWithCompletionHandler(&start_block) };

    // Leak so the VM, delegate and block survive for the life of the process.
    // dispatch_main() diverges; OS reaps on exit.
    Box::leak(Box::new(vm));
    Box::leak(Box::new(delegate));
    std::mem::forget(start_block);

    eprintln!("[host] entering dispatch_main()");
    dispatch_main();
}
