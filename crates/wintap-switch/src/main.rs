#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod windows_runtime {
    use std::env;
    use std::ffi::OsStr;
    use std::mem::MaybeUninit;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use wintap_switch_core::{
        select_io_ring_version, BufferPool, EndpointId, ForwardingError, IoRingCapabilities,
        IoRingVersion, Switch, FRAME_MAXIMUM,
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
    const IORING_E_SUBMISSION_QUEUE_FULL: HResult = 0x8046_0002u32 as HResult;
    const HRESULT_FROM_NT_STATUS_DEVICE_BUSY: HResult = 0x9000_0011u32 as HResult;
    const HRESULT_FROM_WIN32_INVALID_USER_BUFFER: HResult = 0x8007_06F8u32 as HResult;
    const IORING_OP_READ: Dword = 1;
    const IORING_OP_WRITE: Dword = 5;
    const IORING_SQE_FLAG_NONE: Dword = 0;
    const IORING_VERSION_3: Dword = 300;
    const IORING_REF_RAW: Dword = 0;
    const FILE_WRITE_FLAG_NONE: Dword = 0;
    const CTRL_C_EVENT: Dword = 0;
    const CTRL_CLOSE_EVENT: Dword = 2;
    const CANCEL_COMPLETION_MARKER: Ulonglong = 1_u64 << 63;
    const COMPLETION_WAIT_MILLISECONDS: Dword = 100;
    const BUSY_RETRY_INITIAL_DELAY: Duration = Duration::from_millis(1);
    const BUSY_RETRY_MAX_DELAY: Duration = Duration::from_millis(64);
    const ENDPOINT_COUNT: usize = 2;
    const DEFAULT_READ_DEPTH: usize = 128;
    const STATS_REPORT_INTERVAL: Duration = Duration::from_secs(5);
    const SLOT_BITS: u32 = 31;
    const GENERATION_SHIFT: u32 = SLOT_BITS;
    const GENERATION_BITS: u32 = 32;
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
        active: Vec<Option<ActiveOperation>>,
        cancellations: Vec<Option<Ulonglong>>,
        operations_may_be_in_flight: bool,
        submission_queue_size: usize,
        reads_per_endpoint: usize,
        stats: RuntimeStats,
    }

    #[derive(Clone, Copy)]
    struct ActiveOperation {
        completion: wintap_switch_core::SlotCompletion,
        handle: Handle,
        user_data: Ulonglong,
        is_write: bool,
        length: Dword,
        queued: bool,
        submitted: bool,
        busy_retries: u32,
        drain_before_cancellation: bool,
    }

    struct RuntimeStats {
        enabled: bool,
        started: Instant,
        last_report: Instant,
        wait_calls: u64,
        signaled_wakes: u64,
        batches: u64,
        completions: u64,
        reads: u64,
        writes: u64,
        max_batch: u64,
    }

    impl RuntimeStats {
        fn new(enabled: bool) -> Self {
            let now = Instant::now();
            Self {
                enabled,
                started: now,
                last_report: now,
                wait_calls: 0,
                signaled_wakes: 0,
                batches: 0,
                completions: 0,
                reads: 0,
                writes: 0,
                max_batch: 0,
            }
        }

        fn record_wait(&mut self, signaled: bool) {
            self.wait_calls += 1;
            if signaled {
                self.signaled_wakes += 1;
            }
        }

        fn record_batch(&mut self, reads: u64, writes: u64) {
            let completions = reads + writes;
            self.batches += 1;
            self.completions += completions;
            self.reads += reads;
            self.writes += writes;
            self.max_batch = self.max_batch.max(completions);
            self.report(false);
        }

        fn report(&mut self, force: bool) {
            if !self.enabled || (!force && self.last_report.elapsed() < STATS_REPORT_INTERVAL) {
                return;
            }
            let elapsed = self.started.elapsed().as_secs_f64();
            let average_batch = if self.batches == 0 {
                0.0
            } else {
                self.completions as f64 / self.batches as f64
            };
            let average_reads_per_wake = if self.signaled_wakes == 0 {
                0.0
            } else {
                self.reads as f64 / self.signaled_wakes as f64
            };
            eprintln!(
                "io-ring stats: elapsed={elapsed:.1}s waits={} signaled_wakes={} batches={} completions={} reads={} writes={} avg_completions_per_batch={average_batch:.2} avg_reads_per_wake={average_reads_per_wake:.2} max_batch={}",
                self.wait_calls,
                self.signaled_wakes,
                self.batches,
                self.completions,
                self.reads,
                self.writes,
                self.max_batch,
            );
            self.last_report = Instant::now();
        }
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            if self.operations_may_be_in_flight && self.active.iter().any(Option::is_some) {
                std::process::abort();
            }
            unsafe {
                CloseIoRing(self.ring);
            }
        }
    }

    impl Runtime {
        fn start(total_depth: usize, stats_enabled: bool) -> Result<Self, String> {
            if total_depth == 0 || total_depth % ENDPOINT_COUNT != 0 {
                return Err("read depth must be a positive even value".to_string());
            }
            if total_depth > SLOT_MASK as usize + 1 {
                return Err("read depth exceeds completion identity capacity".to_string());
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
            let mut registrations = Vec::new();
            registrations
                .try_reserve_exact(buffers.len())
                .map_err(|_| "buffer registration allocation failed".to_string())?;
            for buffer in &buffers {
                registrations.push(IoRingBufferInfo {
                    address: buffer.as_ptr() as *mut u8,
                    length: FRAME_MAXIMUM as Dword,
                });
            }
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

            let pool = BufferPool::try_new(total_depth)
                .map_err(|error| format!("buffer pool allocation failed: {error:?}"))?;
            let mut active = Vec::new();
            active
                .try_reserve_exact(total_depth)
                .map_err(|_| "active operation allocation failed".to_string())?;
            active.resize(total_depth, None);
            let mut cancellations = Vec::new();
            cancellations
                .try_reserve_exact(total_depth)
                .map_err(|_| "cancellation tracking allocation failed".to_string())?;
            cancellations.resize(total_depth, None);
            let ring = ring.into_inner();
            let mut runtime = Self {
                ring,
                endpoints,
                buffers,
                _registered_files: files,
                _registered_buffers: registrations,
                pool,
                active,
                cancellations,
                operations_may_be_in_flight: false,
                submission_queue_size: total_depth,
                reads_per_endpoint,
                stats: RuntimeStats::new(stats_enabled),
            };
            for slot in 0..total_depth {
                runtime.post_read(slot)?;
            }
            if let Err(error) = runtime.submit_pending_operations() {
                return match runtime.shutdown() {
                    Ok(()) => Err(error),
                    Err(shutdown_error) => {
                        Err(format!("{error}; shutdown failed: {shutdown_error}"))
                    }
                };
            }
            Ok(runtime)
        }

        fn endpoint_for_slot(&self, slot: usize) -> &Endpoint {
            &self.endpoints[slot / self.reads_per_endpoint]
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
                        encode_completion(slot, completion.generation)?,
                        IORING_SQE_FLAG_NONE,
                    )
                },
                "BuildIoRingReadFile",
            )?;
            self.active[slot] = Some(ActiveOperation {
                completion,
                handle: endpoint.handle,
                user_data: encode_completion(slot, completion.generation)?,
                is_write: false,
                length: FRAME_MAXIMUM as Dword,
                queued: true,
                submitted: false,
                busy_retries: 0,
                drain_before_cancellation: false,
            });
            Ok(())
        }

        fn submit_pending_operations(&mut self) -> Result<(), String> {
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(self.ring, 0, 0, &mut submitted) },
                "SubmitIoRing",
            )?;
            for active in self.active.iter_mut().flatten() {
                if active.queued && !active.submitted {
                    active.submitted = true;
                }
            }
            self.operations_may_be_in_flight = true;
            Ok(())
        }

        fn wait_for_completion(&self) -> Result<bool, String> {
            let mut submitted = 0;
            let status =
                unsafe { SubmitIoRing(self.ring, 1, COMPLETION_WAIT_MILLISECONDS, &mut submitted) };
            if status == WAIT_TIMEOUT {
                Ok(false)
            } else {
                check_hr(status, "SubmitIoRing wait").map(|()| true)
            }
        }

        fn validate_completion(
            &self,
            completion: &IoRingCompletion,
        ) -> Result<(usize, u64, bool), String> {
            let (slot, generation) = decode_completion(completion.user_data)?;
            if slot >= self.active.len() {
                return Err(format!("completion references invalid slot {slot}"));
            }
            let active = self.active[slot]
                .as_ref()
                .ok_or_else(|| format!("completion references inactive slot {slot}"))?;
            if active.completion.slot != slot
                || active.completion.generation != generation
                || active.user_data != completion.user_data
            {
                return Err(format!("stale or unexpected completion for slot {slot}"));
            }
            Ok((slot, generation, active.is_write))
        }

        fn process_completion(
            &mut self,
            switch: &mut Switch,
            completion: IoRingCompletion,
        ) -> Result<bool, String> {
            let (slot, generation, is_write) = self.validate_completion(&completion)?;
            let slot_completion = wintap_switch_core::SlotCompletion { slot, generation };
            if is_device_busy(completion.result_code) {
                self.retry_busy_operation(slot)?;
                return Ok(is_write);
            }
            self.active[slot] = None;
            if completion.result_code != S_OK {
                self.pool
                    .cancel(slot_completion)
                    .map_err(|error| format!("failed operation completion: {error:?}"))?;
                return Err(format!(
                    "I/O-ring operation failed with HRESULT 0x{:08X}",
                    completion.result_code
                ));
            }
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
                            return Ok(true);
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
                                    encode_completion(slot, generation)?,
                                    IORING_SQE_FLAG_NONE,
                                )
                            },
                            "BuildIoRingWriteFile",
                        )?;
                        self.active[slot] = Some(ActiveOperation {
                            completion: slot_completion,
                            handle: peer,
                            user_data: encode_completion(slot, generation)?,
                            is_write: true,
                            length: length as Dword,
                            queued: true,
                            submitted: false,
                            busy_retries: 0,
                            drain_before_cancellation: false,
                        });
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
            Ok(!is_write)
        }

        fn run(&mut self) -> Result<(), String> {
            let result = self.run_until_stopped();
            let shutdown_result = self.shutdown();
            match (result, shutdown_result) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(error)) => Err(error),
                (Err(error), Err(shutdown_error)) => {
                    Err(format!("{error}; shutdown failed: {shutdown_error}"))
                }
            }
        }

        fn run_until_stopped(&mut self) -> Result<(), String> {
            let mut switch = Switch::static_pair();
            loop {
                if STOP_REQUESTED.load(Ordering::SeqCst) {
                    self.stats.report(true);
                    return Ok(());
                }
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                if status == S_FALSE {
                    let signaled = self.wait_for_completion()?;
                    self.stats.record_wait(signaled);
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                let mut reads = 0;
                let mut writes = 0;
                if self.process_completion(&mut switch, unsafe { completion.assume_init() })? {
                    reads += 1;
                } else {
                    writes += 1;
                }

                loop {
                    let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                    let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                    if status == S_FALSE {
                        break;
                    }
                    check_hr(status, "PopIoRingCompletion")?;
                    if self.process_completion(&mut switch, unsafe { completion.assume_init() })? {
                        reads += 1;
                    } else {
                        writes += 1;
                    }
                }

                self.stats.record_batch(reads, writes);
                self.submit_pending_operations()?;
            }
        }

        fn shutdown(&mut self) -> Result<(), String> {
            self.retire_unqueued_operations()?;
            for active in self.active.iter_mut().flatten() {
                if active.is_write && active.queued && !active.submitted {
                    active.drain_before_cancellation = true;
                }
            }

            self.submit_pending_operations()?;

            let mut cancellation_error = None;
            while self
                .active
                .iter()
                .flatten()
                .any(|active| active.drain_before_cancellation)
            {
                self.wait_for_shutdown_completion(&mut cancellation_error)?;
            }

            while self.active.iter().any(Option::is_some)
                || self.cancellations.iter().any(Option::is_some)
            {
                let (queued, queue_was_full) = self.queue_shutdown_cancellations()?;
                if queued || queue_was_full {
                    self.submit_cancellation_batch()?;
                }
                if !self.try_process_shutdown_completion(&mut cancellation_error)? {
                    self.wait_for_completion()?;
                }
            }
            self.operations_may_be_in_flight = false;
            match cancellation_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn retry_busy_operation(&mut self, slot: usize) -> Result<(), String> {
            let (handle, length, user_data, is_write, busy_retries) = {
                let active = self.active[slot]
                    .as_mut()
                    .ok_or_else(|| format!("busy completion references inactive slot {slot}"))?;
                if !active.submitted {
                    return Err(format!(
                        "busy completion references inactive operation slot {slot}"
                    ));
                }
                active.queued = false;
                active.submitted = false;
                active.busy_retries = active.busy_retries.saturating_add(1);
                (
                    active.handle,
                    active.length,
                    active.user_data,
                    active.is_write,
                    active.busy_retries,
                )
            };
            std::thread::sleep(busy_retry_delay(busy_retries));
            let status = if is_write {
                unsafe {
                    BuildIoRingWriteFile(
                        self.ring,
                        handle_ref(handle),
                        buffer_ref(slot as Dword),
                        length,
                        0,
                        FILE_WRITE_FLAG_NONE,
                        user_data,
                        IORING_SQE_FLAG_NONE,
                    )
                }
            } else {
                unsafe {
                    BuildIoRingReadFile(
                        self.ring,
                        handle_ref(handle),
                        buffer_ref(slot as Dword),
                        length,
                        0,
                        user_data,
                        IORING_SQE_FLAG_NONE,
                    )
                }
            };
            check_hr(
                status,
                if is_write {
                    "BuildIoRingWriteFile retry"
                } else {
                    "BuildIoRingReadFile retry"
                },
            )?;
            self.active[slot]
                .as_mut()
                .ok_or_else(|| format!("busy retry lost operation slot {slot}"))?
                .queued = true;
            Ok(())
        }

        fn retire_unqueued_operations(&mut self) -> Result<(), String> {
            for slot in 0..self.active.len() {
                let Some(active) = self.active[slot] else {
                    continue;
                };
                if active.queued {
                    continue;
                }
                self.pool
                    .cancel(active.completion)
                    .map_err(|error| format!("retire unscheduled operation: {error:?}"))?;
                self.active[slot] = None;
            }
            Ok(())
        }

        fn queue_shutdown_cancellations(&mut self) -> Result<(bool, bool), String> {
            let mut queued = false;
            let mut queue_was_full = false;
            let mut count = 0;
            for slot in 0..self.active.len() {
                if count == self.submission_queue_size {
                    break;
                }
                let Some(active) = self.active[slot].as_ref() else {
                    continue;
                };
                if !active.queued || !active.submitted || self.cancellations[slot].is_some() {
                    continue;
                }
                let operation = active.user_data;
                let status = unsafe {
                    BuildIoRingCancelRequest(
                        self.ring,
                        handle_ref(active.handle),
                        operation,
                        CANCEL_COMPLETION_MARKER | operation,
                    )
                };
                if status == IORING_E_SUBMISSION_QUEUE_FULL {
                    queue_was_full = true;
                    break;
                }
                check_hr(status, "BuildIoRingCancelRequest")?;
                self.cancellations[slot] = Some(operation);
                queued = true;
                count += 1;
            }
            Ok((queued, queue_was_full))
        }

        fn submit_cancellation_batch(&self) -> Result<(), String> {
            let mut submitted = 0;
            check_hr(
                unsafe { SubmitIoRing(self.ring, 0, 0, &mut submitted) },
                "SubmitIoRing cancellation batch",
            )
        }

        fn wait_for_shutdown_completion(
            &mut self,
            cancellation_error: &mut Option<String>,
        ) -> Result<(), String> {
            loop {
                if self.try_process_shutdown_completion(cancellation_error)? {
                    return Ok(());
                }
                self.wait_for_completion()?;
            }
        }

        fn try_process_shutdown_completion(
            &mut self,
            cancellation_error: &mut Option<String>,
        ) -> Result<bool, String> {
            let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
            let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
            if status == S_FALSE {
                return Ok(false);
            }
            check_hr(status, "PopIoRingCompletion")?;
            let completion = unsafe { completion.assume_init() };
            if completion.user_data & CANCEL_COMPLETION_MARKER != 0 {
                let operation = completion.user_data & !CANCEL_COMPLETION_MARKER;
                let (slot, _) = decode_completion(operation)?;
                if slot >= self.active.len() {
                    return Err(format!("cancellation references invalid slot {slot}"));
                }
                if self.cancellations[slot] != Some(operation) {
                    return Err(format!(
                        "unexpected cancellation completion for slot {slot}"
                    ));
                }
                self.cancellations[slot] = None;
                if completion.result_code != S_OK && cancellation_error.is_none() {
                    *cancellation_error = Some(format!(
                        "I/O-ring cancellation failed with HRESULT 0x{:08X}",
                        completion.result_code
                    ));
                }
                return Ok(true);
            }

            let (slot, generation, _) = self.validate_completion(&completion)?;
            let slot_completion = wintap_switch_core::SlotCompletion { slot, generation };
            self.pool
                .cancel(slot_completion)
                .map_err(|error| format!("cancel completion: {error:?}"))?;
            self.active[slot] = None;
            Ok(true)
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

    fn encode_completion(slot: usize, generation: u64) -> Result<Ulonglong, String> {
        let slot = Ulonglong::try_from(slot)
            .map_err(|_| "slot does not fit completion identity".to_string())?;
        if slot > SLOT_MASK {
            return Err("slot count exceeds completion identity capacity".to_string());
        }
        if generation == 0 || generation > GENERATION_MASK {
            return Err("slot generation exceeds completion identity capacity".to_string());
        }
        Ok(slot | (generation << GENERATION_SHIFT))
    }

    fn decode_completion(value: Ulonglong) -> Result<(usize, u64), String> {
        if value & CANCEL_COMPLETION_MARKER != 0 {
            return Err("completion identity contains cancellation marker".to_string());
        }
        let slot = (value & SLOT_MASK) as usize;
        let generation = (value >> GENERATION_SHIFT) & GENERATION_MASK;
        if generation == 0 {
            return Err("completion identity contains invalid generation".to_string());
        }
        Ok((slot, generation))
    }

    fn is_device_busy(status: HResult) -> bool {
        status == HRESULT_FROM_NT_STATUS_DEVICE_BUSY
            || status == HRESULT_FROM_WIN32_INVALID_USER_BUFFER
    }

    fn busy_retry_delay(retries: u32) -> Duration {
        let shift = retries.saturating_sub(1).min(6);
        BUSY_RETRY_INITIAL_DELAY
            .checked_mul(1_u32 << shift)
            .unwrap_or(BUSY_RETRY_MAX_DELAY)
            .min(BUSY_RETRY_MAX_DELAY)
    }

    fn check_hr(status: HResult, operation: &str) -> Result<(), String> {
        if status == S_OK {
            Ok(())
        } else {
            Err(format!("{operation} failed with HRESULT 0x{status:08X}"))
        }
    }

    fn parse_arguments() -> Result<(usize, bool), String> {
        let mut args = env::args().skip(1);
        let mut read_depth = DEFAULT_READ_DEPTH;
        let mut stats_enabled = false;
        while let Some(argument) = args.next() {
            if argument == "--read-depth" {
                let value = args
                    .next()
                    .ok_or_else(|| "--read-depth requires a value".to_string())?;
                read_depth = value
                    .parse()
                    .map_err(|_| format!("invalid read depth '{value}'"))?;
            } else if argument == "--stats" {
                stats_enabled = true;
            } else if argument == "--help" || argument == "-h" {
                println!("Usage: wintap-switch.exe [--read-depth <positive even total>] [--stats]");
                println!("Default read depth: {DEFAULT_READ_DEPTH}");
                println!("--stats reports I/O-ring batching counters every 5 seconds");
                std::process::exit(0);
            } else {
                return Err(format!("unknown argument '{argument}'"));
            }
        }
        Ok((read_depth, stats_enabled))
    }

    pub fn run() -> Result<(), String> {
        let (read_depth, stats_enabled) = parse_arguments()?;
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err("SetConsoleCtrlHandler failed".to_string());
        }
        let result = Runtime::start(read_depth, stats_enabled)?.run();
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
