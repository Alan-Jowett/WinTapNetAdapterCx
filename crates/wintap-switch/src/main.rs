#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod windows_runtime {
    use std::ffi::OsStr;
    use std::mem::MaybeUninit;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};

    use wintap_switch_core::{
        select_io_ring_version, BufferPool, EndpointId, IoRingCapabilities, IoRingVersion, Switch,
        FRAME_MAXIMUM,
    };

    type Handle = *mut core::ffi::c_void;
    type HResult = i32;
    type Dword = u32;
    type Ulonglong = u64;

    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const FILE_FLAG_OVERLAPPED: Dword = 0x4000_0000;
    const FILE_GENERIC_READ: Dword = 0x8000_0000;
    const FILE_GENERIC_WRITE: Dword = 0x4000_0000;
    const OPEN_EXISTING: Dword = 3;
    const S_OK: HResult = 0;
    const S_FALSE: HResult = 1;
    const IORING_OP_READ: Dword = 1;
    const IORING_OP_WRITE: Dword = 5;
    const IORING_SQE_FLAG_NONE: Dword = 0;
    const IORING_VERSION_3: Dword = 300;
    const IORING_REF_RAW: Dword = 0;
    const FILE_WRITE_FLAG_NONE: Dword = 0;
    const CTRL_C_EVENT: Dword = 0;
    const CTRL_CLOSE_EVENT: Dword = 2;
    const CANCEL_COMPLETION_MARKER: Ulonglong = 1_u64 << 62;

    #[repr(C)]
    struct RawIoRingCapabilities {
        max_version: Dword,
        _reserved: [Dword; 15],
    }

    #[repr(C)]
    struct IoRingCreateFlags {
        required: Dword,
        advisory: Dword,
    }

    #[repr(C)]
    struct IoRingHandleRef {
        kind: Dword,
        handle: Handle,
    }

    #[repr(C)]
    struct IoRingBufferInfo {
        address: *mut u8,
        length: Dword,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RegisteredBuffer {
        index: Dword,
        offset: Dword,
    }

    #[repr(C)]
    union IoRingBufferRefValue {
        address: *mut u8,
        registered: RegisteredBuffer,
    }

    #[repr(C)]
    struct IoRingBufferRef {
        kind: Dword,
        value: IoRingBufferRefValue,
    }

    #[repr(C)]
    struct IoRingCompletion {
        user_data: Ulonglong,
        result_code: HResult,
        information: Ulonglong,
    }

    static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn console_handler(control_type: Dword) -> i32 {
        if control_type == CTRL_C_EVENT || control_type == CTRL_CLOSE_EVENT {
            STOP_REQUESTED.store(true, Ordering::SeqCst);
            1
        } else {
            0
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CloseHandle(handle: Handle) -> i32;
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(Dword) -> i32>,
            add: i32,
        ) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: Dword,
            share: Dword,
            security: *mut core::ffi::c_void,
            creation: Dword,
            flags: Dword,
            template: Handle,
        ) -> Handle;
        fn CreateIoRing(
            version: Dword,
            flags: IoRingCreateFlags,
            submission_queue_size: Dword,
            completion_queue_size: Dword,
            ring: *mut Handle,
        ) -> HResult;
        fn CloseIoRing(ring: Handle) -> HResult;
        fn QueryIoRingCapabilities(capabilities: *mut RawIoRingCapabilities) -> HResult;
        fn IsIoRingOpSupported(ring: Handle, operation: Dword) -> i32;
        fn BuildIoRingRegisterFileHandles(
            ring: Handle,
            count: Dword,
            files: *const Handle,
            user_data: Ulonglong,
        ) -> HResult;
        fn BuildIoRingRegisterBuffers(
            ring: Handle,
            count: Dword,
            buffers: *const IoRingBufferInfo,
            user_data: Ulonglong,
        ) -> HResult;
        fn BuildIoRingCancelRequest(
            ring: Handle,
            file: IoRingHandleRef,
            operation: Ulonglong,
            user_data: Ulonglong,
        ) -> HResult;
        fn BuildIoRingReadFile(
            ring: Handle,
            file: IoRingHandleRef,
            buffer: IoRingBufferRef,
            bytes: Dword,
            offset: Ulonglong,
            user_data: Ulonglong,
            flags: Dword,
        ) -> HResult;
        fn BuildIoRingWriteFile(
            ring: Handle,
            file: IoRingHandleRef,
            buffer: IoRingBufferRef,
            bytes: Dword,
            offset: Ulonglong,
            write_flags: Dword,
            user_data: Ulonglong,
            flags: Dword,
        ) -> HResult;
        fn SubmitIoRing(
            ring: Handle,
            wait_operations: Dword,
            milliseconds: Dword,
            submitted: *mut Dword,
        ) -> HResult;
        fn PopIoRingCompletion(ring: Handle, completion: *mut IoRingCompletion) -> HResult;
    }

    struct Endpoint {
        id: EndpointId,
        handle: Handle,
    }

    impl Drop for Endpoint {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    struct Runtime {
        ring: Handle,
        endpoints: [Endpoint; 2],
        buffers: Vec<Box<[u8; FRAME_MAXIMUM]>>,
        _registered_files: [Handle; 2],
        _registered_buffers: [IoRingBufferInfo; 2],
        pool: BufferPool,
        active: [Option<(wintap_switch_core::SlotCompletion, Handle, Ulonglong)>; 2],
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                CloseIoRing(self.ring);
            }
        }
    }

    impl Runtime {
        fn start() -> Result<Self, String> {
            let maximum_version = query_capabilities()?;

            let endpoints = [
                Endpoint {
                    id: EndpointId::new(1),
                    handle: open_endpoint(r"\\.\WinTapRust")?,
                },
                Endpoint {
                    id: EndpointId::new(2),
                    handle: open_endpoint(r"\\.\WinTapRust2")?,
                },
            ];
            let flags = IoRingCreateFlags {
                required: 0,
                advisory: 0,
            };
            let mut ring = null_mut();
            check_hr(
                unsafe { CreateIoRing(IORING_VERSION_3, flags, 8, 8, &mut ring) },
                "CreateIoRing",
            )?;
            let capabilities = IoRingCapabilities {
                maximum_version,
                supports_read: unsafe { IsIoRingOpSupported(ring, IORING_OP_READ) != 0 },
                supports_write: unsafe { IsIoRingOpSupported(ring, IORING_OP_WRITE) != 0 },
                supports_read_scatter: false,
                supports_write_gather: false,
            };
            let version = match select_io_ring_version(capabilities) {
                Ok(version) => version,
                Err(error) => {
                    unsafe {
                        CloseIoRing(ring);
                    }
                    return Err(format!(
                        "required I/O-ring capability unavailable: {error:?}"
                    ));
                }
            };
            if version != IoRingVersion::V3 {
                unsafe {
                    CloseIoRing(ring);
                }
                return Err("v4 scatter/gather requires dedicated validation".to_string());
            }

            let mut buffers = Vec::new();
            buffers
                .try_reserve_exact(2)
                .map_err(|_| "buffer pool allocation failed".to_string())?;
            buffers.push(Box::new([0; FRAME_MAXIMUM]));
            buffers.push(Box::new([0; FRAME_MAXIMUM]));
            let registrations = [
                IoRingBufferInfo {
                    address: buffers[0].as_ptr() as *mut u8,
                    length: FRAME_MAXIMUM as Dword,
                },
                IoRingBufferInfo {
                    address: buffers[1].as_ptr() as *mut u8,
                    length: FRAME_MAXIMUM as Dword,
                },
            ];
            let files = [endpoints[0].handle, endpoints[1].handle];
            check_hr(
                unsafe {
                    BuildIoRingRegisterFileHandles(ring, files.len() as Dword, files.as_ptr(), 0)
                },
                "BuildIoRingRegisterFileHandles",
            )?;
            check_hr(
                unsafe {
                    BuildIoRingRegisterBuffers(
                        ring,
                        registrations.len() as Dword,
                        registrations.as_ptr(),
                        0,
                    )
                },
                "BuildIoRingRegisterBuffers",
            )?;
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(ring, 2, 0, &mut submitted) },
                "SubmitIoRing",
            )?;
            for _ in 0..2 {
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                check_hr(
                    unsafe { PopIoRingCompletion(ring, completion.as_mut_ptr()) },
                    "PopIoRingCompletion",
                )?;
            }

            let mut runtime = Self {
                ring,
                endpoints,
                buffers,
                _registered_files: files,
                _registered_buffers: registrations,
                pool: BufferPool::new(2),
                active: [None, None],
            };
            runtime.post_read(0)?;
            runtime.post_read(1)?;
            runtime.submit()?;
            Ok(runtime)
        }

        fn post_read(&mut self, slot: usize) -> Result<(), String> {
            let completion = self
                .pool
                .begin_read(slot)
                .map_err(|error| format!("begin read: {error:?}"))?;
            let endpoint = &self.endpoints[slot];
            check_hr(
                unsafe {
                    BuildIoRingReadFile(
                        self.ring,
                        handle_ref(endpoint.handle),
                        buffer_ref(slot as Dword),
                        FRAME_MAXIMUM as Dword,
                        0,
                        encode_completion(slot, completion.generation, false),
                        IORING_SQE_FLAG_NONE,
                    )
                },
                "BuildIoRingReadFile",
            )?;
            self.active[slot] = Some((
                completion,
                endpoint.handle,
                encode_completion(slot, completion.generation, false),
            ));
            Ok(())
        }

        fn submit(&self) -> Result<(), String> {
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(self.ring, 0, 0, &mut submitted) },
                "SubmitIoRing",
            )
        }

        fn run(&mut self) -> Result<(), String> {
            let mut switch = Switch::static_pair();
            loop {
                if STOP_REQUESTED.load(Ordering::SeqCst) {
                    return self.shutdown();
                }
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                if status == S_FALSE {
                    self.submit()?;
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                let completion = unsafe { completion.assume_init() };
                let (slot, generation, is_write) = decode_completion(completion.user_data);
                let slot_completion = wintap_switch_core::SlotCompletion { slot, generation };
                self.active[slot] = None;
                if is_write {
                    self.pool
                        .complete_write(slot_completion)
                        .map_err(|error| format!("write completion: {error:?}"))?;
                    self.post_read(slot)?;
                } else {
                    self.pool
                        .begin_dispatch(slot_completion)
                        .map_err(|error| format!("read completion: {error:?}"))?;
                    let source = self.endpoints[slot].id;
                    let length = completion.information as usize;
                    if length <= FRAME_MAXIMUM {
                        let recipients = switch
                            .forward(source, &self.buffers[slot][..length])
                            .map_err(|error| format!("forwarding failure: {error:?}"))?;
                        if let Some(destination) = recipients.first() {
                            let peer = if *destination == self.endpoints[0].id {
                                self.endpoints[0].handle
                            } else {
                                self.endpoints[1].handle
                            };
                            self.pool
                                .begin_writes(slot_completion, 1)
                                .map_err(|error| format!("begin write: {error:?}"))?;
                            check_hr(
                                unsafe {
                                    BuildIoRingWriteFile(
                                        self.ring,
                                        handle_ref(peer),
                                        buffer_ref(slot as Dword),
                                        length as Dword,
                                        0,
                                        FILE_WRITE_FLAG_NONE,
                                        encode_completion(slot, generation, true),
                                        IORING_SQE_FLAG_NONE,
                                    )
                                },
                                "BuildIoRingWriteFile",
                            )?;
                            self.active[slot] = Some((
                                slot_completion,
                                peer,
                                encode_completion(slot, generation, true),
                            ));
                        } else {
                            self.post_read(slot)?;
                        }
                    } else {
                        self.post_read(slot)?;
                    }
                }
                self.submit()?;
            }
        }

        fn shutdown(&mut self) -> Result<(), String> {
            for active in self.active.iter().flatten() {
                check_hr(
                    unsafe {
                        BuildIoRingCancelRequest(
                            self.ring,
                            handle_ref(active.1),
                            active.2,
                            CANCEL_COMPLETION_MARKER | active.0.slot as Ulonglong,
                        )
                    },
                    "BuildIoRingCancelRequest",
                )?;
            }
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(self.ring, 0, 0, &mut submitted) },
                "SubmitIoRing",
            )?;
            while self.active.iter().any(Option::is_some) {
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                if status == S_FALSE {
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                let completion = unsafe { completion.assume_init() };
                if completion.user_data & CANCEL_COMPLETION_MARKER != 0 {
                    continue;
                }
                let (slot, generation, _) = decode_completion(completion.user_data);
                let slot_completion = wintap_switch_core::SlotCompletion { slot, generation };
                self.pool
                    .cancel(slot_completion)
                    .map_err(|error| format!("cancel completion: {error:?}"))?;
                self.active[slot] = None;
            }
            Ok(())
        }
    }

    fn query_capabilities() -> Result<Dword, String> {
        let mut raw = RawIoRingCapabilities {
            max_version: 0,
            _reserved: [0; 15],
        };
        check_hr(
            unsafe { QueryIoRingCapabilities(&mut raw) },
            "QueryIoRingCapabilities",
        )?;
        Ok(raw.max_version)
    }

    fn open_endpoint(path: &str) -> Result<Handle, String> {
        let wide: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                0,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(format!("CreateFileW failed for {path}"))
        } else {
            Ok(handle)
        }
    }

    fn buffer_ref(index: Dword) -> IoRingBufferRef {
        IoRingBufferRef {
            kind: 1,
            value: IoRingBufferRefValue {
                registered: RegisteredBuffer { index, offset: 0 },
            },
        }
    }

    fn handle_ref(handle: Handle) -> IoRingHandleRef {
        IoRingHandleRef {
            kind: IORING_REF_RAW,
            handle,
        }
    }

    fn encode_completion(slot: usize, generation: u64, write: bool) -> Ulonglong {
        (slot as Ulonglong) | (generation << 8) | ((write as Ulonglong) << 63)
    }

    fn decode_completion(value: Ulonglong) -> (usize, u64, bool) {
        (
            (value & 0xff) as usize,
            (value >> 8) & 0x007f_ffff_ffff_ffff,
            value >> 63 != 0,
        )
    }

    fn check_hr(status: HResult, operation: &str) -> Result<(), String> {
        if status == S_OK {
            Ok(())
        } else {
            Err(format!("{operation} failed with HRESULT 0x{status:08X}"))
        }
    }

    pub fn run() -> Result<(), String> {
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err("SetConsoleCtrlHandler failed".to_string());
        }
        let result = Runtime::start()?.run();
        unsafe {
            SetConsoleCtrlHandler(Some(console_handler), 0);
        }
        result
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_runtime::run() {
        eprintln!("WinTap switch stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("wintap-switch requires Windows");
    std::process::exit(1);
}
