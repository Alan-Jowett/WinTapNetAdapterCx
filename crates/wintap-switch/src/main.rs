// SPDX-License-Identifier: MIT
// Copyright (c) 2026 WinTapNetAdapterCx contributors
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
        ADAPTIVE_INTEREST_ALL, ADAPTIVE_INTEREST_READABLE, ADAPTIVE_INTEREST_WRITABLE,
        ADAPTIVE_POLLING_PROTOCOL_VERSION,
        AdaptiveEnableRequest, AdaptiveEnableResponse, AdaptiveEndpointCapability,
        AdaptiveWaitRequest, AdaptiveWaitResponse, BufferPool, EndpointId, FRAME_MAXIMUM,
        ForwardingError, IoRingCapabilities, IoRingVersion, Switch,
        TAP_IOCTL_ENABLE_ADAPTIVE_POLLING, TAP_IOCTL_WAIT_FOR_CHANGE, select_adaptive_polling,
        select_io_ring_version,
    };

    type Handle = *mut core::ffi::c_void;
    type HResult = i32;
    type Dword = u32;
    type Ulonglong = u64;
    type Guid = wintap_switch_core::Guid;

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
    const HRESULT_FROM_NT_STATUS_NO_MORE_ENTRIES: HResult = 0x9000_001Au32 as HResult;
    const STATUS_NO_MORE_ENTRIES: HResult = 0x8000_001Au32 as HResult;
    const HRESULT_FROM_WIN32_ERROR_NO_MORE_ITEMS: HResult = 0x8007_0103u32 as HResult;
    const HRESULT_FROM_WIN32_INVALID_USER_BUFFER: HResult = 0x8007_06F8u32 as HResult;
    const HRESULT_FROM_WIN32_ERROR_BUSY: HResult = 0x8007_00AAu32 as HResult;
    const IORING_OP_READ: Dword = 1;
    const IORING_OP_WRITE: Dword = 5;
    const IORING_SQE_FLAG_NONE: Dword = 0;
    const IORING_VERSION_3: Dword = 300;
    const IORING_REF_RAW: Dword = 0;
    const FILE_WRITE_FLAG_NONE: Dword = 0;
    const CTRL_C_EVENT: Dword = 0;
    const CTRL_CLOSE_EVENT: Dword = 2;
    const CANCEL_COMPLETION_MARKER: Ulonglong = 1_u64 << 63;
    const DEFAULT_COMPLETION_WAIT_MILLISECONDS: Dword = 1;
    const DEFAULT_WAIT_OPERATIONS: Dword = 32;
    const BUSY_RETRY_INITIAL_DELAY: Duration = Duration::from_millis(1);
    const BUSY_RETRY_MAX_DELAY: Duration = Duration::from_millis(64);
    const DEFAULT_ADAPTIVE_POLLING_BUDGET: Duration = Duration::from_micros(100);
    const ENDPOINT_COUNT: usize = 2;
    const DEFAULT_READ_DEPTH: usize = 128;
    const STATS_REPORT_INTERVAL: Duration = Duration::from_secs(5);
    const MAX_BUSY_RETRIES: u32 = 8;
    const SLOT_BITS: u32 = 31;
    const GENERATION_SHIFT: u32 = SLOT_BITS;
    const GENERATION_BITS: u32 = 32;
    const SLOT_MASK: Ulonglong = (1_u64 << SLOT_BITS) - 1;
    const GENERATION_MASK: Ulonglong = (1_u64 << GENERATION_BITS) - 1;
    const ERROR_SUCCESS: Dword = 0;
    const ERROR_INVALID_FUNCTION: Dword = 1;
    const ERROR_NOT_SUPPORTED: Dword = 50;
    const ERROR_SHARING_VIOLATION: Dword = 32;
    const ERROR_BUSY: Dword = 170;
    const ERROR_IO_PENDING: Dword = 997;
    const ERROR_OPERATION_ABORTED: Dword = 995;
    const ERROR_NOT_FOUND: Dword = 1168;
    const WAIT_OBJECT_0: Dword = 0;
    const WAIT_TIMEOUT_RESULT: Dword = 258;
    const WAIT_FAILED: Dword = 0xffff_ffff;
    const CLEANUP_REOPEN_ATTEMPTS: u32 = 50;
    const CLEANUP_REOPEN_DELAY: Duration = Duration::from_millis(10);
    #[repr(C)]
    struct RawIoRingCapabilities {
        max_version: Dword,
        _reserved: [Dword; 15],
    }

    #[repr(C)]
    struct MibIfRow2 {
        interface_luid: Ulonglong,
        interface_index: Dword,
        interface_guid: Guid,
        alias: [u16; 257],
        description: [u16; 257],
        physical_address_length: Dword,
        physical_address: [u8; 32],
        permanent_physical_address: [u8; 32],
        mtu: Dword,
        interface_type: Dword,
        tunnel_type: Dword,
        media_type: Dword,
        physical_medium_type: Dword,
        access_type: Dword,
        direction_type: Dword,
        interface_and_oper_status_flags: u8,
        oper_status: Dword,
        admin_status: Dword,
        media_connect_state: Dword,
        network_guid: Guid,
        connection_type: Dword,
        transmit_link_speed: Ulonglong,
        receive_link_speed: Ulonglong,
        in_octets: Ulonglong,
        in_ucast_pkts: Ulonglong,
        in_nucast_pkts: Ulonglong,
        in_discards: Ulonglong,
        in_errors: Ulonglong,
        in_unknown_protos: Ulonglong,
        in_ucast_octets: Ulonglong,
        in_multicast_octets: Ulonglong,
        in_broadcast_octets: Ulonglong,
        out_octets: Ulonglong,
        out_ucast_pkts: Ulonglong,
        out_nucast_pkts: Ulonglong,
        out_discards: Ulonglong,
        out_errors: Ulonglong,
        out_ucast_octets: Ulonglong,
        out_multicast_octets: Ulonglong,
        out_broadcast_octets: Ulonglong,
        out_qlen: Ulonglong,
    }

    #[repr(C)]
    struct MibIfTable2 {
        num_entries: Dword,
        _padding: Dword,
        table: [MibIfRow2; 0],
    }

    #[repr(C)]
    struct IoRingCreateFlags {
        required: Dword,
        advisory: Dword,
    }

    fn parse_guid(value: &str) -> Result<Guid, String> {
        let parts: Vec<&str> = value.split('-').collect();
        if parts.len() != 5
            || parts[0].len() != 8
            || parts[1].len() != 4
            || parts[2].len() != 4
            || parts[3].len() != 4
            || parts[4].len() != 12
        {
            return Err(format!("invalid GUID '{value}'"));
        }
        let parse = |part: &str| {
            u64::from_str_radix(part, 16).map_err(|_| format!("invalid GUID '{value}'"))
        };
        let data1 =
            u32::try_from(parse(parts[0])?).map_err(|_| format!("invalid GUID '{value}'"))?;
        let data2 =
            u16::try_from(parse(parts[1])?).map_err(|_| format!("invalid GUID '{value}'"))?;
        let data3 =
            u16::try_from(parse(parts[2])?).map_err(|_| format!("invalid GUID '{value}'"))?;
        let data4_a =
            u16::try_from(parse(parts[3])?).map_err(|_| format!("invalid GUID '{value}'"))?;
        let data4_b =
            u64::try_from(parse(parts[4])?).map_err(|_| format!("invalid GUID '{value}'"))?;
        Ok(Guid {
            data1,
            data2,
            data3,
            data4: [
                (data4_a >> 8) as u8,
                data4_a as u8,
                (data4_b >> 40) as u8,
                (data4_b >> 32) as u8,
                (data4_b >> 24) as u8,
                (data4_b >> 16) as u8,
                (data4_b >> 8) as u8,
                data4_b as u8,
            ],
        })
    }

    fn query_adapter_mtu(guid: &str) -> Result<usize, String> {
        let guid = parse_guid(guid)?;
        let mut table = std::ptr::null_mut();
        let status = unsafe { GetIfTable2(&mut table) };
        if status != ERROR_SUCCESS {
            return Err(format!("GetIfTable2 failed with Win32 error {status}"));
        }
        if table.is_null() {
            return Err("GetIfTable2 returned a null table".to_string());
        }
        let result = unsafe {
            let table_ref = &*table;
            let rows = std::slice::from_raw_parts(
                table_ref.table.as_ptr(),
                table_ref.num_entries as usize,
            );
            rows.iter()
                .find(|row| row.interface_guid == guid)
                .map(|row| row.mtu as usize)
                .ok_or_else(|| {
                    format!("adapter GUID {guid:?} was not found in the interface table")
                })
        };
        unsafe { FreeMibTable(table.cast()) };
        result
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

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: Dword,
        offset_high: Dword,
        event: Handle,
    }

    struct AdaptiveWait {
        event: Handle,
        overlapped: Overlapped,
        request: AdaptiveWaitRequest,
        response: AdaptiveWaitResponse,
        pending: bool,
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
        fn CreateEventW(
            attributes: *mut core::ffi::c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> Handle;
        fn GetLastError() -> Dword;
        fn ResetEvent(event: Handle) -> i32;
        fn DeviceIoControl(
            handle: Handle,
            code: Dword,
            input: *mut core::ffi::c_void,
            input_length: Dword,
            output: *mut core::ffi::c_void,
            output_length: Dword,
            bytes_returned: *mut Dword,
            overlapped: *mut Overlapped,
        ) -> i32;
        fn GetOverlappedResult(
            handle: Handle,
            overlapped: *mut Overlapped,
            bytes_transferred: *mut Dword,
            wait: i32,
        ) -> i32;
        fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> i32;
        fn WaitForMultipleObjects(
            count: Dword,
            handles: *const Handle,
            wait_all: i32,
            milliseconds: Dword,
        ) -> Dword;
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

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetIfTable2(table: *mut *mut MibIfTable2) -> Dword;
        fn FreeMibTable(table: *mut core::ffi::c_void);
    }

    struct Endpoint {
        id: EndpointId,
        guid: String,
        handle: Handle,
        adaptive_wait: Option<AdaptiveWait>,
    }

    struct EndpointConfig {
        guid: String,
        interface_path: String,
    }

    impl Drop for Endpoint {
        fn drop(&mut self) {
            unsafe {
                if let Some(wait) = self.adaptive_wait.take() {
                    CloseHandle(wait.event);
                }
                CloseHandle(self.handle);
            }
        }
    }

    impl AdaptiveWait {
        fn new() -> Result<Self, String> {
            let event = unsafe { CreateEventW(null_mut(), 0, 0, std::ptr::null()) };
            if event.is_null() {
                return Err(format!("CreateEventW failed with error {}", unsafe {
                    GetLastError()
                }));
            }
            Ok(Self {
                event,
                overlapped: Overlapped {
                    internal: 0,
                    internal_high: 0,
                    offset: 0,
                    offset_high: 0,
                    event,
                },
                request: AdaptiveWaitRequest {
                    version: ADAPTIVE_POLLING_PROTOCOL_VERSION,
                    interest: 0,
                },
                response: AdaptiveWaitResponse { satisfied: 0 },
                pending: false,
            })
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
        wait_operations: Dword,
        completion_wait_milliseconds: Dword,
        frame_maximum: usize,
        adaptive_polling: bool,
        polling_budget: Duration,
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
        empty_reads: u64,
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
                empty_reads: 0,
                max_batch: 0,
            }
        }

        fn record_wait_submission(&mut self) {
            self.wait_calls += 1;
        }

        fn record_signaled_wake(&mut self) {
            self.signaled_wakes += 1;
        }

        fn record_empty_read(&mut self) {
            self.empty_reads += 1;
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
            let message = format!(
                "io-ring stats: elapsed={elapsed:.1}s waits={} signaled_wakes={} batches={} completions={} reads={} writes={} empty_reads={} avg_completions_per_batch={average_batch:.2} avg_reads_per_wake={average_reads_per_wake:.2} max_batch={}",
                self.wait_calls,
                self.signaled_wakes,
                self.batches,
                self.completions,
                self.reads,
                self.writes,
                self.empty_reads,
                self.max_batch,
            );
            eprintln!("{message}");
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
        fn start(
            total_depth: usize,
            stats_enabled: bool,
            wait_operations: Dword,
            completion_wait_milliseconds: Dword,
            polling_budget: Duration,
            endpoint_configs: [EndpointConfig; ENDPOINT_COUNT],
        ) -> Result<Self, String> {
            if total_depth == 0 || total_depth % ENDPOINT_COUNT != 0 {
                return Err("read depth must be a positive even value".to_string());
            }
            if total_depth > SLOT_MASK as usize + 1 {
                return Err("read depth exceeds completion identity capacity".to_string());
            }
            if wait_operations == 0 || wait_operations as usize > total_depth {
                return Err(format!(
                    "wait operations must be between 1 and read depth ({total_depth})"
                ));
            }
            let reads_per_endpoint = total_depth / ENDPOINT_COUNT;
            let mtu_a = query_adapter_mtu(&endpoint_configs[0].guid)?;
            let mtu_b = query_adapter_mtu(&endpoint_configs[1].guid)?;
            if mtu_a != mtu_b {
                return Err(format!("endpoint MTUs differ: {} and {}", mtu_a, mtu_b));
            }
            if !(1_500..=65_521).contains(&mtu_a) {
                return Err(format!("endpoint MTU {mtu_a} is outside supported limits"));
            }
            let frame_maximum = mtu_a
                .checked_add(wintap_switch_core::FRAME_MINIMUM)
                .filter(|value| *value <= FRAME_MAXIMUM)
                .ok_or_else(|| format!("endpoint MTU {mtu_a} exceeds switch frame limits"))?;
            let total_bytes = total_depth
                .checked_mul(frame_maximum)
                .ok_or_else(|| "read depth buffer-size calculation overflowed".to_string())?;
            let queue_size = Dword::try_from(total_depth)
                .map_err(|_| "read depth exceeds I/O-ring limits".to_string())?;
            let maximum_version = query_capabilities()?;

            let [first, second] = endpoint_configs;
            let mut endpoints = open_endpoints(&first, &second).map_err(|error| error.message())?;
            let (adaptive_polling, reopen_for_legacy) = negotiate_adaptive_polling(&mut endpoints)?;
            if reopen_for_legacy {
                drop(endpoints);
                endpoints = reopen_legacy_endpoints_after_cleanup(&first, &second)?;
            }
            eprintln!(
                "selected dynamic endpoints: {}={:?} {}={:?}; adaptive polling={adaptive_polling}",
                endpoints[0].guid, endpoints[0].id, endpoints[1].guid, endpoints[1].id
            );
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
            if total_bytes < frame_maximum {
                return Err("read depth buffer-size calculation was invalid".to_string());
            }
            for _ in 0..total_depth {
                let mut buffer = Vec::new();
                buffer
                    .try_reserve_exact(frame_maximum)
                    .map_err(|_| "buffer pool allocation failed".to_string())?;
                buffer.resize(frame_maximum, 0);
                buffers.push(buffer);
            }
            let mut registrations = Vec::new();
            registrations
                .try_reserve_exact(buffers.len())
                .map_err(|_| "buffer registration allocation failed".to_string())?;
            for buffer in &buffers {
                registrations.push(IoRingBufferInfo {
                    address: buffer.as_ptr() as *mut u8,
                    length: frame_maximum as Dword,
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
                wait_operations,
                completion_wait_milliseconds,
                frame_maximum,
                adaptive_polling,
                polling_budget,
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
                        self.frame_maximum as Dword,
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
                length: self.frame_maximum as Dword,
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
            let status = unsafe {
                SubmitIoRing(
                    self.ring,
                    self.wait_operations,
                    self.completion_wait_milliseconds,
                    &mut submitted,
                )
            };
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
        ) -> Result<Option<bool>, String> {
            let (slot, generation, is_write) = self.validate_completion(&completion)?;
            let slot_completion = wintap_switch_core::SlotCompletion { slot, generation };
            if is_device_busy(completion.result_code) {
                self.retry_busy_operation(slot)?;
                return Ok(None);
            }
            self.active[slot] = None;
            if self.adaptive_polling && !is_write && is_no_more_entries(completion.result_code) {
                self.pool
                    .cancel(slot_completion)
                    .map_err(|error| format!("empty adaptive read completion: {error:?}"))?;
                self.stats.record_empty_read();
                return Ok(None);
            }
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
                if length <= self.frame_maximum {
                    let recipients = match switch.forward(source, &self.buffers[slot][..length]) {
                        Ok(recipients) => recipients,
                        Err(ForwardingError::InvalidFrame(_)) => {
                            self.pool
                                .complete_dispatch(slot_completion)
                                .map_err(|error| format!("invalid frame: {error:?}"))?;
                            self.post_read(slot)?;
                            return Ok(Some(true));
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
            Ok(Some(!is_write))
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
            if self.adaptive_polling {
                return self.run_until_stopped_adaptive();
            }
            self.run_until_stopped_legacy()
        }

        fn run_until_stopped_legacy(&mut self) -> Result<(), String> {
            let mut switch =
                Switch::from_endpoints(self.endpoints.iter().map(|endpoint| endpoint.id))
                    .map_err(|error| format!("selected endpoint collection: {error:?}"))?;
            loop {
                if STOP_REQUESTED.load(Ordering::SeqCst) {
                    self.stats.report(true);
                    return Ok(());
                }
                let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                if status == S_FALSE {
                    let signaled = self.wait_for_completion()?;
                    self.stats.record_wait_submission();
                    if signaled {
                        self.stats.record_signaled_wake();
                    }
                    self.stats.report(false);
                    continue;
                }
                check_hr(status, "PopIoRingCompletion")?;
                let mut reads = 0;
                let mut writes = 0;
                if let Some(is_read) =
                    self.process_completion(&mut switch, unsafe { completion.assume_init() })?
                {
                    if is_read {
                        reads += 1;
                    } else {
                        writes += 1;
                    }
                }

                loop {
                    let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                    let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                    if status == S_FALSE {
                        break;
                    }
                    check_hr(status, "PopIoRingCompletion")?;
                    if let Some(is_read) =
                        self.process_completion(&mut switch, unsafe { completion.assume_init() })?
                    {
                        if is_read {
                            reads += 1;
                        } else {
                            writes += 1;
                        }
                    }
                }

                self.stats.record_batch(reads, writes);
                self.submit_pending_operations()?;
            }
        }

        fn run_until_stopped_adaptive(&mut self) -> Result<(), String> {
            let mut switch =
                Switch::from_endpoints(self.endpoints.iter().map(|endpoint| endpoint.id))
                    .map_err(|error| format!("selected endpoint collection: {error:?}"))?;
            let mut idle_since = Instant::now();
            loop {
                if STOP_REQUESTED.load(Ordering::SeqCst) {
                    self.stats.report(true);
                    return Ok(());
                }

                let mut saw_completion = false;
                let mut made_progress = false;
                let mut reads = 0;
                let mut writes = 0;
                loop {
                    let mut completion = MaybeUninit::<IoRingCompletion>::zeroed();
                    let status = unsafe { PopIoRingCompletion(self.ring, completion.as_mut_ptr()) };
                    if status == S_FALSE {
                        break;
                    }
                    check_hr(status, "PopIoRingCompletion")?;
                    saw_completion = true;
                    if let Some(is_read) =
                        self.process_completion(&mut switch, unsafe { completion.assume_init() })?
                    {
                        made_progress = true;
                        if is_read {
                            reads += 1;
                        } else {
                            writes += 1;
                        }
                    }
                }

                if saw_completion {
                    self.submit_pending_operations()?;
                    self.stats.record_batch(reads, writes);
                    self.stats.report(false);
                }
                if made_progress {
                    idle_since = Instant::now();
                    continue;
                }
                if idle_since.elapsed() >= self.polling_budget {
                    self.stats.record_wait_submission();
                    let signaled = self.wait_for_adaptive_change()?;
                    if signaled {
                        self.stats.record_signaled_wake();
                    }
                    if signaled {
                        self.post_adaptive_idle_reads()?;
                        self.submit_pending_operations()?;
                        idle_since = Instant::now();
                    }
                } else {
                    std::thread::yield_now();
                }
            }
        }

        fn post_adaptive_idle_reads(&mut self) -> Result<(), String> {
            for slot in 0..self.active.len() {
                if self.active[slot].is_none() {
                    self.post_read(slot)?;
                }
            }
            Ok(())
        }

        fn adaptive_interest_for_endpoint(&self, endpoint: usize) -> u32 {
            let handle = self.endpoints[endpoint].handle;
            let backpressured_write = self.active.iter().flatten().any(|active| {
                active.is_write && active.handle == handle && active.busy_retries != 0
            });
            ADAPTIVE_INTEREST_READABLE
                | if backpressured_write {
                    ADAPTIVE_INTEREST_WRITABLE
                } else {
                    0
                }
        }

        fn wait_for_adaptive_change(&mut self) -> Result<bool, String> {
            let mut pending_events = [core::ptr::null_mut(); ENDPOINT_COUNT];
            let mut pending_indices = [0usize; ENDPOINT_COUNT];
            let mut pending_count = 0usize;
            let mut immediately_ready = false;
            for endpoint in 0..ENDPOINT_COUNT {
                let interest = self.adaptive_interest_for_endpoint(endpoint);
                if self.submit_adaptive_wait(endpoint, interest)? {
                    immediately_ready = true;
                    continue;
                }
                let wait = self.endpoints[endpoint]
                    .adaptive_wait
                    .as_ref()
                    .expect("adaptive endpoint has a wait context");
                if wait.pending {
                    pending_events[pending_count] = wait.event;
                    pending_indices[pending_count] = endpoint;
                    pending_count += 1;
                }
            }
            if immediately_ready {
                // Keep waits already pended on the other endpoint armed. A writable
                // result can be immediate while the peer still needs a readable wake.
                return Ok(true);
            }
            if pending_count == 0 {
                return Ok(true);
            }
            let result = loop {
                let result = unsafe {
                    WaitForMultipleObjects(
                        Dword::try_from(pending_count).expect("endpoint count fits Dword"),
                        pending_events.as_ptr(),
                        0,
                        self.completion_wait_milliseconds,
                    )
                };
                if result != WAIT_TIMEOUT_RESULT || STOP_REQUESTED.load(Ordering::SeqCst) {
                    break result;
                }
                self.stats.report(false);
            };
            if result == WAIT_FAILED {
                return Err(format!(
                    "WaitForMultipleObjects failed with error {}",
                    unsafe { GetLastError() }
                ));
            }
            if result == WAIT_TIMEOUT_RESULT {
                return Ok(false);
            }
            let offset = result
                .checked_sub(WAIT_OBJECT_0)
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value < pending_count)
                .ok_or_else(|| {
                    format!("WaitForMultipleObjects returned unexpected value {result}")
                })?;
            self.complete_adaptive_wait(pending_indices[offset])?;
            Ok(true)
        }

        fn submit_adaptive_wait(&mut self, endpoint: usize, interest: u32) -> Result<bool, String> {
            let endpoint = &mut self.endpoints[endpoint];
            let wait = endpoint.adaptive_wait.get_or_insert(AdaptiveWait::new()?);
            if wait.pending {
                if wait.request.interest == interest {
                    return Ok(false);
                }
                if unsafe { CancelIoEx(endpoint.handle, &mut wait.overlapped) } == 0 {
                    let error = unsafe { GetLastError() };
                    if error != ERROR_NOT_FOUND {
                        return Err(format!(
                            "CancelIoEx for WAIT_FOR_CHANGE rearm failed for {} with Win32 error {error}",
                            endpoint.guid
                        ));
                    }
                }
                let mut bytes = 0;
                if unsafe {
                    GetOverlappedResult(endpoint.handle, &mut wait.overlapped, &mut bytes, 1)
                } == 0
                {
                    let error = unsafe { GetLastError() };
                    if error != ERROR_OPERATION_ABORTED && error != ERROR_NOT_FOUND {
                        return Err(format!(
                            "draining WAIT_FOR_CHANGE rearm failed for {} with Win32 error {error}",
                            endpoint.guid
                        ));
                    }
                }
                wait.pending = false;
            }
            wait.request.interest = interest;
            wait.response.satisfied = 0;
            wait.overlapped = Overlapped {
                internal: 0,
                internal_high: 0,
                offset: 0,
                offset_high: 0,
                event: wait.event,
            };
            if unsafe { ResetEvent(wait.event) } == 0 {
                return Err(format!(
                    "ResetEvent failed for {} with Win32 error {}",
                    endpoint.guid,
                    unsafe { GetLastError() }
                ));
            }
            let mut bytes = 0;
            let completed = unsafe {
                DeviceIoControl(
                    endpoint.handle,
                    TAP_IOCTL_WAIT_FOR_CHANGE,
                    (&mut wait.request as *mut AdaptiveWaitRequest).cast(),
                    Dword::try_from(std::mem::size_of::<AdaptiveWaitRequest>())
                        .expect("wait input fits Dword"),
                    (&mut wait.response as *mut AdaptiveWaitResponse).cast(),
                    Dword::try_from(std::mem::size_of::<AdaptiveWaitResponse>())
                        .expect("wait output fits Dword"),
                    &mut bytes,
                    &mut wait.overlapped,
                )
            };
            if completed != 0 {
                validate_adaptive_wait_response(bytes, interest, wait.response)?;
                return Ok(true);
            }
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_PENDING {
                wait.pending = true;
                Ok(false)
            } else {
                Err(format!(
                    "WAIT_FOR_CHANGE failed for {} with Win32 error {error}",
                    endpoint.guid
                ))
            }
        }

        fn complete_adaptive_wait(&mut self, endpoint: usize) -> Result<(), String> {
            let endpoint = &mut self.endpoints[endpoint];
            let wait = endpoint
                .adaptive_wait
                .as_mut()
                .expect("adaptive endpoint has a wait context");
            if !wait.pending {
                return Ok(());
            }
            let mut bytes = 0;
            if unsafe { GetOverlappedResult(endpoint.handle, &mut wait.overlapped, &mut bytes, 0) }
                == 0
            {
                return Err(format!(
                    "WAIT_FOR_CHANGE completion failed for {} with Win32 error {}",
                    endpoint.guid,
                    unsafe { GetLastError() }
                ));
            }
            wait.pending = false;
            validate_adaptive_wait_response(bytes, wait.request.interest, wait.response)
        }

        fn cancel_adaptive_waits(&mut self) -> Result<(), String> {
            let mut first_error = None;
            for endpoint in &mut self.endpoints {
                let Some(wait) = endpoint.adaptive_wait.as_mut() else {
                    continue;
                };
                if !wait.pending {
                    continue;
                }
                if unsafe { CancelIoEx(endpoint.handle, &mut wait.overlapped) } == 0 {
                    let error = unsafe { GetLastError() };
                    if error != ERROR_NOT_FOUND {
                        first_error.get_or_insert_with(|| {
                            format!(
                            "CancelIoEx for WAIT_FOR_CHANGE failed for {} with Win32 error {error}",
                            endpoint.guid
                        )
                        });
                    }
                }
            }
            for endpoint in &mut self.endpoints {
                let Some(wait) = endpoint.adaptive_wait.as_mut() else {
                    continue;
                };
                if !wait.pending {
                    continue;
                }
                let mut bytes = 0;
                if unsafe {
                    GetOverlappedResult(endpoint.handle, &mut wait.overlapped, &mut bytes, 1)
                } == 0
                {
                    let error = unsafe { GetLastError() };
                    if error != ERROR_OPERATION_ABORTED {
                        first_error.get_or_insert_with(|| {
                            format!(
                                "draining WAIT_FOR_CHANGE failed for {} with Win32 error {error}",
                                endpoint.guid
                            )
                        });
                    }
                }
                wait.pending = false;
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn shutdown(&mut self) -> Result<(), String> {
            let mut cancellation_error = self.cancel_adaptive_waits().err();
            self.retire_unqueued_operations()?;
            for active in self.active.iter_mut().flatten() {
                if active.is_write && active.queued && !active.submitted {
                    active.drain_before_cancellation = true;
                }
            }
            self.submit_pending_operations()?;
            self.submit_pending_operations()?;

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
            let retry = {
                let active = self.active[slot]
                    .as_mut()
                    .ok_or_else(|| format!("busy completion references inactive slot {slot}"))?;
                if !active.submitted {
                    return Err(format!(
                        "busy completion references inactive operation slot {slot}"
                    ));
                }
                if active.busy_retries >= MAX_BUSY_RETRIES {
                    None
                } else {
                    active.queued = false;
                    active.submitted = false;
                    active.busy_retries = active.busy_retries.saturating_add(1);
                    Some((
                        active.handle,
                        active.length,
                        active.user_data,
                        active.is_write,
                        active.busy_retries,
                    ))
                }
            };
            let Some((handle, length, user_data, is_write, busy_retries)) = retry else {
                let completion = self.active[slot]
                    .as_ref()
                    .expect("busy retry slot remains active")
                    .completion;
                self.pool
                    .cancel(completion)
                    .map_err(|error| format!("busy retry exhaustion cleanup: {error:?}"))?;
                self.active[slot] = None;
                return Err(format!(
                    "I/O-ring busy retry limit exceeded for slot {slot}"
                ));
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

    struct EndpointOpenError {
        path: String,
        error: Dword,
    }

    impl EndpointOpenError {
        fn message(&self) -> String {
            format!(
                "CreateFileW failed for {} with Win32 error {}",
                self.path, self.error
            )
        }

        fn cleanup_is_pending(&self) -> bool {
            self.error == ERROR_BUSY || self.error == ERROR_SHARING_VIOLATION
        }
    }

    fn open_endpoint(path: &str) -> Result<Handle, EndpointOpenError> {
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
            Err(EndpointOpenError {
                path: path.to_string(),
                error: unsafe { GetLastError() },
            })
        } else {
            Ok(handle)
        }
    }

    fn open_endpoints(
        first: &EndpointConfig,
        second: &EndpointConfig,
    ) -> Result<[Endpoint; ENDPOINT_COUNT], EndpointOpenError> {
        Ok([
            Endpoint {
                id: EndpointId::new(1),
                handle: open_endpoint(&first.interface_path)?,
                guid: first.guid.clone(),
                adaptive_wait: None,
            },
            Endpoint {
                id: EndpointId::new(2),
                handle: open_endpoint(&second.interface_path)?,
                guid: second.guid.clone(),
                adaptive_wait: None,
            },
        ])
    }

    fn reopen_legacy_endpoints_after_cleanup(
        first: &EndpointConfig,
        second: &EndpointConfig,
    ) -> Result<[Endpoint; ENDPOINT_COUNT], String> {
        for attempt in 0..CLEANUP_REOPEN_ATTEMPTS {
            match open_endpoints(first, second) {
                Ok(endpoints) => return Ok(endpoints),
                Err(error) if error.cleanup_is_pending() => {
                    std::thread::sleep(CLEANUP_REOPEN_DELAY);
                }
                Err(error) => return Err(error.message()),
            }
            if attempt + 1 == CLEANUP_REOPEN_ATTEMPTS {
                break;
            }
        }
        Err(format!(
            "timed out waiting for endpoint cleanup after {} attempts",
            CLEANUP_REOPEN_ATTEMPTS
        ))
    }

    fn negotiate_adaptive_polling(
        endpoints: &mut [Endpoint; ENDPOINT_COUNT],
    ) -> Result<(bool, bool), String> {
        let first = negotiate_adaptive_endpoint(&endpoints[0])?;
        let second = negotiate_adaptive_endpoint(&endpoints[1])?;
        let adaptive_polling = select_adaptive_polling(&[first, second]).is_some();
        let reopen_for_legacy = !adaptive_polling
            && (matches!(first, AdaptiveEndpointCapability::Supported { .. })
                || matches!(second, AdaptiveEndpointCapability::Supported { .. }));
        Ok((adaptive_polling, reopen_for_legacy))
    }

    fn negotiate_adaptive_endpoint(
        endpoint: &Endpoint,
    ) -> Result<AdaptiveEndpointCapability, String> {
        let mut request = AdaptiveEnableRequest {
            version: ADAPTIVE_POLLING_PROTOCOL_VERSION,
            flags: ADAPTIVE_INTEREST_ALL,
        };
        let mut response = AdaptiveEnableResponse {
            version: 0,
            flags: 0,
        };
        match synchronous_device_control(
            endpoint.handle,
            TAP_IOCTL_ENABLE_ADAPTIVE_POLLING,
            &mut request,
            &mut response,
        ) {
            Ok(bytes) => {
                if bytes as usize != std::mem::size_of::<AdaptiveEnableResponse>() {
                    return Err(format!(
                        "adaptive enable returned an invalid response size for {}",
                        endpoint.guid
                    ));
                }
                if response.version != ADAPTIVE_POLLING_PROTOCOL_VERSION
                    || response.flags != ADAPTIVE_INTEREST_ALL
                {
                    eprintln!(
                        "adaptive polling incompatible for {}: version={} flags=0x{:08X}",
                        endpoint.guid, response.version, response.flags
                    );
                    Ok(AdaptiveEndpointCapability::Incompatible)
                } else {
                    eprintln!(
                        "adaptive polling enabled for {}: version={} flags=0x{:08X}",
                        endpoint.guid, response.version, response.flags
                    );
                    Ok(AdaptiveEndpointCapability::Supported {
                        version: response.version,
                        accepted_flags: response.flags,
                    })
                }
            }
            Err(error) if is_adaptive_unsupported(error) => {
                eprintln!(
                    "adaptive polling unsupported for {} with Win32 error {error}",
                    endpoint.guid
                );
                Ok(AdaptiveEndpointCapability::Unsupported)
            }
            Err(error) => Err(format!(
                "adaptive enable failed for {} with Win32 error {error}",
                endpoint.guid
            )),
        }
    }

    fn synchronous_device_control<Input, Output>(
        handle: Handle,
        code: Dword,
        input: &mut Input,
        output: &mut Output,
    ) -> Result<Dword, Dword> {
        let event = unsafe { CreateEventW(null_mut(), 0, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(unsafe { GetLastError() });
        }
        let mut overlapped = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0,
            offset_high: 0,
            event,
        };
        let mut bytes = 0;
        let result = unsafe {
            DeviceIoControl(
                handle,
                code,
                (input as *mut Input).cast(),
                Dword::try_from(std::mem::size_of::<Input>()).expect("protocol input fits Dword"),
                (output as *mut Output).cast(),
                Dword::try_from(std::mem::size_of::<Output>()).expect("protocol output fits Dword"),
                &mut bytes,
                &mut overlapped,
            )
        };
        if result == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                unsafe {
                    CloseHandle(event);
                }
                return Err(error);
            }
            if unsafe { GetOverlappedResult(handle, &mut overlapped, &mut bytes, 1) } == 0 {
                let error = unsafe { GetLastError() };
                unsafe {
                    CloseHandle(event);
                }
                return Err(error);
            }
        }
        unsafe {
            CloseHandle(event);
        }
        Ok(bytes)
    }

    fn is_adaptive_unsupported(error: Dword) -> bool {
        error == ERROR_INVALID_FUNCTION || error == ERROR_NOT_SUPPORTED
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
            || status == HRESULT_FROM_WIN32_ERROR_BUSY
    }

    fn is_no_more_entries(status: HResult) -> bool {
        status == HRESULT_FROM_NT_STATUS_NO_MORE_ENTRIES
            || status == STATUS_NO_MORE_ENTRIES
            || status == HRESULT_FROM_WIN32_ERROR_NO_MORE_ITEMS
    }

    fn validate_adaptive_wait_response(
        bytes: Dword,
        interest: u32,
        response: AdaptiveWaitResponse,
    ) -> Result<(), String> {
        if bytes as usize != std::mem::size_of::<AdaptiveWaitResponse>()
            || response.satisfied == 0
            || response.satisfied & !interest != 0
        {
            return Err("WAIT_FOR_CHANGE returned an invalid readiness response".to_string());
        }
        Ok(())
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

    fn is_guid(value: &str) -> bool {
        value.len() == 36
            && value.bytes().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            })
    }

    fn parse_endpoint(value: String) -> Result<EndpointConfig, String> {
        let (guid, interface_path) = value
            .split_once('=')
            .ok_or_else(|| "--endpoint must be <GUID>=<device-interface-path>".to_string())?;
        if !is_guid(guid) || !interface_path.starts_with(r"\\?\") {
            return Err(format!(
                "invalid dynamic endpoint '{value}'; use <GUID>=\\\\?\\device-interface-path"
            ));
        }
        Ok(EndpointConfig {
            guid: guid.to_ascii_lowercase(),
            interface_path: interface_path.to_string(),
        })
    }

    fn parse_arguments()
    -> Result<(usize, bool, Dword, Dword, Duration, [EndpointConfig; ENDPOINT_COUNT]), String> {
        let mut args = env::args().skip(1);
        let mut read_depth = DEFAULT_READ_DEPTH;
        let mut stats_enabled = false;
        let mut wait_operations = DEFAULT_WAIT_OPERATIONS;
        let mut completion_wait_milliseconds = DEFAULT_COMPLETION_WAIT_MILLISECONDS;
        let mut polling_budget = DEFAULT_ADAPTIVE_POLLING_BUDGET;
        let mut endpoints = Vec::with_capacity(ENDPOINT_COUNT);
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
            } else if argument == "--wait-operations" {
                let value = args
                    .next()
                    .ok_or_else(|| "--wait-operations requires a value".to_string())?;
                wait_operations = value
                    .parse()
                    .map_err(|_| format!("invalid wait operations '{value}'"))?;
            } else if argument == "--completion-timeout-ms" {
                let value = args
                    .next()
                    .ok_or_else(|| "--completion-timeout-ms requires a value".to_string())?;
                completion_wait_milliseconds = value
                    .parse()
                    .map_err(|_| format!("invalid completion timeout '{value}'"))?;
            } else if argument == "--adaptive-polling-budget-us" {
                let value = args
                    .next()
                    .ok_or_else(|| "--adaptive-polling-budget-us requires a value".to_string())?;
                let microseconds: u64 = value
                    .parse()
                    .map_err(|_| format!("invalid adaptive polling budget '{value}'"))?;
                if microseconds == 0 {
                    return Err("adaptive polling budget must be positive".to_string());
                }
                polling_budget = Duration::from_micros(microseconds);
            } else if argument == "--endpoint" {
                let value = args.next().ok_or_else(|| {
                    "--endpoint requires <GUID>=<device-interface-path>".to_string()
                })?;
                let endpoint = parse_endpoint(value)?;
                if endpoints.iter().any(|existing: &EndpointConfig| {
                    existing.guid == endpoint.guid
                        || existing.interface_path == endpoint.interface_path
                }) {
                    return Err("duplicate dynamic endpoint GUID or interface path".to_string());
                }
                endpoints.push(endpoint);
            } else if argument == "--help" || argument == "-h" {
                println!(
                    "Usage: wintap-switch.exe --endpoint <GUID>=<interface> --endpoint <GUID>=<interface> [--read-depth <positive even total>] [--wait-operations <positive>] [--completion-timeout-ms <milliseconds>] [--adaptive-polling-budget-us <positive>] [--stats]"
                );
                println!("Default read depth: {DEFAULT_READ_DEPTH}");
                println!("Default wait operations: {DEFAULT_WAIT_OPERATIONS}");
                println!("Default completion timeout: {DEFAULT_COMPLETION_WAIT_MILLISECONDS} ms");
                println!(
                    "Default adaptive polling budget: {} microseconds",
                    DEFAULT_ADAPTIVE_POLLING_BUDGET.as_micros()
                );
                println!("--stats reports I/O-ring batching counters every 5 seconds");
                println!(
                    "Pass manager-returned GUID/interface pairs; fixed DOS paths are not supported."
                );
                std::process::exit(0);
            } else {
                return Err(format!("unknown argument '{argument}'"));
            }
        }
        let endpoints: [EndpointConfig; ENDPOINT_COUNT] = endpoints
            .try_into()
            .map_err(|entries: Vec<EndpointConfig>| {
                format!(
                    "exactly {ENDPOINT_COUNT} GUID-correlated --endpoint values are required; got {}",
                    entries.len()
                )
            })?;
        Ok((
            read_depth,
            stats_enabled,
            wait_operations,
            completion_wait_milliseconds,
            polling_budget,
            endpoints,
        ))
    }

    pub fn run() -> Result<(), String> {
        let (
            read_depth,
            stats_enabled,
            wait_operations,
            completion_wait_milliseconds,
            polling_budget,
            endpoints,
        ) =
            parse_arguments()?;
        if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
            return Err("SetConsoleCtrlHandler failed".to_string());
        }
        let result = Runtime::start(
            read_depth,
            stats_enabled,
            wait_operations,
            completion_wait_milliseconds,
            polling_budget,
            endpoints,
        )?
        .run();
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
