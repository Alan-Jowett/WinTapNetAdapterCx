#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod windows_runtime {
    use std::env;
    use std::ffi::OsStr;
    use std::mem::MaybeUninit;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};

    use wintap_switch_core::{
        BufferPool, EndpointId, FRAME_MAXIMUM, ForwardingError, IoRingCapabilities, IoRingVersion,
        Switch, select_io_ring_version,
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
    const WAIT_TIMEOUT: HResult = 0x8007_05B4u32 as HResult;
    const IORING_OP_READ: Dword = 1;
    const IORING_OP_WRITE: Dword = 5;
    const IORING_SQE_FLAG_NONE: Dword = 0;
    const IORING_VERSION_3: Dword = 300;
    const IORING_REF_RAW: Dword = 0;
    const FILE_WRITE_FLAG_NONE: Dword = 0;
    const CTRL_C_EVENT: Dword = 0;
    const CTRL_CLOSE_EVENT: Dword = 2;
    const CANCEL_COMPLETION_MARKER: Ulonglong = 1_u64 << 62;
    const COMPLETION_WAIT_MILLISECONDS: Dword = 100;
    const ENDPOINT_COUNT: usize = 2;
    const DEFAULT_READ_DEPTH: usize = 128;
    const SLOT_BITS: u32 = 31;
    const ENDPOINT_BIT: u32 = 31;
    const GENERATION_BITS: u32 = 30;
    const SLOT_MASK: Ulonglong = (1_u64 << SLOT_BITS) - 1;
    const GENERATION_MASK: Ulonglong = (1_u64 << GENERATION_BITS) - 1;

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

    struct IoRingGuard(Handle);

    impl Drop for IoRingGuard {
        fn drop(&mut self) {
            unsafe {
                CloseIoRing(self.0);
            }
        }
    }

    impl IoRingGuard {
        fn into_inner(self) -> Handle {
            let ring = self.0;
            std::mem::forget(self);
            ring
        }
    }

    struct Runtime {
        ring: Handle,
        endpoints: [Endpoint; 2],
        buffers: Vec<Vec<u8>>,
        _registered_files: [Handle; 2],
        _registered_buffers: Vec<IoRingBufferInfo>,
        pool: BufferPool,
        active: Vec<Option<(wintap_switch_core::SlotCompletion, Handle, Ulonglong)>>,
        reads_per_endpoint: usize,
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            unsafe {
                CloseIoRing(self.ring);
            }
        }
    }

    impl Runtime {
        fn start(total_depth: usize) -> Result<Self, String> {
            if total_depth == 0 || total_depth % ENDPOINT_COUNT != 0 {
                return Err("read depth must be a positive even value".to_string());
            }
            let reads_per_endpoint = total_depth / ENDPOINT_COUNT;
            let total_bytes = total_depth
                .checked_mul(FRAME_MAXIMUM)
                .ok_or_else(|| "read depth buffer-size calculation overflowed".to_string())?;
            let queue_size = Dword::try_from(total_depth)
                .map_err(|_| "read depth exceeds I/O-ring limits".to_string())?;
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
                unsafe { CreateIoRing(IORING_VERSION_3, flags, queue_size, queue_size, &mut ring) },
                "CreateIoRing",
            )?;
            let ring = IoRingGuard(ring);
            let capabilities = IoRingCapabilities {
                maximum_version,
                supports_read: unsafe { IsIoRingOpSupported(ring.0, IORING_OP_READ) != 0 },
                supports_write: unsafe { IsIoRingOpSupported(ring.0, IORING_OP_WRITE) != 0 },
                supports_read_scatter: false,
                supports_write_gather: false,
            };
            let version = match select_io_ring_version(capabilities) {
                Ok(version) => version,
                Err(error) => {
                    return Err(format!(
                        "required I/O-ring capability unavailable: {error:?}"
                    ));
                }
            };
            if version != IoRingVersion::V3 {
                return Err("v4 scatter/gather requires dedicated validation".to_string());
            }

            let mut buffers = Vec::new();
            buffers
                .try_reserve_exact(total_depth)
                .map_err(|_| "buffer pool allocation failed".to_string())?;
            if total_bytes < FRAME_MAXIMUM {
                return Err("read depth buffer-size calculation was invalid".to_string());
            }
            for _ in 0..total_depth {
                let mut buffer = Vec::new();
                buffer
                    .try_reserve_exact(FRAME_MAXIMUM)
                    .map_err(|_| "buffer pool allocation failed".to_string())?;
                buffer.resize(FRAME_MAXIMUM, 0);
                buffers.push(buffer);
            }
            let registrations: Vec<_> = buffers
                .iter()
                .map(|buffer| IoRingBufferInfo {
                    address: buffer.as_ptr() as *mut u8,
                    length: FRAME_MAXIMUM as Dword,
                })
                .collect();
            let registration_count = Dword::try_from(registrations.len())
                .map_err(|_| "registered buffer count exceeds I/O-ring limits".to_string())?;
            let files = [endpoints[0].handle, endpoints[1].handle];
            check_hr(
                unsafe {
                    BuildIoRingRegisterFileHandles(
                        ring.0,
                        Dword::try_from(files.len()).expect("static endpoint count fits Dword"),
                        files.as_ptr(),
                        0,
                    )
                },
                "BuildIoRingRegisterFileHandles",
            )?;
            check_hr(
                unsafe {
                    BuildIoRingRegisterBuffers(
                        ring.0,
                        registration_count,
                        registrations.as_ptr(),
                        0,
                    )
                },
                "BuildIoRingRegisterBuffers",
            )?;
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(ring.0, 2, 0, &mut submitted) },
                "SubmitIoRing",
            )?;
            for _ in 0..2 {
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                check_hr(
                    unsafe { PopIoRingCompletion(ring.0, completion.as_mut_ptr()) },
                    "PopIoRingCompletion",
                )?;
                let completion = unsafe { completion.assume_init() };
                check_hr(completion.result_code, "I/O-ring registration")?;
            }

            let mut runtime = Self {
                ring: ring.into_inner(),
                endpoints,
                buffers,
                _registered_files: files,
                _registered_buffers: registrations,
                pool: BufferPool::new(total_depth),
                active: (0..total_depth).map(|_| None).collect(),
                reads_per_endpoint,
            };
            for slot in 0..total_depth {
                runtime.post_read(slot)?;
            }
            runtime.submit()?;
            Ok(runtime)
        }

        fn endpoint_for_slot(&self, slot: usize) -> &Endpoint {
            &self.endpoints[slot / self.reads_per_endpoint]
        }

        fn endpoint_index_for_slot(&self, slot: usize) -> usize {
            slot / self.reads_per_endpoint
        }

        fn post_read(&mut self, slot: usize) -> Result<(), String> {
            let completion = self
                .pool
                .begin_read(slot)
                .map_err(|error| format!("begin read: {error:?}"))?;
            let endpoint = self.endpoint_for_slot(slot);
            check_hr(
                unsafe {
                    BuildIoRingReadFile(
                        self.ring,
                        handle_ref(endpoint.handle),
                        buffer_ref(slot as Dword),
                        FRAME_MAXIMUM as Dword,
                        0,
                        encode_completion(
                            self.endpoint_index_for_slot(slot),
                            slot,
                            completion.generation,
                            false,
                        )?,
                        IORING_SQE_FLAG_NONE,
                    )
                },
                "BuildIoRingReadFile",
            )?;
            self.active[slot] = Some((
                completion,
                endpoint.handle,
                encode_completion(
                    self.endpoint_index_for_slot(slot),
                    slot,
                    completion.generation,
                    false,
                )?,
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

        fn wait_for_completion(&self) -> Result<(), String> {
            let mut submitted = 0;
            let status =
                unsafe { SubmitIoRing(self.ring, 1, COMPLETION_WAIT_MILLISECONDS, &mut submitted) };
            if status == WAIT_TIMEOUT {
                Ok(())
            } else {
                check_hr(status, "SubmitIoRing wait")
            }
        }

        fn validate_completion(
            &self,
            completion: &IoRingCompletion,
        ) -> Result<(usize, u64, bool), String> {
            let (endpoint_index, slot, generation, is_write) =
                decode_completion(completion.user_data)?;
            if slot >= self.active.len() {
                return Err(format!("completion references invalid slot {slot}"));
            }
            if endpoint_index != self.endpoint_index_for_slot(slot) {
                return Err(format!(
                    "completion references endpoint {endpoint_index} for slot {slot}"
                ));
            }
            let active = self.active[slot]
                .as_ref()
                .ok_or_else(|| format!("completion references inactive slot {slot}"))?;
            if active.0.slot != slot
                || active.0.generation != generation
                || active.2 != completion.user_data
            {
                return Err(format!("stale or unexpected completion for slot {slot}"));
            }
            check_hr(completion.result_code, "I/O-ring operation")?;
            Ok((slot, generation, is_write))
        }

        fn process_completion(
            &mut self,
            switch: &mut Switch,
            completion: IoRingCompletion,
        ) -> Result<(), String> {
            let (slot, generation, is_write) = self.validate_completion(&completion)?;
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
                let source = self.endpoint_for_slot(slot).id;
                let length = completion.information as usize;
                if length <= FRAME_MAXIMUM {
                    let recipients = match switch.forward(source, &self.buffers[slot][..length]) {
                        Ok(recipients) => recipients,
                        Err(ForwardingError::InvalidFrame(_)) => {
                            self.pool
                                .complete_dispatch(slot_completion)
                                .map_err(|error| format!("invalid frame: {error:?}"))?;
                            self.post_read(slot)?;
                            return Ok(());
                        }
                        Err(error) => {
                            return Err(format!("forwarding failure: {error:?}"));
                        }
                    };
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
                                    encode_completion(
                                        self.endpoint_index_for_slot(slot),
                                        slot,
                                        generation,
                                        true,
                                    )?,
                                    IORING_SQE_FLAG_NONE,
                                )
                            },
                            "BuildIoRingWriteFile",
                        )?;
                        self.active[slot] = Some((
                            slot_completion,
                            peer,
                            encode_completion(
                                self.endpoint_index_for_slot(slot),
                                slot,
                                generation,
                                true,
                            )?,
                        ));
                    } else {
                        self.pool
                            .complete_dispatch(slot_completion)
                            .map_err(|error| format!("drop completion: {error:?}"))?;
                        self.post_read(slot)?;
                    }
                } else {
                    self.pool
                        .complete_dispatch(slot_completion)
                        .map_err(|error| format!("invalid frame completion: {error:?}"))?;
                    self.post_read(slot)?;
                }
            }
            Ok(())
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
                    self.wait_for_completion()?;
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                self.process_completion(&mut switch, unsafe { completion.assume_init() })?;

                loop {
                    let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                    let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                    if status == S_FALSE {
                        break;
                    }
                    check_hr(status, "PopIoRingCompletion")?;
                    self.process_completion(&mut switch, unsafe { completion.assume_init() })?;
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
                            CANCEL_COMPLETION_MARKER | active.2,
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
                    self.wait_for_completion()?;
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                let completion = unsafe { completion.assume_init() };
                if completion.user_data & CANCEL_COMPLETION_MARKER != 0 {
                    let operation = completion.user_data & !CANCEL_COMPLETION_MARKER;
                    let (_, slot, _, _) = decode_completion(operation)?;
                    if slot >= self.active.len() {
                        return Err(format!("cancellation references invalid slot {slot}"));
                    }
                    let active = self.active[slot]
                        .take()
                        .ok_or_else(|| format!("cancellation references inactive slot {slot}"))?;
                    if active.2 != operation {
                        return Err(format!("stale or unexpected cancellation for slot {slot}"));
                    }
                    self.pool
                        .cancel(active.0)
                        .map_err(|error| format!("cancel completion: {error:?}"))?;
                    continue;
                }
                let (slot, generation, _) = self.validate_completion(&completion)?;
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

    fn encode_completion(
        endpoint_index: usize,
        slot: usize,
        generation: u64,
        write: bool,
    ) -> Result<Ulonglong, String> {
        let endpoint = Ulonglong::try_from(endpoint_index)
            .map_err(|_| "endpoint index does not fit completion identity".to_string())?;
        let slot = Ulonglong::try_from(slot)
            .map_err(|_| "slot does not fit completion identity".to_string())?;
        if endpoint > 1 {
            return Err("endpoint index does not fit completion identity".to_string());
        }
        if slot > SLOT_MASK {
            return Err("slot count exceeds completion identity capacity".to_string());
        }
        if generation == 0 || generation > GENERATION_MASK {
            return Err("slot generation exceeds completion identity capacity".to_string());
        }
        Ok(slot | (endpoint << ENDPOINT_BIT) | (generation << 32) | ((write as Ulonglong) << 63))
    }

    fn decode_completion(value: Ulonglong) -> Result<(usize, usize, u64, bool), String> {
        if value & CANCEL_COMPLETION_MARKER != 0 {
            return Err("completion identity contains cancellation marker".to_string());
        }
        let endpoint_index = ((value >> ENDPOINT_BIT) & 1) as usize;
        let slot = (value & SLOT_MASK) as usize;
        let generation = (value >> 32) & GENERATION_MASK;
        if generation == 0 {
            return Err("completion identity contains invalid generation".to_string());
        }
        Ok((endpoint_index, slot, generation, value >> 63 != 0))
    }

    fn check_hr(status: HResult, operation: &str) -> Result<(), String> {
        if status == S_OK {
            Ok(())
        } else {
            Err(format!("{operation} failed with HRESULT 0x{status:08X}"))
        }
    }

    fn parse_read_depth() -> Result<usize, String> {
        let mut args = env::args().skip(1);
        let mut read_depth = DEFAULT_READ_DEPTH;
        while let Some(argument) = args.next() {
            if argument == "--read-depth" {
                let value = args
                    .next()
                    .ok_or_else(|| "--read-depth requires a value".to_string())?;
                read_depth = value
                    .parse()
                    .map_err(|_| format!("invalid read depth '{value}'"))?;
            } else if argument == "--help" || argument == "-h" {
                println!("Usage: wintap-switch.exe [--read-depth <positive even total>]");
                println!("Default read depth: {DEFAULT_READ_DEPTH}");
                std::process::exit(0);
            } else {
                return Err(format!("unknown argument '{argument}'"));
            }
        }
        Ok(read_depth)
    }

    pub fn run() -> Result<(), String> {
        let read_depth = parse_read_depth()?;
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err("SetConsoleCtrlHandler failed".to_string());
        }
        let result = Runtime::start(read_depth)?.run();
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
