// SPDX-License-Identifier: MIT
// Copyright (c) 2026 WinTapNetAdapterCx contributors
#![no_std]

extern crate alloc;

use alloc::alloc::alloc;
use alloc::boxed::Box;

#[cfg(not(test))]
extern crate wdk_panic;

mod frame_queue;
mod ring;
use frame_queue::{Frame, FrameQueue, QueueError, QueueState, FRAME_STORAGE_SIZE};
use ring::{advance_index, fragment_at, fragment_virtual_address, increment_index, packet_at};

use core::alloc::Layout;
use core::ffi::c_void;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{
    AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    call_unsafe_wdf_function_binding, DRIVER_OBJECT, GUID, NTSTATUS, PCUNICODE_STRING,
    PDRIVER_OBJECT, STATUS_DEVICE_BUSY, ULONG, UNICODE_STRING, WDFCMRESLIST, WDFDEVICE,
    WDFDEVICE_INIT, WDFDRIVER, WDFFILEOBJECT, WDFOBJECT, WDFQUEUE, WDFREQUEST, WDFSPINLOCK,
    WDFWAITLOCK, WDFWORKITEM, WDF_DRIVER_CONFIG, WDF_FILEOBJECT_CONFIG, WDF_IO_QUEUE_CONFIG,
    WDF_NO_OBJECT_ATTRIBUTES, WDF_OBJECT_ATTRIBUTES, WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_WORKITEM_CONFIG,
};

unsafe extern "C" {
    fn DbgPrintEx(component_id: ULONG, level: ULONG, format: *const i8, ...) -> ULONG;
}

struct WriteLifetimeGuard {
    state: *mut InstanceState,
}

impl WriteLifetimeGuard {
    unsafe fn acquire(state: *mut InstanceState) -> Option<Self> {
        if state.is_null() || unsafe { (*state).write_lifetime_lock.is_null() } {
            return None;
        }
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockAcquire,
                (*state).write_lifetime_lock,
                core::ptr::null_mut::<i64>(),
            )
        };
        if status == STATUS_SUCCESS {
            Some(Self { state })
        } else {
            None
        }
    }
}

impl Drop for WriteLifetimeGuard {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockRelease,
                (*self.state).write_lifetime_lock
            );
        }
    }
}

const DPFLTR_IHVDRIVER_ID: ULONG = 77;

fn debug_status(label: &[u8], status: NTSTATUS) {
    let mut format = [0i8; 96];
    let prefix = b"WinTapRust: ";
    let suffix = b" status=0x%08X\n\0";
    let mut offset = 0;
    for byte in prefix {
        format[offset] = *byte as i8;
        offset += 1;
    }
    for byte in label {
        format[offset] = *byte as i8;
        offset += 1;
    }
    for byte in suffix {
        format[offset] = *byte as i8;
        offset += 1;
    }
    unsafe {
        DbgPrintEx(DPFLTR_IHVDRIVER_ID, 0, format.as_ptr(), status as u32);
    }
}

fn debug_marker(label: &[u8]) {
    debug_status(label, STATUS_SUCCESS);
}

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_CANCELLED: NTSTATUS = 0xC000_0120_u32 as i32;
const STATUS_DEVICE_NOT_READY: NTSTATUS = 0xC000_00A3_u32 as i32;
const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009A_u32 as i32;
const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000D_u32 as i32;
const STATUS_INVALID_DEVICE_REQUEST: NTSTATUS = 0xC000_0010_u32 as i32;
const STATUS_INVALID_BUFFER_SIZE: NTSTATUS = 0xC000_0206_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: NTSTATUS = 0xC000_0023_u32 as i32;
const STATUS_NOT_SUPPORTED: NTSTATUS = 0xC000_00BB_u32 as i32;
const STATUS_NO_MORE_ENTRIES: NTSTATUS = 0x8000_001A_u32 as i32;
const INSTANCE_OPEN: u8 = 0;
const INSTANCE_SUSPENDED: u8 = 1;
const INSTANCE_CLOSING: u8 = 2;
const INSTANCE_CLOSED: u8 = 3;
/// No wait is registered; the request slot is free.
const WAIT_FREE: u8 = 0;
/// The dispatch path owns the request and has not marked it cancelable yet.
const WAIT_REGISTERING: u8 = 1;
/// The request is marked cancelable and published for atomic claiming.
const WAIT_PENDING: u8 = 2;
/// A queue transition claimed the wait; the passive worker owns the unmark.
const WAIT_SCHEDULED: u8 = 3;
/// A single owner is inside `WdfRequestUnmarkCancelable` for this request.
const WAIT_UNMARKING: u8 = 4;
/// Teardown claimed a registration that has not finished marking.
const WAIT_TEARDOWN: u8 = 5;
/// The WDF cancellation callback owns or performed terminal completion.
const WAIT_CANCELLED: u8 = 6;
const WAIT_HANDOFF_NONE: u8 = 0;
const WAIT_HANDOFF_CANCEL_ARRIVED: u8 = 1;
const WAIT_HANDOFF_UNMARK_CANCELLED: u8 = 2;
/// State field of the packed adaptive-wait record word.
const WAIT_RECORD_STATE_MASK: u64 = 0xFF;
const WAIT_RECORD_SATISFIED_SHIFT: u32 = 8;
/// Satisfied-readiness field of the packed adaptive-wait record word.
const WAIT_RECORD_SATISFIED_MASK: u64 = 0xFF << WAIT_RECORD_SATISFIED_SHIFT;
/// Monotonic registration sequence field of the packed record word.
const WAIT_RECORD_SEQUENCE_SHIFT: u32 = 16;
/// Admission closers of a direction-specific packet-callback lease word.
///
/// Each quiesce scope owns one bit, so a scope can only readmit callbacks that
/// it closed itself and a nested scope keeps admission closed until it also
/// readmits.
const DATAPATH_CLOSED_POWER: u64 = 1 << 63;
const DATAPATH_CLOSED_HARDWARE: u64 = 1 << 62;
const DATAPATH_CLOSED_OWNER: u64 = 1 << 61;
const DATAPATH_CLOSED_ANY: u64 =
    DATAPATH_CLOSED_POWER | DATAPATH_CLOSED_HARDWARE | DATAPATH_CLOSED_OWNER;
const DATAPATH_LEASE_COUNT: u64 = !DATAPATH_CLOSED_ANY;
const PENDING_READ_LIMIT: usize = 256;
const PENDING_WRITE_LIMIT: usize = 256;
const FRAME_QUEUE_LIMIT: usize = 256;
const FRAME_MINIMUM: usize = 14;
const DEFAULT_MTU: usize = 1_500;
const MAXIMUM_MTU: usize = 65_521;
const MAXIMUM_MULTICAST_ADDRESSES: usize = 64;
const ETHERNET_ADDRESS_LENGTH: usize = 6;
const TAP_IOCTL_ENABLE_ADAPTIVE_POLLING: ULONG = 0x0022_2400;
const TAP_IOCTL_WAIT_FOR_CHANGE: ULONG = 0x0022_2404;
const ADAPTIVE_POLLING_PROTOCOL_VERSION: u32 = 1;
const ADAPTIVE_INTEREST_READABLE: u32 = 1;
const ADAPTIVE_INTEREST_WRITABLE: u32 = 2;
const ADAPTIVE_INTEREST_SUPPORTED: u32 = ADAPTIVE_INTEREST_READABLE | ADAPTIVE_INTEREST_WRITABLE;
const TAP_INTERFACE_CLASS: GUID = GUID {
    Data1: 0x25d3_2edf,
    Data2: 0x7c8c,
    Data3: 0x4f09,
    Data4: [0x90, 0x1f, 0x65, 0x0b, 0x23, 0x2e, 0x86, 0x4d],
};

static FRAGMENT_VIRTUAL_ADDRESS_NAME: [u16; 27] = [
    109, 115, 95, 102, 114, 97, 103, 109, 101, 110, 116, 95, 118, 105, 114, 116, 117, 97, 108, 97,
    100, 100, 114, 101, 115, 115, 0,
];
static QUEUE_CONTEXT_NAME: &[u8] = b"WINTAP_QUEUE_CONTEXT\0";
static DEVICE_CONTEXT_NAME: &[u8] = b"WINTAP_DEVICE_CONTEXT\0";
static ADAPTER_CONTEXT_NAME: &[u8] = b"WINTAP_ADAPTER_CONTEXT\0";
static WORK_ITEM_CONTEXT_NAME: &[u8] = b"WINTAP_WORK_ITEM_CONTEXT\0";

#[repr(C)]
#[derive(Clone, Copy)]
struct AdaptiveEnableRequest {
    version: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdaptiveEnableResponse {
    version: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdaptiveWaitRequest {
    version: u32,
    interest: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AdaptiveWaitResponse {
    satisfied: u32,
}

#[repr(C)]
struct InstanceState {
    guid: GUID,
    mtu: usize,
    frame_maximum: usize,
    mac_address: [u8; ETHERNET_ADDRESS_LENGTH],
    pnp_device: WDFDEVICE,
    adapter: netadaptercx_sys::NETADAPTER,
    read_queue: WDFQUEUE,
    injection_lock: WDFSPINLOCK,
    capture_lock: WDFSPINLOCK,
    frame_pool: wdk_sys::WDFLOOKASIDE,
    state_lock: WDFSPINLOCK,
    injection_queue: Option<FrameQueue>,
    capture_queue: Option<FrameQueue>,
    active_packet_filters: netadaptercx_sys::_NET_PACKET_FILTER_FLAGS,
    active_multicast_address_count: usize,
    active_multicast_addresses: [[u8; ETHERNET_ADDRESS_LENGTH]; MAXIMUM_MULTICAST_ADDRESSES],
    read_work_item: WDFWORKITEM,
    legacy_direct_read_lock: WDFWAITLOCK,
    write_lifetime_lock: WDFWAITLOCK,
    tx_queue: netadaptercx_sys::NETPACKETQUEUE,
    rx_queue: netadaptercx_sys::NETPACKETQUEUE,
    tx_queue_started: AtomicBool,
    rx_queue_started: AtomicBool,
    rx_notification_armed: AtomicBool,
    pending_reads: AtomicUsize,
    legacy_direct_read_claims: AtomicUsize,
    pending_writes: AtomicUsize,
    control_open: AtomicBool,
    adaptive_enabled: AtomicBool,
    injection_generation: AtomicU64,
    capture_generation: AtomicU64,
    rx_callback_leases: AtomicU64,
    tx_callback_leases: AtomicU64,
    wait_state: AtomicU64,
    wait_request: AtomicPtr<c_void>,
    wait_cancel_handoff: AtomicU8,
    wait_interest: AtomicU32,
    wait_registration_capture_generation: AtomicU64,
    wait_registration_injection_generation: AtomicU64,
    owner_generation: AtomicU64,
    lifecycle: AtomicU8,
}

struct LegacyDirectReadGuard {
    state: *mut InstanceState,
}

impl LegacyDirectReadGuard {
    unsafe fn acquire(state: *mut InstanceState) -> Option<Self> {
        unsafe { Self::acquire_with_timeout(state, core::ptr::null_mut()) }
    }

    /// Nonblocking acquisition used by the TX packet-queue advance callback so
    /// the callback never waits on a lock that a teardown path can hold.
    unsafe fn try_acquire(state: *mut InstanceState) -> Option<Self> {
        let mut immediate: i64 = 0;
        unsafe { Self::acquire_with_timeout(state, &mut immediate) }
    }

    unsafe fn acquire_with_timeout(state: *mut InstanceState, timeout: *mut i64) -> Option<Self> {
        if state.is_null() || unsafe { (*state).legacy_direct_read_lock.is_null() } {
            return None;
        }
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockAcquire,
                (*state).legacy_direct_read_lock,
                timeout,
            )
        };
        if status == STATUS_SUCCESS {
            Some(Self { state })
        } else {
            None
        }
    }
}

impl Drop for LegacyDirectReadGuard {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockRelease,
                (*self.state).legacy_direct_read_lock
            );
        }
    }
}

/// Nonblocking callback-lifetime lease over a direction-specific datapath.
///
/// Admission closure and lease acquisition share one atomic word, so a lease
/// can never be granted after admission closes. Each quiesce scope owns a
/// distinct closer bit of that word, so nested scopes readmit independently and
/// one scope can never readmit callbacks for another. Teardown closes its
/// admission bit and then waits at `PASSIVE_LEVEL` for the outstanding lease
/// count to reach zero before clearing, reopening, or invalidating queue state.
/// Acquisition never blocks and is therefore safe from a DISPATCH_LEVEL packet
/// callback.
struct DatapathLease {
    counter: *const AtomicU64,
}

impl DatapathLease {
    fn acquire(counter: *const AtomicU64) -> Option<Self> {
        if counter.is_null() {
            return None;
        }
        let leases = unsafe { &*counter };
        let mut observed = leases.load(Ordering::Acquire);
        loop {
            if observed & DATAPATH_CLOSED_ANY != 0
                || observed & DATAPATH_LEASE_COUNT == DATAPATH_LEASE_COUNT
            {
                return None;
            }
            match leases.compare_exchange_weak(
                observed,
                observed + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self { counter }),
                Err(current) => observed = current,
            }
        }
    }
}

impl Drop for DatapathLease {
    fn drop(&mut self) {
        unsafe {
            (*self.counter).fetch_sub(1, Ordering::AcqRel);
        }
    }
}

fn acquire_receive_lease(state: *mut InstanceState) -> Option<DatapathLease> {
    if state.is_null() {
        return None;
    }
    DatapathLease::acquire(unsafe { core::ptr::addr_of!((*state).rx_callback_leases) })
}

fn acquire_capture_lease(state: *mut InstanceState) -> Option<DatapathLease> {
    if state.is_null() {
        return None;
    }
    DatapathLease::acquire(unsafe { core::ptr::addr_of!((*state).tx_callback_leases) })
}

fn drain_datapath_leases(leases: &AtomicU64) {
    let mut interval = wdk_sys::LARGE_INTEGER { QuadPart: -1_000 };
    while leases.load(Ordering::Acquire) & DATAPATH_LEASE_COUNT != 0 {
        if at_passive_level() {
            unsafe {
                // KernelMode, non-alertable, 100us relative delay.
                let _ = wdk_sys::ntddk::KeDelayExecutionThread(0, 0, &mut interval);
            }
        } else {
            unsafe {
                wdk_sys::ntddk::KeStallExecutionProcessor(10);
            }
        }
    }
}

/// Closes one scope's packet-callback admission for both directions and waits
/// for every outstanding lease to drain. `closer` identifies the quiesce scope;
/// admission stays closed until every scope that closed it has resumed.
/// Callers must hold no lock that a leased callback can wait on.
fn quiesce_datapath_callbacks(state: *mut InstanceState, closer: u64) {
    if state.is_null() {
        return;
    }
    let (receive_leases, capture_leases) =
        unsafe { (&(*state).rx_callback_leases, &(*state).tx_callback_leases) };
    receive_leases.fetch_or(closer, Ordering::AcqRel);
    capture_leases.fetch_or(closer, Ordering::AcqRel);
    drain_datapath_leases(receive_leases);
    drain_datapath_leases(capture_leases);
}

/// Releases one scope's admission closer after queue state has been
/// republished. Callbacks are readmitted only once no other scope is quiesced.
fn resume_datapath_callbacks(state: *mut InstanceState, closer: u64) {
    if state.is_null() {
        return;
    }
    unsafe {
        (*state)
            .rx_callback_leases
            .fetch_and(!closer, Ordering::AcqRel);
        (*state)
            .tx_callback_leases
            .fetch_and(!closer, Ordering::AcqRel);
    }
}

/// Scope guard that keeps a quiesce closer set until the scope ends, so an
/// early return can never strand packet-callback admission closed.
struct DatapathQuiesceGuard {
    state: *mut InstanceState,
    closer: u64,
}

impl DatapathQuiesceGuard {
    fn acquire(state: *mut InstanceState, closer: u64) -> Self {
        quiesce_datapath_callbacks(state, closer);
        Self { state, closer }
    }
}

impl Drop for DatapathQuiesceGuard {
    fn drop(&mut self) {
        resume_datapath_callbacks(self.state, self.closer);
    }
}

/// Reads the packed adaptive-wait record word.
fn load_wait_record(state: *mut InstanceState) -> u64 {
    if state.is_null() {
        return wait_record_word(0, 0, WAIT_FREE);
    }
    unsafe { (*state).wait_state.load(Ordering::SeqCst) }
}

fn wait_record_state(word: u64) -> u8 {
    (word & WAIT_RECORD_STATE_MASK) as u8
}

fn wait_record_satisfied(word: u64) -> u32 {
    ((word & WAIT_RECORD_SATISFIED_MASK) >> WAIT_RECORD_SATISFIED_SHIFT) as u32
}

fn wait_record_sequence(word: u64) -> u64 {
    word >> WAIT_RECORD_SEQUENCE_SHIFT
}

fn wait_record_word(sequence: u64, satisfied: u32, state: u8) -> u64 {
    (sequence << WAIT_RECORD_SEQUENCE_SHIFT)
        | ((u64::from(satisfied) << WAIT_RECORD_SATISFIED_SHIFT) & WAIT_RECORD_SATISFIED_MASK)
        | u64::from(state)
}

/// Moves the wait record from `from` to `to` while preserving its registration
/// sequence and satisfied mask. Returns the published word on success and the
/// observed word when the record is no longer in `from`.
fn transition_wait_record(state: *mut InstanceState, from: u8, to: u8) -> Result<u64, u64> {
    if state.is_null() {
        return Err(wait_record_word(0, 0, WAIT_FREE));
    }
    let record = unsafe { &(*state).wait_state };
    loop {
        let observed = record.load(Ordering::SeqCst);
        if wait_record_state(observed) != from {
            return Err(observed);
        }
        let next = wait_record_word(
            wait_record_sequence(observed),
            wait_record_satisfied(observed),
            to,
        );
        if record
            .compare_exchange_weak(observed, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(next);
        }
    }
}

/// Publishes a fresh `WAIT_REGISTERING` record.
///
/// The registration sequence advances and the satisfied mask is cleared in the
/// same store, so a queue transition that observed the previous record can
/// neither contribute readiness bits to this one nor schedule it.
fn begin_wait_record(state: *mut InstanceState) {
    if state.is_null() {
        return;
    }
    let record = unsafe { &(*state).wait_state };
    let observed = record.load(Ordering::SeqCst);
    record.store(
        wait_record_word(wait_record_sequence(observed) + 1, 0, WAIT_REGISTERING),
        Ordering::SeqCst,
    );
}

impl InstanceState {
    fn new(guid: GUID, mtu: usize) -> Self {
        Self {
            guid,
            mtu,
            frame_maximum: mtu + FRAME_MINIMUM,
            mac_address: mac_address_from_guid(&guid),
            pnp_device: core::ptr::null_mut(),
            adapter: core::ptr::null_mut(),
            read_queue: core::ptr::null_mut(),
            injection_lock: core::ptr::null_mut(),
            capture_lock: core::ptr::null_mut(),
            frame_pool: core::ptr::null_mut(),
            state_lock: core::ptr::null_mut(),
            injection_queue: None,
            capture_queue: None,
            active_packet_filters: 0,
            active_multicast_address_count: 0,
            active_multicast_addresses: [[0; ETHERNET_ADDRESS_LENGTH]; MAXIMUM_MULTICAST_ADDRESSES],
            read_work_item: core::ptr::null_mut(),
            legacy_direct_read_lock: core::ptr::null_mut(),
            write_lifetime_lock: core::ptr::null_mut(),
            tx_queue: core::ptr::null_mut(),
            rx_queue: core::ptr::null_mut(),
            tx_queue_started: AtomicBool::new(false),
            rx_queue_started: AtomicBool::new(false),
            rx_notification_armed: AtomicBool::new(false),
            pending_reads: AtomicUsize::new(0),
            legacy_direct_read_claims: AtomicUsize::new(0),
            pending_writes: AtomicUsize::new(0),
            control_open: AtomicBool::new(false),
            adaptive_enabled: AtomicBool::new(false),
            injection_generation: AtomicU64::new(0),
            capture_generation: AtomicU64::new(0),
            rx_callback_leases: AtomicU64::new(DATAPATH_CLOSED_HARDWARE),
            tx_callback_leases: AtomicU64::new(DATAPATH_CLOSED_HARDWARE),
            wait_state: AtomicU64::new(wait_record_word(0, 0, WAIT_FREE)),
            wait_request: AtomicPtr::new(core::ptr::null_mut()),
            wait_cancel_handoff: AtomicU8::new(WAIT_HANDOFF_NONE),
            wait_interest: AtomicU32::new(0),
            wait_registration_capture_generation: AtomicU64::new(0),
            wait_registration_injection_generation: AtomicU64::new(0),
            owner_generation: AtomicU64::new(0),
            lifecycle: AtomicU8::new(INSTANCE_OPEN),
        }
    }
}

fn allocate_instance_state(guid: GUID, mtu: usize) -> *mut InstanceState {
    let layout = Layout::new::<InstanceState>();
    let state = unsafe {
        // SAFETY: The layout exactly describes the InstanceState allocation.
        alloc(layout).cast::<InstanceState>()
    };
    if !state.is_null() {
        unsafe {
            // SAFETY: The allocation is uniquely owned and properly aligned for InstanceState.
            state.write(InstanceState::new(guid, mtu));
        }
    }
    state
}

struct InstanceStateGuard {
    state: *mut InstanceState,
    lock: WDFSPINLOCK,
}

impl InstanceStateGuard {
    unsafe fn new(state: *mut InstanceState) -> Option<Self> {
        if state.is_null() {
            return None;
        }
        let lock = unsafe { (*state).state_lock };
        if lock.is_null() {
            return None;
        }
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        }
        Some(Self { state, lock })
    }
}

impl Deref for InstanceStateGuard {
    type Target = InstanceState;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.state }
    }
}

impl DerefMut for InstanceStateGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.state }
    }
}

impl Drop for InstanceStateGuard {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, self.lock);
        }
    }
}

static mut QUEUE_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: QUEUE_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<QueueContext>(),
        UniqueType: &raw const QUEUE_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

static mut DEVICE_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: DEVICE_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<DeviceContext>(),
        UniqueType: &raw const DEVICE_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

static mut ADAPTER_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: ADAPTER_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<AdapterContext>(),
        UniqueType: &raw const ADAPTER_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

static mut WORK_ITEM_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: WORK_ITEM_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<WorkItemContext>(),
        UniqueType: &raw const WORK_ITEM_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

#[repr(C)]
struct QueueContext {
    is_transmit: bool,
    _padding: [u8; 7],
    instance: *mut InstanceState,
    rings: netadaptercx_sys::NET_RING_COLLECTION,
    fragment_extension: netadaptercx_sys::NET_EXTENSION,
    frame_maximum: usize,
    frame_pool: wdk_sys::WDFLOOKASIDE,
}

#[repr(C)]
struct DeviceContext {
    instance: *mut InstanceState,
}

#[repr(C)]
struct AdapterContext {
    instance: *mut InstanceState,
}

#[repr(C)]
struct WorkItemContext {
    instance: *mut InstanceState,
}

const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_LINK_STATE>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_PACKET_QUEUE_CONFIG>();
const _: () = {
    assert!(
        core::mem::offset_of!(
            netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES,
            MappingRequirement
        ) == 4
    );
    assert!(
        core::mem::offset_of!(
            netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES,
            FragmentBufferAlignment
        ) == 24
    );
    assert!(
        core::mem::offset_of!(
            netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES,
            DmaCapabilities
        ) == 48
    );
};

unsafe fn object_context<T>(
    object: WDFOBJECT,
    type_info: *const wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO,
) -> *mut T {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfObjectGetTypedContextWorker, object, type_info).cast()
    }
}

unsafe fn instance_from_device(device: WDFDEVICE) -> Option<*mut InstanceState> {
    let context = unsafe {
        object_context::<DeviceContext>(device.cast(), &raw const DEVICE_CONTEXT_TYPE_INFO)
    };
    if context.is_null() || unsafe { (*context).instance.is_null() } {
        None
    } else {
        Some(unsafe { (*context).instance })
    }
}

unsafe fn instance_from_pnp_device(device: WDFDEVICE) -> Option<*mut InstanceState> {
    unsafe { instance_from_device(device) }
}

unsafe fn instance_from_adapter(
    adapter: netadaptercx_sys::NETADAPTER,
) -> Option<*mut InstanceState> {
    let context = unsafe {
        object_context::<AdapterContext>(adapter.cast(), &raw const ADAPTER_CONTEXT_TYPE_INFO)
    };
    if context.is_null() || unsafe { (*context).instance.is_null() } {
        None
    } else {
        Some(unsafe { (*context).instance })
    }
}

unsafe fn instance_from_io_queue(queue: WDFQUEUE) -> Option<*mut InstanceState> {
    let device = unsafe { call_unsafe_wdf_function_binding!(WdfIoQueueGetDevice, queue) };
    unsafe { instance_from_device(device) }
}

unsafe fn instance_from_work_item(work_item: WDFWORKITEM) -> Option<*mut InstanceState> {
    let context = unsafe {
        object_context::<WorkItemContext>(work_item.cast(), &raw const WORK_ITEM_CONTEXT_TYPE_INFO)
    };
    if context.is_null() || unsafe { (*context).instance.is_null() } {
        None
    } else {
        Some(unsafe { (*context).instance })
    }
}

unsafe fn packet_queue_context(queue: netadaptercx_sys::NETPACKETQUEUE) -> *mut QueueContext {
    unsafe { object_context::<QueueContext>(queue.cast(), &raw const QUEUE_CONTEXT_TYPE_INFO) }
}

/// Required WDF driver entry point.
///
/// IRQL: PASSIVE_LEVEL. This scaffold may not unwind; Cargo profiles use
/// `panic = "abort"` and `wdk-panic` supplies the kernel panic handler.
#[unsafe(export_name = "DriverEntry")]
pub unsafe extern "system" fn driver_entry(
    driver: &mut DRIVER_OBJECT,
    registry_path: PCUNICODE_STRING,
) -> NTSTATUS {
    debug_marker(b"DriverEntry enter");
    let mut driver_config = {
        let config_size = core::mem::size_of::<WDF_DRIVER_CONFIG>();
        const { assert!(core::mem::size_of::<WDF_DRIVER_CONFIG>() <= ULONG::MAX as usize) };
        WDF_DRIVER_CONFIG {
            Size: config_size as ULONG,
            EvtDriverDeviceAdd: Some(evt_driver_device_add),
            ..WDF_DRIVER_CONFIG::default()
        }
    };

    // SAFETY: DriverEntry receives valid WDF-owned driver and registry path
    // pointers, the object attributes output is intentionally null, and the
    // driver config lives until WdfDriverCreate returns.
    let mut wdf_driver: WDFDRIVER = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDriverCreate,
            driver as PDRIVER_OBJECT,
            registry_path,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut driver_config,
            &mut wdf_driver,
        )
    };
    debug_status(b"WdfDriverCreate", status);

    if status != STATUS_SUCCESS {
        status
    } else {
        STATUS_SUCCESS
    }
}

/// Creates the administrator-only control device used by the TAP data path.
///
/// IRQL: PASSIVE_LEVEL. Queues and file callbacks are added in the subsequent
/// implementation slice; the device is deliberately not exposed as a
/// functional data path until those callbacks are complete.
extern "C" fn evt_driver_device_add(
    _driver: WDFDRIVER,
    device_init: *mut WDFDEVICE_INIT,
) -> NTSTATUS {
    debug_marker(b"EvtDriverDeviceAdd enter");
    let status = unsafe {
        // SAFETY: NetDeviceInitConfig is called once at PASSIVE_LEVEL before
        // the WDF device is created.
        net_call_device_init_config(device_init)
    };
    debug_status(b"NetDeviceInitConfig", status);
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut pnp_callbacks = WDF_PNPPOWER_EVENT_CALLBACKS {
        Size: core::mem::size_of::<WDF_PNPPOWER_EVENT_CALLBACKS>() as ULONG,
        EvtDeviceD0Entry: Some(evt_device_d0_entry),
        EvtDeviceD0Exit: Some(evt_device_d0_exit),
        EvtDevicePrepareHardware: Some(evt_device_prepare_hardware),
        EvtDeviceReleaseHardware: Some(evt_device_release_hardware),
        ..WDF_PNPPOWER_EVENT_CALLBACKS::default()
    };
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitSetPnpPowerEventCallbacks,
            device_init,
            &mut pnp_callbacks,
        );
    }
    let mut file_config = WDF_FILEOBJECT_CONFIG {
        Size: core::mem::size_of::<WDF_FILEOBJECT_CONFIG>() as ULONG,
        EvtDeviceFileCreate: Some(evt_file_create),
        EvtFileClose: Some(evt_file_close),
        EvtFileCleanup: Some(evt_file_cleanup),
        FileObjectClass: wdk_sys::_WDF_FILEOBJECT_CLASS::WdfFileObjectWdfCannotUseFsContexts,
        AutoForwardCleanupClose: wdk_sys::_WDF_TRI_STATE::WdfUseDefault,
        ..WDF_FILEOBJECT_CONFIG::default()
    };
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitSetFileObjectConfig,
            device_init,
            &mut file_config,
            WDF_NO_OBJECT_ATTRIBUTES,
        );
    }
    let mut pnp_init = device_init;
    let mut _pnp_device: WDFDEVICE = core::ptr::null_mut();
    let mut device_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        EvtCleanupCallback: Some(evt_instance_context_destroy),
        // Mirror WDF_OBJECT_ATTRIBUTES_INIT; Rust's Default leaves these invalid.
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ContextTypeInfo: &raw const DEVICE_CONTEXT_TYPE_INFO,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreate,
            &mut pnp_init,
            &mut device_attributes,
            &mut _pnp_device,
        )
    };
    debug_status(b"WdfDeviceCreate", status);
    if status != STATUS_SUCCESS {
        return status;
    }
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreateDeviceInterface,
            _pnp_device,
            &TAP_INTERFACE_CLASS as *const GUID,
            core::ptr::null::<UNICODE_STRING>(),
        )
    };
    debug_status(b"WdfDeviceCreateDeviceInterface", status);
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, _pnp_device.cast());
        }
        return status;
    }
    let device_context = unsafe {
        object_context::<DeviceContext>(_pnp_device.cast(), &raw const DEVICE_CONTEXT_TYPE_INFO)
    };
    if device_context.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, _pnp_device.cast());
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    let (guid, mtu) = match child_properties_from_hardware_id(_pnp_device) {
        Ok(properties) => properties,
        Err(status) => {
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, _pnp_device.cast());
            }
            debug_status(b"ChildGuidFromHardwareId", status);
            return status;
        }
    };
    let state = allocate_instance_state(guid, mtu);
    if state.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, _pnp_device.cast());
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*device_context).instance = state;
        (*state).pnp_device = _pnp_device;
    }

    let status = create_tap_device(_pnp_device, unsafe { &mut *state });
    debug_status(b"CreateTapDevice", status);
    if status != STATUS_SUCCESS {
        return status;
    }

    // SAFETY: The WDF device has been created and this callback runs at
    // PASSIVE_LEVEL during device addition.
    let status = unsafe { create_adapter(_pnp_device, &mut *state) };
    debug_status(b"CreateAdapter", status);
    status
}

fn mac_address_from_guid(guid: &GUID) -> [u8; ETHERNET_ADDRESS_LENGTH] {
    [
        0x02,
        guid.Data4[0],
        guid.Data4[1],
        guid.Data4[2],
        guid.Data4[3],
        guid.Data4[4],
    ]
}

fn hex_nibble(character: u16) -> Option<u8> {
    if (b'0' as u16..=b'9' as u16).contains(&character) {
        Some((character - b'0' as u16) as u8)
    } else if (b'a' as u16..=b'f' as u16).contains(&character) {
        Some((character - b'a' as u16 + 10) as u8)
    } else if (b'A' as u16..=b'F' as u16).contains(&character) {
        Some((character - b'A' as u16 + 10) as u8)
    } else {
        None
    }
}

fn parse_child_guid_from_hardware_id(hardware_id: &[u16]) -> Option<GUID> {
    const PREFIX: [u16; 11] = [
        b'W' as u16,
        b'I' as u16,
        b'N' as u16,
        b'T' as u16,
        b'A' as u16,
        b'P' as u16,
        b'B' as u16,
        b'U' as u16,
        b'S' as u16,
        b'\\' as u16,
        b'{' as u16,
    ];
    if !hardware_id.starts_with(&PREFIX) {
        return None;
    }
    let mut digits = [0u8; 32];
    let mut digit_count = 0;
    for character in hardware_id[11..].iter() {
        if *character == b'}' as u16 {
            if digit_count != digits.len() {
                return None;
            }
            let byte = |index: usize| (digits[index] << 4) | digits[index + 1];
            return Some(GUID {
                Data1: ((byte(0) as u32) << 24)
                    | ((byte(2) as u32) << 16)
                    | ((byte(4) as u32) << 8)
                    | byte(6) as u32,
                Data2: ((byte(8) as u16) << 8) | byte(10) as u16,
                Data3: ((byte(12) as u16) << 8) | byte(14) as u16,
                Data4: [
                    byte(16),
                    byte(18),
                    byte(20),
                    byte(22),
                    byte(24),
                    byte(26),
                    byte(28),
                    byte(30),
                ],
            });
        }
        if *character == b'-' as u16 {
            continue;
        }
        let Some(value) = hex_nibble(*character) else {
            return None;
        };
        if digit_count == digits.len() {
            return None;
        }
        digits[digit_count] = value;
        digit_count += 1;
    }
    None
}

fn parse_child_mtu_from_hardware_id(hardware_id: &[u16]) -> Option<usize> {
    const PREFIX: [u16; 14] = [
        b'W' as u16,
        b'I' as u16,
        b'N' as u16,
        b'T' as u16,
        b'A' as u16,
        b'P' as u16,
        b'B' as u16,
        b'U' as u16,
        b'S' as u16,
        b'M' as u16,
        b'T' as u16,
        b'U' as u16,
        b'\\' as u16,
        0,
    ];
    if !hardware_id.starts_with(&PREFIX[..PREFIX.len() - 1]) {
        return None;
    }
    let mut value = 0usize;
    for character in &hardware_id[PREFIX.len() - 1..] {
        if *character == 0 {
            break;
        }
        if !(b'0' as u16..=b'9' as u16).contains(character) {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add((*character - b'0' as u16) as usize)?;
    }
    if (DEFAULT_MTU..=MAXIMUM_MTU).contains(&value) {
        Some(value)
    } else {
        None
    }
}

fn child_properties_from_hardware_id(device: WDFDEVICE) -> Result<(GUID, usize), NTSTATUS> {
    let mut hardware_ids = [0u16; 128];
    let mut result_length: ULONG = 0;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceQueryProperty,
            device,
            wdk_sys::DEVICE_REGISTRY_PROPERTY::DevicePropertyHardwareID,
            (hardware_ids.len() * core::mem::size_of::<u16>()) as ULONG,
            hardware_ids.as_mut_ptr().cast::<c_void>(),
            &mut result_length,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(status);
    }
    if result_length as usize % core::mem::size_of::<u16>() != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let character_count =
        (result_length as usize / core::mem::size_of::<u16>()).min(hardware_ids.len());
    let mut guid = None;
    let mut mtu = None;
    let mut offset = 0;
    while offset < character_count {
        let Some(length) = hardware_ids[offset..character_count]
            .iter()
            .position(|character| *character == 0)
        else {
            return Err(STATUS_INVALID_PARAMETER);
        };
        if length == 0 {
            break;
        }
        if let Some(parsed_guid) =
            parse_child_guid_from_hardware_id(&hardware_ids[offset..offset + length])
        {
            guid = Some(parsed_guid);
        }
        let hardware_id = &hardware_ids[offset..offset + length];
        if hardware_id.starts_with(&[
            b'W' as u16,
            b'I' as u16,
            b'N' as u16,
            b'T' as u16,
            b'A' as u16,
            b'P' as u16,
            b'B' as u16,
            b'U' as u16,
            b'S' as u16,
            b'M' as u16,
            b'T' as u16,
            b'U' as u16,
            b'\\' as u16,
        ]) {
            let Some(value) = parse_child_mtu_from_hardware_id(hardware_id) else {
                return Err(STATUS_INVALID_PARAMETER);
            };
            mtu = Some(value);
        }
        offset += length + 1;
    }
    match (guid, mtu) {
        (Some(guid), Some(mtu)) => Ok((guid, mtu)),
        (Some(guid), None) => Ok((guid, DEFAULT_MTU)),
        _ => Err(STATUS_INVALID_PARAMETER),
    }
}

unsafe fn net_function<T: Copy>(index: usize) -> T {
    // NetAdapterCx exposes its ABI as a versioned function table. The
    // generated bindings provide the table symbol and exact handle types.
    let table =
        core::ptr::addr_of!(netadaptercx_sys::NetFunctions) as *const netadaptercx_sys::NETFUNC;
    let entry = unsafe { table.add(index).read() };
    unsafe { core::mem::transmute_copy(&entry) }
}

unsafe fn net_call_device_init_config(device_init: *mut WDFDEVICE_INIT) -> NTSTATUS {
    let function: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        *mut WDFDEVICE_INIT,
    ) -> NTSTATUS = unsafe {
        net_function(netadaptercx_sys::_NETFUNCENUM_NetDeviceInitConfigTableIndex as usize)
    };
    // SAFETY: NetDriverGlobals and DeviceInit are supplied by NetAdapterCx/WDF.
    unsafe { function(netadaptercx_sys::NetDriverGlobals, device_init) }
}

unsafe fn create_adapter(device: WDFDEVICE, state: &mut InstanceState) -> NTSTATUS {
    let allocate: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        WDFDEVICE,
    ) -> *mut netadaptercx_sys::NETADAPTER_INIT = unsafe {
        net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterInitAllocateTableIndex as usize)
    };
    let adapter_init = unsafe { allocate(netadaptercx_sys::NetDriverGlobals, device) };
    if adapter_init.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    let mut callbacks = netadaptercx_sys::NET_ADAPTER_DATAPATH_CALLBACKS {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_DATAPATH_CALLBACKS>() as ULONG,
        EvtAdapterCreateTxQueue: Some(evt_create_tx_queue),
        EvtAdapterCreateRxQueue: Some(evt_create_rx_queue),
    };
    let set_callbacks: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        *mut netadaptercx_sys::NETADAPTER_INIT,
        *mut netadaptercx_sys::NET_ADAPTER_DATAPATH_CALLBACKS,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterInitSetDatapathCallbacksTableIndex as usize,
        )
    };
    // SAFETY: AdapterInit and callback storage remain valid for this call.
    unsafe {
        set_callbacks(
            netadaptercx_sys::NetDriverGlobals,
            adapter_init,
            &mut callbacks,
        );
    }

    let mut adapter_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ContextTypeInfo: &raw const ADAPTER_CONTEXT_TYPE_INFO,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let create: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        *mut netadaptercx_sys::NETADAPTER_INIT,
        *mut wdk_sys::WDF_OBJECT_ATTRIBUTES,
        *mut netadaptercx_sys::NETADAPTER,
    ) -> NTSTATUS =
        unsafe { net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterCreateTableIndex as usize) };
    let mut adapter = core::ptr::null_mut();
    // SAFETY: The adapter-init object is valid for one NetAdapterCreate call.
    let status = unsafe {
        create(
            netadaptercx_sys::NetDriverGlobals,
            adapter_init,
            &mut adapter_attributes,
            &mut adapter,
        )
    };
    debug_status(b"NetAdapterCreate", status);
    let free: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        *mut netadaptercx_sys::NETADAPTER_INIT,
    ) = unsafe {
        net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterInitFreeTableIndex as usize)
    };
    // SAFETY: NetAdapterInitFree releases every successful allocation after its
    // NetAdapterCreate attempt, including a successful creation.
    unsafe { free(netadaptercx_sys::NetDriverGlobals, adapter_init) };
    if status != STATUS_SUCCESS {
        return status;
    }

    state.adapter = adapter;
    let adapter_context = unsafe {
        object_context::<AdapterContext>(adapter.cast(), &raw const ADAPTER_CONTEXT_TYPE_INFO)
    };
    if adapter_context.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*adapter_context).instance = state;
    }
    STATUS_SUCCESS
}

fn configure_adapter_link_state(
    adapter: netadaptercx_sys::NETADAPTER,
    mac_address: [u8; ETHERNET_ADDRESS_LENGTH],
    mtu: usize,
) {
    let mut link_layer = netadaptercx_sys::NET_ADAPTER_LINK_LAYER_CAPABILITIES {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_LINK_LAYER_CAPABILITIES>()
            as ULONG,
        MaxTxLinkSpeed: 1_000_000_000,
        MaxRxLinkSpeed: 1_000_000_000,
    };
    let set_link_layer: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *mut netadaptercx_sys::NET_ADAPTER_LINK_LAYER_CAPABILITIES,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetLinkLayerCapabilitiesTableIndex as usize,
        )
    };
    unsafe {
        set_link_layer(netadaptercx_sys::NetDriverGlobals, adapter, &mut link_layer);
    }

    let set_mtu: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        ULONG,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetLinkLayerMtuSizeTableIndex as usize,
        )
    };
    unsafe {
        set_mtu(netadaptercx_sys::NetDriverGlobals, adapter, mtu as ULONG);
    }

    let address = netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS {
        Length: 6,
        Address: [
            mac_address[0],
            mac_address[1],
            mac_address[2],
            mac_address[3],
            mac_address[4],
            mac_address[5],
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ],
    };
    let set_permanent: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *const netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetPermanentLinkLayerAddressTableIndex
                as usize,
        )
    };
    let set_current: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *const netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetCurrentLinkLayerAddressTableIndex as usize,
        )
    };
    unsafe {
        set_permanent(netadaptercx_sys::NetDriverGlobals, adapter, &address);
        set_current(netadaptercx_sys::NetDriverGlobals, adapter, &address);
    }

    let mut link_state = netadaptercx_sys::NET_ADAPTER_LINK_STATE {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_LINK_STATE>() as ULONG,
        TxLinkSpeed: 1_000_000_000,
        RxLinkSpeed: 1_000_000_000,
        MediaConnectState:
            netadaptercx_sys::_NET_IF_MEDIA_CONNECT_STATE_MediaConnectStateConnected,
        MediaDuplexState: netadaptercx_sys::_NET_IF_MEDIA_DUPLEX_STATE_MediaDuplexStateFull,
        SupportedPauseFunctions:
            netadaptercx_sys::_NET_ADAPTER_PAUSE_FUNCTION_TYPE_NetAdapterPauseFunctionTypeUnsupported,
        AutoNegotiationFlags:
            netadaptercx_sys::_NET_ADAPTER_AUTO_NEGOTIATION_FLAGS_NetAdapterAutoNegotiationFlagNone,
    };
    let set_link_state: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *mut netadaptercx_sys::NET_ADAPTER_LINK_STATE,
    ) = unsafe {
        net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterSetLinkStateTableIndex as usize)
    };
    unsafe {
        set_link_state(netadaptercx_sys::NetDriverGlobals, adapter, &mut link_state);
    }
}

extern "C" fn evt_create_tx_queue(
    adapter: netadaptercx_sys::NETADAPTER,
    queue_init: *mut netadaptercx_sys::NETTXQUEUE_INIT,
) -> NTSTATUS {
    create_packet_queue(adapter, queue_init.cast(), true)
}

extern "C" fn evt_create_rx_queue(
    adapter: netadaptercx_sys::NETADAPTER,
    queue_init: *mut netadaptercx_sys::NETRXQUEUE_INIT,
) -> NTSTATUS {
    create_packet_queue(adapter, queue_init.cast(), false)
}

fn create_packet_queue(
    adapter: netadaptercx_sys::NETADAPTER,
    queue_init: *mut c_void,
    is_transmit: bool,
) -> NTSTATUS {
    debug_marker(if is_transmit {
        b"NetTxQueueCreate enter"
    } else {
        b"NetRxQueueCreate enter"
    });
    let mut config = netadaptercx_sys::NET_PACKET_QUEUE_CONFIG {
        Size: core::mem::size_of::<netadaptercx_sys::NET_PACKET_QUEUE_CONFIG>() as ULONG,
        EvtStart: Some(evt_packet_queue_start),
        EvtStop: Some(evt_packet_queue_stop),
        EvtAdvance: Some(evt_packet_queue_advance),
        EvtSetNotificationEnabled: Some(evt_packet_queue_set_notification_enabled),
        EvtCancel: Some(evt_packet_queue_cancel),
        ..netadaptercx_sys::NET_PACKET_QUEUE_CONFIG::default()
    };
    let mut packet_queue = core::ptr::null_mut();
    let mut attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ContextTypeInfo: &raw const QUEUE_CONTEXT_TYPE_INFO,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope: wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeNone,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let status = unsafe {
        let function_index = if is_transmit {
            netadaptercx_sys::_NETFUNCENUM_NetTxQueueCreateTableIndex
        } else {
            netadaptercx_sys::_NETFUNCENUM_NetRxQueueCreateTableIndex
        };
        let create: unsafe extern "system" fn(
            netadaptercx_sys::PNET_DRIVER_GLOBALS,
            *mut c_void,
            *mut WDF_OBJECT_ATTRIBUTES,
            *mut netadaptercx_sys::NET_PACKET_QUEUE_CONFIG,
            *mut netadaptercx_sys::NETPACKETQUEUE,
        ) -> NTSTATUS = net_function(function_index as usize);
        create(
            netadaptercx_sys::NetDriverGlobals,
            queue_init,
            &mut attributes,
            &mut config,
            &mut packet_queue,
        )
    };
    debug_status(
        if is_transmit {
            b"NetTxQueueCreate"
        } else {
            b"NetRxQueueCreate"
        },
        status,
    );
    if status == STATUS_SUCCESS {
        let state = match unsafe { instance_from_adapter(adapter) } {
            Some(state) => state,
            None => return STATUS_DEVICE_NOT_READY,
        };
        let queue_context = unsafe {
            object_context::<QueueContext>(packet_queue.cast(), &raw const QUEUE_CONTEXT_TYPE_INFO)
        };
        if queue_context.is_null() {
            return STATUS_INSUFFICIENT_RESOURCES;
        }
        let get_rings: unsafe extern "system" fn(
            netadaptercx_sys::PNET_DRIVER_GLOBALS,
            netadaptercx_sys::NETPACKETQUEUE,
        )
            -> *const netadaptercx_sys::NET_RING_COLLECTION = unsafe {
            net_function(if is_transmit {
                netadaptercx_sys::_NETFUNCENUM_NetTxQueueGetRingCollectionTableIndex
            } else {
                netadaptercx_sys::_NETFUNCENUM_NetRxQueueGetRingCollectionTableIndex
            } as usize)
        };
        let rings = unsafe { get_rings(netadaptercx_sys::NetDriverGlobals, packet_queue) };
        let get_extension: unsafe extern "system" fn(
            netadaptercx_sys::PNET_DRIVER_GLOBALS,
            netadaptercx_sys::NETPACKETQUEUE,
            *const netadaptercx_sys::NET_EXTENSION_QUERY,
            *mut netadaptercx_sys::NET_EXTENSION,
        ) = unsafe {
            net_function(if is_transmit {
                netadaptercx_sys::_NETFUNCENUM_NetTxQueueGetExtensionTableIndex
            } else {
                netadaptercx_sys::_NETFUNCENUM_NetRxQueueGetExtensionTableIndex
            } as usize)
        };
        let query = netadaptercx_sys::NET_EXTENSION_QUERY {
            Size: core::mem::size_of::<netadaptercx_sys::NET_EXTENSION_QUERY>() as ULONG,
            Name: FRAGMENT_VIRTUAL_ADDRESS_NAME.as_ptr(),
            Version: 1,
            Type: netadaptercx_sys::_NET_EXTENSION_TYPE_NetExtensionTypeFragment,
        };
        let mut extension = netadaptercx_sys::NET_EXTENSION::default();
        unsafe {
            get_extension(
                netadaptercx_sys::NetDriverGlobals,
                packet_queue,
                &query,
                &mut extension,
            );
        }
        if rings.is_null() {
            return STATUS_DEVICE_NOT_READY;
        }
        // Queue-ring discovery is a PASSIVE_LEVEL NetAdapterCx operation.
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let state_ptr = state_guard.state;
        let state = &mut *state_guard;
        let frame_maximum = state.frame_maximum;
        let frame_pool = state.frame_pool;
        if is_transmit {
            state.tx_queue = packet_queue;
        } else {
            state.rx_queue = packet_queue;
        }
        drop(state_guard);
        unsafe {
            // NetAdapterCx cannot invoke packet callbacks until creation
            // returns, so these values remain immutable until queue stop.
            (*queue_context).is_transmit = is_transmit;
            (*queue_context).rings = *rings;
            (*queue_context).fragment_extension = extension;
            (*queue_context).frame_maximum = frame_maximum;
            (*queue_context).frame_pool = frame_pool;
            (*queue_context).instance = state_ptr;
        }
    }
    status
}

extern "C" fn evt_packet_queue_start(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let context = unsafe { packet_queue_context(queue) };
    if context.is_null() {
        return;
    }
    let state = unsafe { (*context).instance };
    if state.is_null() {
        return;
    }
    if unsafe { (*context).is_transmit } {
        unsafe { (*state).tx_queue_started.store(true, Ordering::Release) };
    } else {
        unsafe { (*state).rx_queue_started.store(true, Ordering::Release) };
    }
}

extern "C" fn evt_packet_queue_stop(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let context = unsafe { packet_queue_context(queue) };
    if context.is_null() {
        return;
    }
    let state = unsafe { (*context).instance };
    if state.is_null() {
        return;
    }
    if unsafe { (*context).is_transmit } {
        unsafe { (*state).tx_queue_started.store(false, Ordering::Release) };
    } else {
        unsafe {
            (*state).rx_queue_started.store(false, Ordering::Release);
            (*state)
                .rx_notification_armed
                .store(false, Ordering::Release);
        }
    }
    cancel_wait_for_teardown(state);
    flush_work_item(unsafe { (*state).read_work_item });
}

extern "C" fn evt_packet_queue_advance(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let context = unsafe { packet_queue_context(queue) };
    if context.is_null() {
        return;
    }
    let state = unsafe { (*context).instance };
    if state.is_null() {
        return;
    }
    let rings = unsafe { core::ptr::addr_of!((*context).rings) };
    let extension = unsafe { &(*context).fragment_extension };
    if unsafe { (*context).is_transmit } {
        // The lease is nonblocking and direction-specific: it never waits on
        // the opposite direction, and owner cleanup cannot clear or reopen the
        // capture queue while it is held.
        let Some(_capture_lease) = acquire_capture_lease(state) else {
            return;
        };
        capture_transmit_packets(
            state,
            unsafe { (*state).read_queue },
            unsafe { (*state).read_work_item },
            rings,
            extension,
            unsafe { (*context).frame_maximum },
            unsafe { (*context).frame_pool },
        );
    } else {
        let Some(_receive_lease) = acquire_receive_lease(state) else {
            return;
        };
        inject_receive_frames(state, rings, extension);
    }
}

fn inject_receive_frames(
    state: *mut InstanceState,
    rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    extension: &netadaptercx_sys::NET_EXTENSION,
) {
    let (packet_ring, fragment_ring) = unsafe {
        let collection = match rings.as_ref() {
            Some(collection) => collection,
            None => return,
        };
        (
            collection.Rings[ring::PACKET_RING_INDEX],
            collection.Rings[ring::FRAGMENT_RING_INDEX],
        )
    };
    if packet_ring.is_null() || fragment_ring.is_null() {
        return;
    }
    // The caller holds a receive lease, so the owner observed here cannot be
    // replaced until this callback returns.
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN {
        return;
    }
    let owner_generation = unsafe { (*state).owner_generation.load(Ordering::Acquire) };

    loop {
        let (packet_begin, packet_end, fragment_begin, fragment_end) = unsafe {
            (
                (*packet_ring).BeginIndex,
                (*packet_ring).EndIndex,
                (*fragment_ring).BeginIndex,
                (*fragment_ring).EndIndex,
            )
        };
        if packet_begin == packet_end || fragment_begin == fragment_end {
            break;
        }

        let (frame, injection_was_full) = dequeue_injection_frame(state);
        let frame = match frame {
            Some(frame) => frame,
            None => break,
        };
        let packet = match unsafe { packet_at(packet_ring, packet_begin) } {
            Some(packet) => unsafe { &mut *packet },
            None => {
                requeue_injection_frame(state, frame, owner_generation);
                break;
            }
        };
        let fragment = match unsafe { fragment_at(fragment_ring, fragment_begin) } {
            Some(fragment) => unsafe { &mut *fragment },
            None => {
                requeue_injection_frame(state, frame, owner_generation);
                break;
            }
        };
        let address = match unsafe { fragment_virtual_address(extension, fragment_begin) } {
            Some(address) => unsafe { &*address },
            None => {
                requeue_injection_frame(state, frame, owner_generation);
                break;
            }
        };
        if address.VirtualAddress.is_null() {
            requeue_injection_frame(state, frame, owner_generation);
            break;
        }

        // RX descriptors are reused by NetAdapterCx. Do not inherit a prior
        // frame's byte offset or valid length when indicating this frame.
        fragment.set_Offset(0);
        let frame_length = frame.as_bytes().len();
        let capacity = fragment.Capacity() as usize;
        if frame_length > capacity {
            requeue_injection_frame(state, frame, owner_generation);
            break;
        }

        unsafe {
            core::ptr::copy_nonoverlapping(
                frame.as_bytes().as_ptr(),
                address.VirtualAddress as *mut u8,
                frame_length,
            );
        }
        fragment.set_ValidLength(frame_length as u64);
        packet.set_Ignore(0);
        packet.FragmentIndex = fragment_begin;
        packet.FragmentCount = 1;
        packet.Layout = netadaptercx_sys::NET_PACKET_LAYOUT::default();
        packet.Layout.set_Layer2Type(
            netadaptercx_sys::_NET_PACKET_LAYER2_TYPE_NetPacketLayer2TypeEthernet as u8,
        );
        packet.Layout.set_Layer2HeaderLength(FRAME_MINIMUM as u16);

        let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
            Some(index) => index,
            None => break,
        };
        let next_fragment = match unsafe { increment_index(&*fragment_ring, fragment_begin) } {
            Some(index) => index,
            None => break,
        };
        unsafe {
            (*packet_ring).BeginIndex = next_packet;
            (*fragment_ring).BeginIndex = next_fragment;
        }
        if injection_was_full {
            let _ = claim_wait_for_passive_completion(
                state,
                ADAPTIVE_INTEREST_WRITABLE,
                core::ptr::null_mut(),
            );
        }
    }
}

fn capture_transmit_packets(
    state: *mut InstanceState,
    read_queue: WDFQUEUE,
    read_work_item: WDFWORKITEM,
    rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    extension: &netadaptercx_sys::NET_EXTENSION,
    frame_maximum: usize,
    frame_pool: wdk_sys::WDFLOOKASIDE,
) {
    let (packet_ring, fragment_ring) = unsafe {
        let collection = match rings.as_ref() {
            Some(collection) => collection,
            None => return,
        };
        (
            collection.Rings[ring::PACKET_RING_INDEX],
            collection.Rings[ring::FRAGMENT_RING_INDEX],
        )
    };
    if packet_ring.is_null() || fragment_ring.is_null() {
        return;
    }
    // The caller holds a capture lease, so owner cleanup cannot clear or
    // reopen the capture queue between this ownership snapshot and the
    // enqueue below.
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN {
        return;
    }
    let capture_generation = unsafe { (*state).owner_generation.load(Ordering::Acquire) };

    let mut captured_for_legacy_read = false;
    loop {
        let (packet_begin, packet_end, fragment_end) = unsafe {
            (
                (*packet_ring).BeginIndex,
                (*packet_ring).EndIndex,
                (*fragment_ring).EndIndex,
            )
        };
        if packet_begin == packet_end {
            break;
        }

        let packet = match unsafe { packet_at(packet_ring, packet_begin) } {
            Some(packet) => unsafe { &mut *packet },
            None => break,
        };
        let fragment_count = packet.FragmentCount as u32;
        if fragment_count == 0 {
            unsafe {
                (*packet).set_Ignore(1);
                if let Some(next_packet) = increment_index(&*packet_ring, packet_begin) {
                    (*packet_ring).BeginIndex = next_packet;
                    continue;
                }
            }
            break;
        }
        let fragment_begin = packet.FragmentIndex;
        if fragment_begin == fragment_end
            || fragment_count > unsafe { (*fragment_ring).NumberOfElements }
        {
            unsafe {
                (*packet).set_Ignore(1);
                if let Some(next_packet) = increment_index(&*packet_ring, packet_begin) {
                    (*packet_ring).BeginIndex = next_packet;
                    continue;
                }
            }
            break;
        }

        let mut total_length = 0usize;
        let mut fragment_index = fragment_begin;
        let mut valid = true;
        for _ in 0..fragment_count {
            let fragment = match unsafe { fragment_at(fragment_ring, fragment_index) } {
                Some(fragment) => unsafe { &*fragment },
                None => {
                    valid = false;
                    break;
                }
            };
            let address = match unsafe { fragment_virtual_address(extension, fragment_index) } {
                Some(address) => unsafe { &*address },
                None => {
                    valid = false;
                    break;
                }
            };
            if !validate_fragment(fragment, address, &mut total_length, frame_maximum) {
                valid = false;
                break;
            }
            fragment_index = match unsafe { increment_index(&*fragment_ring, fragment_index) } {
                Some(index) => index,
                None => {
                    valid = false;
                    break;
                }
            };
        }
        if !valid || !(FRAME_MINIMUM..=frame_maximum).contains(&total_length) {
            (*packet).set_Ignore(1);
            let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
                Some(index) => index,
                None => break,
            };
            let next_fragment =
                match unsafe { advance_index(&*fragment_ring, fragment_begin, fragment_count) } {
                    Some(index) => index,
                    None => break,
                };
            unsafe {
                (*packet_ring).BeginIndex = next_packet;
                (*fragment_ring).BeginIndex = next_fragment;
            }
            continue;
        }

        if !adaptive_polling_enabled(state)
            && at_passive_level()
            && deliver_transmit_packet_to_read(
                state,
                read_queue,
                fragment_ring,
                extension,
                fragment_begin,
                fragment_count,
                total_length,
                capture_generation,
            )
        {
            let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
                Some(index) => index,
                None => break,
            };
            let next_fragment =
                match unsafe { advance_index(&*fragment_ring, fragment_begin, fragment_count) } {
                    Some(index) => index,
                    None => break,
                };
            unsafe {
                (*packet_ring).BeginIndex = next_packet;
                (*fragment_ring).BeginIndex = next_fragment;
            }
            continue;
        }

        if frame_pool.is_null() {
            debug_status(b"Tx capture frame pool", STATUS_DEVICE_NOT_READY);
            break;
        }
        let mut frame = match Frame::new(frame_pool) {
            Ok(frame) => frame,
            Err(_) => {
                debug_status(
                    b"Tx capture frame allocation",
                    STATUS_INSUFFICIENT_RESOURCES,
                );
                unsafe {
                    if let Some(packet) = packet_at(packet_ring, packet_begin) {
                        (*packet).set_Ignore(1);
                    }
                }
                let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
                    Some(index) => index,
                    None => break,
                };
                let next_fragment =
                    match unsafe { advance_index(&*fragment_ring, fragment_begin, fragment_count) }
                    {
                        Some(index) => index,
                        None => break,
                    };
                unsafe {
                    (*packet_ring).BeginIndex = next_packet;
                    (*fragment_ring).BeginIndex = next_fragment;
                }
                continue;
            }
        };
        let mut offset = 0;
        let mut copy_succeeded = true;
        fragment_index = fragment_begin;
        for _ in 0..fragment_count {
            let fragment = match unsafe { fragment_at(fragment_ring, fragment_index) } {
                Some(fragment) => unsafe { &*fragment },
                None => {
                    copy_succeeded = false;
                    break;
                }
            };
            let address = match unsafe { fragment_virtual_address(extension, fragment_index) } {
                Some(address) => unsafe { &*address },
                None => {
                    copy_succeeded = false;
                    break;
                }
            };
            let start =
                unsafe { (address.VirtualAddress as *const u8).add(fragment.Offset() as usize) };
            let length = fragment.ValidLength() as usize;
            let data = unsafe { core::slice::from_raw_parts(start, length) };
            if frame.copy_from_slice(offset, data).is_err() {
                copy_succeeded = false;
                break;
            }
            offset += length;
            fragment_index = match unsafe { increment_index(&*fragment_ring, fragment_index) } {
                Some(index) => index,
                None => {
                    copy_succeeded = false;
                    break;
                }
            };
        }
        if copy_succeeded && offset == total_length {
            frame.set_length(total_length);
            if unsafe { (*state).lifecycle.load(Ordering::Acquire) } == INSTANCE_OPEN
                && unsafe { (*state).owner_generation.load(Ordering::Acquire) }
                    == capture_generation
            {
                let adaptive = unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) };
                if enqueue_existing_capture_frame(state, frame).is_ok() && !adaptive {
                    captured_for_legacy_read = true;
                }
            } else {
                (*packet).set_Ignore(1);
            }
        }

        let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
            Some(index) => index,
            None => break,
        };
        let next_fragment =
            match unsafe { advance_index(&*fragment_ring, fragment_begin, fragment_count) } {
                Some(index) => index,
                None => break,
            };
        unsafe {
            (*packet_ring).BeginIndex = next_packet;
            (*fragment_ring).BeginIndex = next_fragment;
        }
    }

    if captured_for_legacy_read {
        enqueue_work_item(read_work_item);
    }
}

fn adaptive_polling_enabled(state: *mut InstanceState) -> bool {
    if state.is_null() {
        return false;
    }
    unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) }
}

fn at_passive_level() -> bool {
    unsafe { wdk_sys::ntddk::KeGetCurrentIrql() == 0 }
}

fn complete_captured_frame_to_read(request: WDFREQUEST, frame: &Frame) -> NTSTATUS {
    let mut output = core::ptr::null_mut::<c_void>();
    let mut output_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveOutputBuffer,
            request,
            frame.as_bytes().len(),
            &mut output,
            &mut output_length,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }
    if output_length < frame.as_bytes().len() {
        return STATUS_BUFFER_TOO_SMALL;
    }

    unsafe {
        core::ptr::copy_nonoverlapping(
            frame.as_bytes().as_ptr(),
            output.cast::<u8>(),
            frame.as_bytes().len(),
        );
    }
    complete_request_with_information(request, STATUS_SUCCESS, frame.as_bytes().len());
    STATUS_SUCCESS
}

fn deliver_transmit_packet_to_read(
    state: *mut InstanceState,
    read_queue: WDFQUEUE,
    fragment_ring: *mut netadaptercx_sys::NET_RING,
    extension: &netadaptercx_sys::NET_EXTENSION,
    fragment_begin: u32,
    fragment_count: u32,
    total_length: usize,
    owner_generation: u64,
) -> bool {
    // Nonblocking: a TX packet callback must never wait on a lock that owner
    // cleanup, D0 exit, or release hardware can hold.
    let Some(_legacy_direct_read_guard) = (unsafe { LegacyDirectReadGuard::try_acquire(state) })
    else {
        return false;
    };
    if unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) }
        || unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN
        || unsafe { (*state).owner_generation.load(Ordering::Acquire) } != owner_generation
    {
        return false;
    }
    let mut request = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(WdfIoQueueRetrieveNextRequest, read_queue, &mut request,)
    };
    if status != STATUS_SUCCESS {
        return false;
    }
    unsafe { release_request(&(*state).pending_reads) };
    unsafe {
        (*state)
            .legacy_direct_read_claims
            .fetch_add(1, Ordering::AcqRel);
    }

    let mut output = core::ptr::null_mut::<c_void>();
    let mut output_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveOutputBuffer,
            request,
            total_length,
            &mut output,
            &mut output_length,
        )
    };
    if status != STATUS_SUCCESS {
        complete_request(request, status);
        release_legacy_direct_read_claim(state);
        return false;
    }
    if output_length < total_length {
        complete_request(request, STATUS_BUFFER_TOO_SMALL);
        release_legacy_direct_read_claim(state);
        return false;
    }

    let mut fragment_index = fragment_begin;
    let mut output_offset = 0usize;
    for _ in 0..fragment_count {
        let fragment = match unsafe { fragment_at(fragment_ring, fragment_index) } {
            Some(fragment) => unsafe { &*fragment },
            None => {
                complete_request(request, STATUS_DEVICE_NOT_READY);
                release_legacy_direct_read_claim(state);
                return false;
            }
        };
        let address = match unsafe { fragment_virtual_address(extension, fragment_index) } {
            Some(address) => unsafe { &*address },
            None => {
                complete_request(request, STATUS_DEVICE_NOT_READY);
                release_legacy_direct_read_claim(state);
                return false;
            }
        };
        let length = fragment.ValidLength() as usize;
        let source =
            unsafe { (address.VirtualAddress as *const u8).add(fragment.Offset() as usize) };
        unsafe {
            core::ptr::copy_nonoverlapping(source, output.cast::<u8>().add(output_offset), length);
        }
        output_offset += length;
        fragment_index = match unsafe { increment_index(&*fragment_ring, fragment_index) } {
            Some(index) => index,
            None => {
                complete_request(request, STATUS_DEVICE_NOT_READY);
                release_legacy_direct_read_claim(state);
                return false;
            }
        };
    }

    debug_assert!(output_offset == total_length);
    complete_request_with_information(request, STATUS_SUCCESS, output_offset);
    release_legacy_direct_read_claim(state);
    true
}

fn release_legacy_direct_read_claim(state: *mut InstanceState) {
    if !state.is_null() {
        release_request(unsafe { &(*state).legacy_direct_read_claims });
    }
}

fn validate_fragment(
    fragment: &netadaptercx_sys::NET_FRAGMENT,
    address: &netadaptercx_sys::NET_FRAGMENT_VIRTUAL_ADDRESS,
    total_length: &mut usize,
    frame_maximum: usize,
) -> bool {
    if address.VirtualAddress.is_null() {
        return false;
    }
    let offset = fragment.Offset() as usize;
    let capacity = fragment.Capacity() as usize;
    let valid_length = fragment.ValidLength() as usize;
    offset <= capacity
        && valid_length <= capacity - offset
        && valid_length <= frame_maximum
        && *total_length <= frame_maximum - valid_length
        && {
            *total_length += valid_length;
            true
        }
}

extern "C" fn evt_packet_queue_set_notification_enabled(
    queue: netadaptercx_sys::NETPACKETQUEUE,
    enabled: wdk_sys::BOOLEAN,
) {
    let context = unsafe { packet_queue_context(queue) };
    if context.is_null() || unsafe { (*context).is_transmit } {
        return;
    }
    let state = unsafe { (*context).instance };
    if state.is_null() {
        return;
    }
    unsafe {
        (*state)
            .rx_notification_armed
            .store(enabled != 0, Ordering::Release);
    }
    if enabled != 0 && has_queued_injection_frame(state) {
        let notification_queue = take_rx_notification(state);
        if !notification_queue.is_null() {
            notify_more_received_packets(notification_queue);
        }
    }
}

extern "C" fn evt_packet_queue_cancel(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let context = unsafe { packet_queue_context(queue) };
    if context.is_null() {
        return;
    }
    let state = unsafe { (*context).instance };
    if state.is_null() {
        return;
    }
    let is_receive = !unsafe { (*context).is_transmit };
    let rings = unsafe { core::ptr::addr_of!((*context).rings) };
    if rings.is_null() {
        return;
    }

    unsafe {
        let rings = &*rings;
        let packet_ring = rings.Rings[ring::PACKET_RING_INDEX];
        let fragment_ring = rings.Rings[ring::FRAGMENT_RING_INDEX];
        if is_receive && !packet_ring.is_null() {
            let mut packet_index = (*packet_ring).BeginIndex;
            while packet_index != (*packet_ring).EndIndex {
                let Some(packet) = packet_at(packet_ring, packet_index) else {
                    break;
                };
                (*packet).set_Ignore(1);
                let Some(next_packet) = increment_index(&*packet_ring, packet_index) else {
                    break;
                };
                packet_index = next_packet;
            }
        }
        for ring in [packet_ring, fragment_ring] {
            if !ring.is_null() {
                // Advancing BeginIndex to EndIndex returns all outstanding
                // packet and fragment entries to NetAdapterCx.
                (*ring).BeginIndex = (*ring).EndIndex;
            }
        }
    }

    if is_receive {
        unsafe {
            (*state)
                .rx_notification_armed
                .store(false, Ordering::Release);
        }
    }
}

extern "C" fn evt_set_receive_filter(
    adapter: netadaptercx_sys::NETADAPTER,
    receive_filter: netadaptercx_sys::NETRECEIVEFILTER,
) {
    let get_packet_filter: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETRECEIVEFILTER,
    )
        -> netadaptercx_sys::_NET_PACKET_FILTER_FLAGS = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetReceiveFilterGetPacketFilterTableIndex as usize,
        )
    };
    let get_multicast_address_count: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETRECEIVEFILTER,
    ) -> usize = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetReceiveFilterGetMulticastAddressCountTableIndex
                as usize,
        )
    };
    let get_multicast_address_list: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETRECEIVEFILTER,
    ) -> *const netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetReceiveFilterGetMulticastAddressListTableIndex
                as usize,
        )
    };

    let packet_filters =
        unsafe { get_packet_filter(netadaptercx_sys::NetDriverGlobals, receive_filter) };
    let requested_count =
        unsafe { get_multicast_address_count(netadaptercx_sys::NetDriverGlobals, receive_filter) };
    let address_count = requested_count.min(MAXIMUM_MULTICAST_ADDRESSES);
    if requested_count != address_count {
        debug_status(b"ReceiveFilter multicast count", STATUS_INVALID_BUFFER_SIZE);
    }

    let mut addresses = [[0; ETHERNET_ADDRESS_LENGTH]; MAXIMUM_MULTICAST_ADDRESSES];
    if address_count != 0 {
        let address_list = unsafe {
            get_multicast_address_list(netadaptercx_sys::NetDriverGlobals, receive_filter)
        };
        if address_list.is_null() {
            debug_status(b"ReceiveFilter multicast list", STATUS_INVALID_BUFFER_SIZE);
            if let Some(state) = unsafe { instance_from_adapter(adapter) } {
                let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                    return;
                };
                let state = &mut *state_guard;
                update_receive_filter_state(state, packet_filters, &addresses[..0]);
            }
            return;
        }
        for (index, address) in addresses.iter_mut().take(address_count).enumerate() {
            // NetAdapterCx owns the list for this callback; take a local,
            // fixed-size Ethernet snapshot before publishing state.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (*address_list.add(index)).Address.as_ptr(),
                    address.as_mut_ptr(),
                    ETHERNET_ADDRESS_LENGTH,
                );
            }
        }
    }

    if let Some(state) = unsafe { instance_from_adapter(adapter) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        update_receive_filter_state(state, packet_filters, &addresses[..address_count]);
    }
}

fn update_receive_filter_state(
    state: &mut InstanceState,
    packet_filters: netadaptercx_sys::_NET_PACKET_FILTER_FLAGS,
    multicast_addresses: &[[u8; ETHERNET_ADDRESS_LENGTH]],
) {
    state.active_packet_filters = packet_filters;
    state.active_multicast_address_count = multicast_addresses.len();
    for (index, address) in multicast_addresses.iter().enumerate() {
        state.active_multicast_addresses[index] = *address;
    }
    for index in multicast_addresses.len()..MAXIMUM_MULTICAST_ADDRESSES {
        state.active_multicast_addresses[index] = [0; ETHERNET_ADDRESS_LENGTH];
    }
}

fn clear_receive_filter_state(state: &mut InstanceState) {
    update_receive_filter_state(state, 0, &[]);
}

unsafe extern "C" fn evt_device_d0_entry(
    device: WDFDEVICE,
    _previous_state: wdk_sys::WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    if let Some(state) = unsafe { instance_from_pnp_device(device) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let (read_queue, adaptive) = {
            let state = &mut *state_guard;
            (
                state.read_queue,
                state.adaptive_enabled.load(Ordering::Acquire),
            )
        };
        drop(state_guard);
        if !adaptive {
            resume_manual_queue(read_queue);
        }
        reopen_frame_queues(state);
        let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        state_guard
            .lifecycle
            .store(INSTANCE_OPEN, Ordering::Release);
        drop(state_guard);
        // Packet callbacks are readmitted only after the queues are open.
        resume_datapath_callbacks(state, DATAPATH_CLOSED_POWER);
    }
    STATUS_SUCCESS
}

unsafe extern "C" fn evt_device_d0_exit(
    device: WDFDEVICE,
    _target_state: wdk_sys::WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    if let Some(state) = unsafe { instance_from_pnp_device(device) } {
        {
            let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return STATUS_DEVICE_NOT_READY;
            };
            state_guard
                .lifecycle
                .store(INSTANCE_SUSPENDED, Ordering::Release);
        }
        // Drain in-flight packet callbacks before any lock is taken so a
        // leased callback can never be blocked by this path. The power closer
        // stays set until D0 entry republishes the queues.
        quiesce_datapath_callbacks(state, DATAPATH_CLOSED_POWER);
        let Some(_legacy_direct_read_guard) = (unsafe { LegacyDirectReadGuard::acquire(state) })
        else {
            return STATUS_DEVICE_NOT_READY;
        };
        let Some(_write_lifetime_guard) = (unsafe { WriteLifetimeGuard::acquire(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let (read_queue, read_work_item, adaptive) = {
            let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return STATUS_DEVICE_NOT_READY;
            };
            (
                state_guard.read_queue,
                state_guard.read_work_item,
                state_guard.adaptive_enabled.load(Ordering::Acquire),
            )
        };
        cancel_wait_for_teardown(state);
        flush_work_item(read_work_item);
        if !adaptive {
            purge_queue(read_queue);
        }
        clear_frame_queues(state);
        let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        state_guard.pending_reads.store(0, Ordering::Release);
        state_guard.pending_writes.store(0, Ordering::Release);
        state_guard
            .rx_notification_armed
            .store(false, Ordering::Release);
    }
    STATUS_SUCCESS
}

extern "C" fn evt_device_prepare_hardware(
    device: WDFDEVICE,
    _resources_raw: WDFCMRESLIST,
    _resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let Some(state) = (unsafe { instance_from_pnp_device(device) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let (adapter, mac_address, mtu, frame_maximum) = {
        let state = &mut *state_guard;
        state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
        (
            state.adapter,
            state.mac_address,
            state.mtu,
            state.frame_maximum,
        )
    };
    drop(state_guard);
    if adapter.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    configure_adapter_link_state(adapter, mac_address, mtu);

    // Match NET_ADAPTER_TX_CAPABILITIES_INIT: this is a system-managed,
    // non-DMA path with no fragment-count limit.
    let mut tx = netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES>() as ULONG,
        MappingRequirement:
            netadaptercx_sys::_NET_MEMORY_MAPPING_REQUIREMENT_NetMemoryMappingRequirementNone,
        PayloadBackfill: 0,
        MaximumNumberOfFragments: netadaptercx_sys::SIZE_T::MAX,
        FragmentBufferAlignment: 1,
        FragmentRingNumberOfElementsHint: 0,
        MaximumNumberOfQueues: 1,
        DmaCapabilities: core::ptr::null_mut(),
    };
    let mut rx = netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES>() as ULONG,
        AllocationMode:
            netadaptercx_sys::_NET_RX_FRAGMENT_BUFFER_ALLOCATION_MODE_NetRxFragmentBufferAllocationModeSystem,
        AttachmentMode:
            netadaptercx_sys::_NET_RX_FRAGMENT_BUFFER_ATTACHMENT_MODE_NetRxFragmentBufferAttachmentModeSystem,
        FragmentRingNumberOfElementsHint: 0,
        MaximumFrameSize: frame_maximum as u64,
        MaximumNumberOfQueues: 1,
        __bindgen_anon_1:
            netadaptercx_sys::_NET_ADAPTER_RX_CAPABILITIES__bindgen_ty_1 {
                __bindgen_anon_2:
                    netadaptercx_sys::_NET_ADAPTER_RX_CAPABILITIES__bindgen_ty_1__bindgen_ty_2 {
                        FragmentBufferAlignment: 1,
                        ..netadaptercx_sys::_NET_ADAPTER_RX_CAPABILITIES__bindgen_ty_1__bindgen_ty_2::default()
                    },
            },
        ..netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES::default()
    };
    let set_data_path: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *mut netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES,
        *mut netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetDataPathCapabilitiesTableIndex as usize,
        )
    };
    // SAFETY: Adapter and capability structures are valid for this PASSIVE_LEVEL callback.
    unsafe {
        set_data_path(
            netadaptercx_sys::NetDriverGlobals,
            adapter,
            &mut tx,
            &mut rx,
        );
    }
    debug_marker(b"NetAdapterSetDataPathCapabilities");

    let mut receive_filter = netadaptercx_sys::NET_ADAPTER_RECEIVE_FILTER_CAPABILITIES {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_RECEIVE_FILTER_CAPABILITIES>()
            as ULONG,
        SupportedPacketFilters:
            netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagDirected
                | netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagBroadcast
                | netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagMulticast
                | netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagAllMulticast
                | netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagPromiscuous,
        MaximumMulticastAddresses: MAXIMUM_MULTICAST_ADDRESSES as u64,
        EvtSetReceiveFilter: Some(evt_set_receive_filter),
        ..netadaptercx_sys::NET_ADAPTER_RECEIVE_FILTER_CAPABILITIES::default()
    };
    let set_receive_filter: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
        *mut netadaptercx_sys::NET_ADAPTER_RECEIVE_FILTER_CAPABILITIES,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetAdapterSetReceiveFilterCapabilitiesTableIndex
                as usize,
        )
    };
    unsafe {
        set_receive_filter(
            netadaptercx_sys::NetDriverGlobals,
            adapter,
            &mut receive_filter,
        );
    }
    debug_marker(b"NetAdapterSetReceiveFilterCapabilities");

    let start: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
    ) -> NTSTATUS =
        unsafe { net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterStartTableIndex as usize) };
    // Readmit packet callbacks before the framework can create queues again; a
    // callback still observes INSTANCE_CLOSING until D0 entry publishes the
    // reopened queues, and the power closer keeps admission shut across a
    // resume until D0 entry releases it.
    resume_datapath_callbacks(state, DATAPATH_CLOSED_HARDWARE);
    // SAFETY: Adapter was created by NetAdapterCx and is started once here.
    debug_marker(b"NetAdapterStart enter");
    let status = unsafe { start(netadaptercx_sys::NetDriverGlobals, adapter) };
    debug_status(b"NetAdapterStart", status);
    status
}

extern "C" fn evt_device_release_hardware(
    device: WDFDEVICE,
    _resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let Some(state) = (unsafe { instance_from_pnp_device(device) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let (adapter, read_queue, read_work_item, adaptive) = {
        let state = &mut *state_guard;
        state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
        (
            state.adapter,
            state.read_queue,
            state.read_work_item,
            state.adaptive_enabled.load(Ordering::Acquire),
        )
    };
    drop(state_guard);
    if !adapter.is_null() {
        let stop: unsafe extern "system" fn(
            netadaptercx_sys::PNET_DRIVER_GLOBALS,
            netadaptercx_sys::NETADAPTER,
        ) = unsafe {
            net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterStopTableIndex as usize)
        };
        // SAFETY: NetAdapterCx owns the adapter and release-hardware runs
        // after the framework has stopped datapath activity.
        unsafe { stop(netadaptercx_sys::NetDriverGlobals, adapter) };
    }
    // Close admission and drain any packet callback that is still in flight
    // before taking a lock or invalidating queue-local state. The hardware
    // closer is released only by a later prepare-hardware.
    quiesce_datapath_callbacks(state, DATAPATH_CLOSED_HARDWARE);
    let Some(_legacy_direct_read_guard) = (unsafe { LegacyDirectReadGuard::acquire(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let Some(_write_lifetime_guard) = (unsafe { WriteLifetimeGuard::acquire(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };

    cancel_wait_for_teardown(state);
    flush_work_item(read_work_item);
    if !adaptive {
        purge_queue(read_queue);
    }
    flush_work_item(read_work_item);
    clear_frame_queues(state);
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let state = &mut *state_guard;
    // The lookaside list is parented to the device; leave it alive until WDF
    // tears down the device so any in-flight Frame can release its memory.
    clear_receive_filter_state(state);
    state.pending_reads.store(0, Ordering::Release);
    state.pending_writes.store(0, Ordering::Release);
    state.tx_queue = core::ptr::null_mut();
    state.rx_queue = core::ptr::null_mut();
    state.tx_queue_started.store(false, Ordering::Release);
    state.rx_queue_started.store(false, Ordering::Release);
    state.rx_notification_armed.store(false, Ordering::Release);
    state.adapter = core::ptr::null_mut();
    state.lifecycle.store(INSTANCE_CLOSED, Ordering::Release);

    STATUS_SUCCESS
}

fn create_tap_device(device: WDFDEVICE, state: &mut InstanceState) -> NTSTATUS {
    let queue_byte_limit = match FRAME_QUEUE_LIMIT.checked_mul(state.frame_maximum) {
        Some(limit) => limit,
        None => {
            debug_status(b"FrameQueueBudget", STATUS_INSUFFICIENT_RESOURCES);
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
            }
            return STATUS_INSUFFICIENT_RESOURCES;
        }
    };
    let mut lookaside_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ParentObject: device.cast(),
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut frame_pool: wdk_sys::WDFLOOKASIDE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfLookasideListCreate,
            &mut lookaside_attributes,
            FRAME_STORAGE_SIZE,
            wdk_sys::_POOL_TYPE::NonPagedPoolNx,
            WDF_NO_OBJECT_ATTRIBUTES,
            u32::from_le_bytes(*b"WTFR"),
            &mut frame_pool,
        )
    };
    if status != STATUS_SUCCESS {
        debug_status(b"FramePoolCreate", status);
        return status;
    }
    let injection_queue = match FrameQueue::try_new(FRAME_QUEUE_LIMIT, queue_byte_limit) {
        Ok(queue) => queue,
        Err(_) => {
            debug_status(b"InjectionQueueCreate", STATUS_INSUFFICIENT_RESOURCES);
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
            }
            return STATUS_INSUFFICIENT_RESOURCES;
        }
    };
    let capture_queue = match FrameQueue::try_new(FRAME_QUEUE_LIMIT, queue_byte_limit) {
        Ok(queue) => queue,
        Err(_) => {
            debug_status(b"CaptureQueueCreate", STATUS_INSUFFICIENT_RESOURCES);
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
            }
            return STATUS_INSUFFICIENT_RESOURCES;
        }
    };
    let mut lock_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ParentObject: device.cast(),
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut injection_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfSpinLockCreate,
            &mut lock_attributes,
            &mut injection_lock,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let mut capture_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfSpinLockCreate,
            &mut lock_attributes,
            &mut capture_lock,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let mut state_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockCreate, &mut lock_attributes, &mut state_lock,)
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let mut write_lifetime_lock: WDFWAITLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfWaitLockCreate,
            &mut lock_attributes,
            &mut write_lifetime_lock,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let mut legacy_direct_read_lock: WDFWAITLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfWaitLockCreate,
            &mut lock_attributes,
            &mut legacy_direct_read_lock,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let mut control_queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchParallel,
        AllowZeroLengthRequests: 1,
        EvtIoDeviceControl: Some(evt_io_device_control),
        EvtIoStop: Some(evt_io_stop),
        ..WDF_IO_QUEUE_CONFIG::default()
    };
    unsafe {
        control_queue_config
            .Settings
            .Parallel
            .NumberOfPresentedRequests = ULONG::MAX;
    }
    // Queue-level automatic synchronization serializes this queue's request
    // handlers with EvtIoStop, so EvtIoStop can never observe a control request
    // that evt_io_device_control still owns. Passive execution keeps the
    // serialized callbacks at PASSIVE_LEVEL. No callback that runs under this
    // lock may block on a driver lock a teardown path can hold.
    let mut control_queue_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelPassive,
        SynchronizationScope: wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeQueue,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut control_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut control_queue_config,
            &mut control_queue_attributes,
            &mut control_queue,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceConfigureRequestDispatching,
            device,
            control_queue,
            wdk_sys::_WDF_REQUEST_TYPE::WdfRequestTypeDeviceControl,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let mut default_queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchSequential,
        AllowZeroLengthRequests: 1,
        DefaultQueue: 1,
        EvtIoRead: Some(evt_io_read),
        EvtIoWrite: Some(evt_io_write),
        ..WDF_IO_QUEUE_CONFIG::default()
    };
    let mut default_queue_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelPassive,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut default_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut default_queue_config,
            &mut default_queue_attributes,
            &mut default_queue,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let mut work_item_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ParentObject: device.cast(),
        ContextTypeInfo: &raw const WORK_ITEM_CONTEXT_TYPE_INFO,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut read_work_config = WDF_WORKITEM_CONFIG {
        Size: core::mem::size_of::<WDF_WORKITEM_CONFIG>() as ULONG,
        EvtWorkItemFunc: Some(evt_read_completion_work_item),
        ..WDF_WORKITEM_CONFIG::default()
    };
    let mut read_work_item: WDFWORKITEM = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfWorkItemCreate,
            &mut read_work_config,
            &mut work_item_attributes,
            &mut read_work_item,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    let work_item_context = unsafe {
        object_context::<WorkItemContext>(
            read_work_item.cast(),
            &raw const WORK_ITEM_CONTEXT_TYPE_INFO,
        )
    };
    if work_item_context.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*work_item_context).instance = state;
    }
    state.read_work_item = read_work_item;

    let mut read_queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchManual,
        AllowZeroLengthRequests: 1,
        EvtIoStop: Some(evt_io_stop),
        ..WDF_IO_QUEUE_CONFIG::default()
    };
    let mut read_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut read_queue_config,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut read_queue,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    state.read_queue = read_queue;
    state.injection_lock = injection_lock;
    state.capture_lock = capture_lock;
    state.frame_pool = frame_pool;
    state.state_lock = state_lock;
    state.legacy_direct_read_lock = legacy_direct_read_lock;
    state.write_lifetime_lock = write_lifetime_lock;
    state.injection_queue = Some(injection_queue);
    state.capture_queue = Some(capture_queue);
    state.read_work_item = read_work_item;
    STATUS_SUCCESS
}

unsafe extern "C" fn evt_instance_context_destroy(object: WDFOBJECT) {
    let context =
        unsafe { object_context::<DeviceContext>(object, &raw const DEVICE_CONTEXT_TYPE_INFO) };
    if !context.is_null() && unsafe { !(*context).instance.is_null() } {
        let state = unsafe { (*context).instance };
        unsafe {
            (*context).instance = core::ptr::null_mut();
        }
        unsafe {
            drop(Box::from_raw(state));
        }
    }
}

extern "C" fn evt_file_create(device: WDFDEVICE, request: WDFREQUEST, _file_object: WDFFILEOBJECT) {
    let Some(state) = (unsafe { instance_from_device(device) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if unsafe {
        (*state)
            .control_open
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    } {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestComplete, request, STATUS_SUCCESS);
    }
}

extern "C" fn evt_file_close(_file_object: WDFFILEOBJECT) {}

extern "C" fn evt_file_cleanup(file_object: WDFFILEOBJECT) {
    let device = unsafe { call_unsafe_wdf_function_binding!(WdfFileObjectGetDevice, file_object) };
    if let Some(state) = unsafe { instance_from_device(device) } {
        // Retire the owner before quiescing so a leased packet callback either
        // observes the retired owner or has already finished.
        let was_suspended = {
            let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return;
            };
            let was_suspended = state_guard.lifecycle.load(Ordering::Acquire) == INSTANCE_SUSPENDED;
            state_guard.owner_generation.fetch_add(1, Ordering::AcqRel);
            state_guard
                .lifecycle
                .store(INSTANCE_CLOSING, Ordering::Release);
            was_suspended
        };
        // Close packet-callback admission and drain outstanding leases while
        // holding no lock that a leased callback can wait on. The owner closer
        // is scoped to this cleanup, so releasing it cannot readmit callbacks
        // while D0 exit or release hardware still holds its own closer.
        let owner_quiesce = DatapathQuiesceGuard::acquire(state, DATAPATH_CLOSED_OWNER);
        let Some(_legacy_direct_read_guard) = (unsafe { LegacyDirectReadGuard::acquire(state) })
        else {
            return;
        };
        let Some(_write_lifetime_guard) = (unsafe { WriteLifetimeGuard::acquire(state) }) else {
            return;
        };
        let (read_queue, read_work_item, adaptive) = {
            let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return;
            };
            (
                state_guard.read_queue,
                state_guard.read_work_item,
                state_guard.adaptive_enabled.load(Ordering::Acquire),
            )
        };
        cancel_wait_for_teardown(state);
        flush_work_item(read_work_item);
        if !adaptive {
            purge_queue(read_queue);
        }
        clear_frame_queues(state);
        let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let should_resume = state_guard.lifecycle.load(Ordering::Acquire) == INSTANCE_CLOSING
            && !state_guard.adapter.is_null()
            && !was_suspended;
        state_guard.pending_reads.store(0, Ordering::Release);
        state_guard.pending_writes.store(0, Ordering::Release);
        state_guard.adaptive_enabled.store(false, Ordering::Release);
        state_guard.control_open.store(false, Ordering::Release);
        drop(state_guard);
        if should_resume {
            reopen_frame_queues(state);
            let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return;
            };
            state_guard
                .lifecycle
                .store(INSTANCE_OPEN, Ordering::Release);
            drop(state_guard);
            // Packet callbacks may only be readmitted after the next owner's
            // queues are open again.
            drop(owner_quiesce);
            // A reopened manual queue belongs to a future legacy owner, never to this adaptive owner.
            resume_manual_queue(read_queue);
        }
        // When this owner did not reopen the queues, the power or hardware
        // closer held by D0 exit or release hardware keeps admission closed
        // after the owner closer is released here.
    }
}

extern "C" fn evt_io_device_control(
    queue: WDFQUEUE,
    request: WDFREQUEST,
    output_length: usize,
    input_length: usize,
    ioctl: ULONG,
) {
    let Some(state) = (unsafe { instance_from_io_queue(queue) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };

    match ioctl {
        TAP_IOCTL_ENABLE_ADAPTIVE_POLLING => {
            handle_enable_adaptive_polling(state, request, output_length, input_length);
        }
        TAP_IOCTL_WAIT_FOR_CHANGE => {
            handle_wait_for_change(state, request, output_length, input_length);
        }
        _ => complete_request(request, STATUS_INVALID_DEVICE_REQUEST),
    }
}

fn handle_enable_adaptive_polling(
    state: *mut InstanceState,
    request: WDFREQUEST,
    output_length: usize,
    input_length: usize,
) {
    if input_length != core::mem::size_of::<AdaptiveEnableRequest>()
        || output_length != core::mem::size_of::<AdaptiveEnableResponse>()
    {
        complete_request(request, STATUS_INVALID_BUFFER_SIZE);
        return;
    }

    let mut input = core::ptr::null_mut::<c_void>();
    let mut actual_input_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveInputBuffer,
            request,
            core::mem::size_of::<AdaptiveEnableRequest>(),
            &mut input,
            &mut actual_input_length,
        )
    };
    if status != STATUS_SUCCESS
        || actual_input_length != core::mem::size_of::<AdaptiveEnableRequest>()
    {
        complete_request(
            request,
            if status == STATUS_SUCCESS {
                STATUS_INVALID_BUFFER_SIZE
            } else {
                status
            },
        );
        return;
    }
    let enable = unsafe { input.cast::<AdaptiveEnableRequest>().read() };
    if enable.version != ADAPTIVE_POLLING_PROTOCOL_VERSION
        || enable.flags == 0
        || enable.flags & !ADAPTIVE_INTEREST_SUPPORTED != 0
    {
        complete_request(request, STATUS_NOT_SUPPORTED);
        return;
    }

    let mut output = core::ptr::null_mut::<c_void>();
    let mut actual_output_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveOutputBuffer,
            request,
            core::mem::size_of::<AdaptiveEnableResponse>(),
            &mut output,
            &mut actual_output_length,
        )
    };
    if status != STATUS_SUCCESS
        || actual_output_length < core::mem::size_of::<AdaptiveEnableResponse>()
    {
        complete_request(
            request,
            if status == STATUS_SUCCESS {
                STATUS_BUFFER_TOO_SMALL
            } else {
                status
            },
        );
        return;
    }

    // The control queue's automatic synchronization lock is held here, so this
    // acquisition must not block: a teardown path can hold the direct-read lock
    // while it completes a control request, which needs the same queue lock.
    let Some(_legacy_direct_read_guard) = (unsafe { LegacyDirectReadGuard::try_acquire(state) })
    else {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    };
    let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if state_guard.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN {
        drop(state_guard);
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if state_guard.adaptive_enabled.load(Ordering::Acquire)
        || state_guard.pending_reads.load(Ordering::Acquire) != 0
        || state_guard
            .legacy_direct_read_claims
            .load(Ordering::Acquire)
            != 0
    {
        drop(state_guard);
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    state_guard.adaptive_enabled.store(true, Ordering::Release);
    drop(state_guard);

    unsafe {
        output
            .cast::<AdaptiveEnableResponse>()
            .write(AdaptiveEnableResponse {
                version: ADAPTIVE_POLLING_PROTOCOL_VERSION,
                flags: enable.flags,
            });
    }
    complete_request_with_information(
        request,
        STATUS_SUCCESS,
        core::mem::size_of::<AdaptiveEnableResponse>(),
    );
}

fn handle_wait_for_change(
    state: *mut InstanceState,
    request: WDFREQUEST,
    output_length: usize,
    input_length: usize,
) {
    if input_length != core::mem::size_of::<AdaptiveWaitRequest>()
        || output_length != core::mem::size_of::<AdaptiveWaitResponse>()
    {
        complete_request(request, STATUS_INVALID_BUFFER_SIZE);
        return;
    }

    let mut input = core::ptr::null_mut::<c_void>();
    let mut actual_input_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveInputBuffer,
            request,
            core::mem::size_of::<AdaptiveWaitRequest>(),
            &mut input,
            &mut actual_input_length,
        )
    };
    if status != STATUS_SUCCESS
        || actual_input_length != core::mem::size_of::<AdaptiveWaitRequest>()
    {
        complete_request(
            request,
            if status == STATUS_SUCCESS {
                STATUS_INVALID_BUFFER_SIZE
            } else {
                status
            },
        );
        return;
    }
    let wait = unsafe { input.cast::<AdaptiveWaitRequest>().read() };
    if wait.version != ADAPTIVE_POLLING_PROTOCOL_VERSION
        || wait.interest == 0
        || wait.interest & !ADAPTIVE_INTEREST_SUPPORTED != 0
    {
        complete_request(request, STATUS_NOT_SUPPORTED);
        return;
    }

    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN
        || !unsafe { (*state).rx_queue_started.load(Ordering::Acquire) }
    {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if !unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) } {
        complete_request(request, STATUS_INVALID_DEVICE_REQUEST);
        return;
    }
    // Admit exactly one wait per exclusive handle by claiming the single
    // request slot. The control queue is queue-synchronized, so EvtIoStop
    // cannot run while this dispatch owns the request; the slot exists so that
    // cancellation and teardown identify the published wait, and it is released
    // only after the wait record has been retired.
    if unsafe {
        (*state).wait_request.compare_exchange(
            core::ptr::null_mut(),
            request.cast(),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
    }
    .is_err()
    {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let (_, capture_generation, injection_generation) = readiness_snapshot(state);
    unsafe {
        (*state)
            .wait_interest
            .store(wait.interest, Ordering::Release);
        (*state)
            .wait_registration_capture_generation
            .store(capture_generation, Ordering::Release);
        (*state)
            .wait_registration_injection_generation
            .store(injection_generation, Ordering::Release);
        (*state)
            .wait_cancel_handoff
            .store(WAIT_HANDOFF_NONE, Ordering::Release);
    }
    // Publishing the record advances its sequence and clears the satisfied
    // mask, so no readiness bit from a previous wait can survive into this one.
    begin_wait_record(state);
    // Recheck after publishing REGISTERING so teardown either observes and
    // claims this registration or this path retires it before marking.
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN
        || !unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) }
        || !unsafe { (*state).rx_queue_started.load(Ordering::Acquire) }
    {
        finish_wait(state);
        complete_request(request, STATUS_CANCELLED);
        return;
    }

    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestMarkCancelableEx,
            request,
            Some(evt_wait_for_change_cancel),
        )
    };
    if status == STATUS_CANCELLED {
        // MarkCancelableEx does not invoke the cancellation callback when the
        // request was already cancelled; registration retains completion ownership.
        finish_wait(state);
        complete_request(request, STATUS_CANCELLED);
        return;
    }
    if status != STATUS_SUCCESS {
        // WDF did not accept the cancellation routine, so no cancellation
        // callback can run and registration owns terminal completion.
        finish_wait(state);
        complete_request(request, status);
        return;
    }
    // The wait becomes claimable only after WDF accepts the cancellation
    // routine. A cancellation that observed REGISTERING leaves WAIT_CANCELLED
    // for this path to retire without publishing.
    match transition_wait_record(state, WAIT_REGISTERING, WAIT_PENDING) {
        Ok(_) => {
            let (ready, observed_capture_generation, observed_injection_generation) =
                readiness_snapshot(state);
            let registration_capture_generation = unsafe {
                (*state)
                    .wait_registration_capture_generation
                    .load(Ordering::Acquire)
            };
            let registration_injection_generation = unsafe {
                (*state)
                    .wait_registration_injection_generation
                    .load(Ordering::Acquire)
            };
            if ready & wait.interest != 0
                || observed_capture_generation != registration_capture_generation
                || observed_injection_generation != registration_injection_generation
            {
                let _ = claim_wait_for_passive_completion(state, ready & wait.interest, request);
            }
        }
        Err(observed) if wait_record_state(observed) == WAIT_TEARDOWN => {
            // Teardown claimed this registration and left the unmark handshake
            // and terminal completion here.
            if transition_wait_record(state, WAIT_TEARDOWN, WAIT_UNMARKING).is_ok() {
                unmark_and_complete_wait(state, request, STATUS_CANCELLED, 0);
            } else {
                // The cancellation callback claimed the teardown state and owns
                // terminal completion; retire the wait so it is not left stuck.
                finish_wait(state);
            }
        }
        Err(_) => {
            // WAIT_CANCELLED: the cancellation callback completed the request.
            finish_wait(state);
        }
    }
}

unsafe extern "C" fn evt_wait_for_change_cancel(request: WDFREQUEST) {
    let queue = unsafe { call_unsafe_wdf_function_binding!(WdfRequestGetIoQueue, request) };
    if let Some(state) = unsafe { instance_from_io_queue(queue) } {
        cancel_wait_from_wdf(state, request);
        return;
    }
    complete_request(request, STATUS_CANCELLED);
}

extern "C" fn evt_io_read(_queue: WDFQUEUE, request: WDFREQUEST, _length: usize) {
    let Some(state) = (unsafe { instance_from_io_queue(_queue) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) } {
        let owner_generation = unsafe { (*state).owner_generation.load(Ordering::Acquire) };
        let frame = dequeue_capture_frame(state);
        if let Some(frame) = frame {
            let status = complete_captured_frame_to_read(request, &frame);
            if status != STATUS_SUCCESS {
                requeue_capture_frame_and_schedule_wait(state, frame, owner_generation);
                complete_request(request, status);
            }
        } else {
            complete_request(request, STATUS_NO_MORE_ENTRIES);
        }
        return;
    }
    if !unsafe { try_admit(&(*state).pending_reads, PENDING_READ_LIMIT) } {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let frame = dequeue_capture_frame(state);
    if let Some(frame) = frame {
        let owner_generation = unsafe { (*state).owner_generation.load(Ordering::Acquire) };
        unsafe { release_request(&(*state).pending_reads) };

        let status = complete_captured_frame_to_read(request, &frame);
        if status != STATUS_SUCCESS {
            requeue_capture_frame_and_schedule_wait(state, frame, owner_generation);
            complete_request(request, status);
        }
        return;
    }
    let target = unsafe { (*state).read_queue };
    if !forward_request(request, target) {
        unsafe { release_request(&(*state).pending_reads) };
    }
}

extern "C" fn evt_io_write(_queue: WDFQUEUE, request: WDFREQUEST, length: usize) {
    let Some(state) = (unsafe { instance_from_io_queue(_queue) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    let Some(_write_lifetime_guard) = (unsafe { WriteLifetimeGuard::acquire(state) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if length == 0 {
        complete_request(request, STATUS_SUCCESS);
        return;
    }
    let frame_maximum = unsafe { (*state).frame_maximum };
    if !(FRAME_MINIMUM..=frame_maximum).contains(&length) {
        complete_request(request, STATUS_INVALID_PARAMETER);
        return;
    }
    if !unsafe { try_admit(&(*state).pending_writes, PENDING_WRITE_LIMIT) } {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }

    let mut input = core::ptr::null_mut::<c_void>();
    let mut input_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveInputBuffer,
            request,
            FRAME_MINIMUM,
            &mut input,
            &mut input_length,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe { release_request(&(*state).pending_writes) };
        complete_request(request, status);
        return;
    }
    if input_length > frame_maximum {
        unsafe { release_request(&(*state).pending_writes) };
        complete_request(request, STATUS_INVALID_BUFFER_SIZE);
        return;
    }

    let bytes = unsafe { core::slice::from_raw_parts(input.cast::<u8>(), input_length) };
    let notification_queue = match enqueue_injection_frame(state, bytes) {
        Ok(()) => {
            let notification_queue = take_rx_notification(state);
            unsafe { release_request(&(*state).pending_writes) };
            complete_request_with_information(request, STATUS_SUCCESS, input_length);
            notification_queue
        }
        Err(QueueError::Full) => {
            unsafe { release_request(&(*state).pending_writes) };
            complete_request(request, STATUS_DEVICE_BUSY);
            core::ptr::null_mut()
        }
        Err(QueueError::Closed) => {
            unsafe { release_request(&(*state).pending_writes) };
            complete_request(request, STATUS_DEVICE_NOT_READY);
            core::ptr::null_mut()
        }
        Err(QueueError::InvalidFrameLength) => {
            unsafe { release_request(&(*state).pending_writes) };
            complete_request(request, STATUS_INVALID_BUFFER_SIZE);
            core::ptr::null_mut()
        }
        Err(QueueError::InsufficientResources) => {
            unsafe { release_request(&(*state).pending_writes) };
            complete_request(request, STATUS_INSUFFICIENT_RESOURCES);
            core::ptr::null_mut()
        }
    };
    if !notification_queue.is_null() {
        notify_more_received_packets(notification_queue);
    }
}

extern "C" fn evt_io_stop(queue: WDFQUEUE, request: WDFREQUEST, _action_flags: ULONG) {
    let Some(state) = (unsafe { instance_from_io_queue(queue) }) else {
        complete_request(request, STATUS_CANCELLED);
        return;
    };
    if queue == unsafe { (*state).read_queue } {
        // WDF owns pending manual READ requests; the driver may complete them.
        unsafe { release_request(&(*state).pending_reads) };
        complete_request(request, STATUS_CANCELLED);
        return;
    }
    // The control queue is queue-synchronized, so no request handler is running
    // and the only control request the driver can still own is the published
    // adaptive wait.
    if wait_request_is_active(state, request) {
        if cancel_wait_request_for_teardown(state, request) {
            // This call took exclusive terminal ownership and completed the
            // request; the stop is satisfied by that completion.
            return;
        }
        // Cancellation or the passive completion worker already owns terminal
        // completion and can be manipulating this handle right now. Touching it
        // here — including WdfRequestStopAcknowledge — would race that owner.
        // The owner completes promptly, which releases the D0 transition.
        return;
    }
    // Unreachable while the queue lock serializes dispatch with this callback.
    // Requeue defensively so the framework redelivers the request after the
    // device returns to D0 rather than waiting on an owner that does not exist.
    acknowledge_stopped_request(request);
}

fn forward_request(request: WDFREQUEST, target_queue: WDFQUEUE) -> bool {
    if target_queue.is_null() {
        complete_request(request, STATUS_NOT_SUPPORTED);
        return false;
    }

    let status = unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestForwardToIoQueue, request, target_queue)
    };
    if status != STATUS_SUCCESS {
        debug_status(b"WdfRequestForwardToIoQueue", status);
        complete_request(request, status);
        return false;
    }
    true
}

fn try_admit(counter: &AtomicUsize, limit: usize) -> bool {
    let mut current = counter.load(Ordering::Acquire);
    loop {
        if current >= limit {
            return false;
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn release_request(counter: &AtomicUsize) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        count.checked_sub(1)
    });
}

fn complete_request(request: WDFREQUEST, status: NTSTATUS) {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestComplete, request, status);
    }
}

fn complete_request_with_information(request: WDFREQUEST, status: NTSTATUS, information: usize) {
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestCompleteWithInformation,
            request,
            status,
            information as u64,
        );
    }
}

fn purge_queue(queue: WDFQUEUE) {
    if !queue.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfIoQueuePurgeSynchronously, queue);
        }
    }
}

fn resume_manual_queue(queue: WDFQUEUE) {
    if !queue.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfIoQueueStart, queue);
        }
    }
}

fn enqueue_work_item(work_item: WDFWORKITEM) {
    if !work_item.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemEnqueue, work_item);
        }
    }
}

fn flush_work_item(work_item: WDFWORKITEM) {
    if !work_item.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemFlush, work_item);
        }
    }
}

fn take_rx_notification(state: *mut InstanceState) -> netadaptercx_sys::NETPACKETQUEUE {
    if state.is_null() {
        return core::ptr::null_mut();
    }
    if unsafe { (*state).rx_notification_armed.swap(false, Ordering::AcqRel) } {
        unsafe { (*state).rx_queue }
    } else {
        core::ptr::null_mut()
    }
}

fn notify_more_received_packets(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let notify: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETPACKETQUEUE,
    ) = unsafe {
        net_function(
            netadaptercx_sys::_NETFUNCENUM_NetRxQueueNotifyMoreReceivedPacketsAvailableTableIndex
                as usize,
        )
    };
    unsafe {
        notify(netadaptercx_sys::NetDriverGlobals, queue);
    }
}

extern "C" fn evt_read_completion_work_item(work_item: WDFWORKITEM) {
    let Some(state) = (unsafe { instance_from_work_item(work_item) }) else {
        return;
    };
    loop {
        let scheduled_wait = take_scheduled_wait(state);
        if let Some((request, satisfied)) = scheduled_wait {
            complete_scheduled_wait_at_passive(state, request, satisfied);
            continue;
        }

        let mut request = core::ptr::null_mut();
        let (frame, owner_generation) = {
            let status = unsafe {
                call_unsafe_wdf_function_binding!(
                    WdfIoQueueRetrieveNextRequest,
                    (*state).read_queue,
                    &mut request,
                )
            };
            if status != STATUS_SUCCESS {
                return;
            }
            let frame = match dequeue_capture_frame(state) {
                Some(frame) => frame,
                None => {
                    let target = unsafe { (*state).read_queue };
                    if !forward_request(request, target) {
                        unsafe { release_request(&(*state).pending_reads) };
                    }
                    return;
                }
            };
            unsafe { release_request(&(*state).pending_reads) };
            (frame, unsafe {
                (*state).owner_generation.load(Ordering::Acquire)
            })
        };
        let status = complete_captured_frame_to_read(request, &frame);
        if status != STATUS_SUCCESS {
            requeue_capture_frame_and_schedule_wait(state, frame, owner_generation);
            complete_request(request, status);
        }
    }
}

fn enqueue_injection_frame(state: *mut InstanceState, bytes: &[u8]) -> Result<(), QueueError> {
    let lock = unsafe { (*state).injection_lock };
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let capacity = (*state)
            .injection_queue
            .as_ref()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.check_capacity(bytes.len()));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        capacity?;
    }

    let frame = Frame::from_bytes(unsafe { (*state).frame_pool }, bytes)?;
    enqueue_existing_injection_frame(state, frame)
}

fn enqueue_existing_injection_frame(
    state: *mut InstanceState,
    frame: Frame,
) -> Result<(), QueueError> {
    let lock = unsafe { (*state).injection_lock };
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let result = (*state)
            .injection_queue
            .as_mut()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.enqueue(frame));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        if result.is_ok() {
            (*state)
                .injection_generation
                .fetch_add(1, Ordering::Release);
        }
        result
    }
}

fn enqueue_existing_capture_frame(
    state: *mut InstanceState,
    frame: Frame,
) -> Result<(), QueueError> {
    let lock = unsafe { (*state).capture_lock };
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let (result, was_empty) = match (*state).capture_queue.as_mut() {
            Some(queue) => {
                let was_empty = queue.is_empty();
                (queue.enqueue(frame), was_empty)
            }
            None => (Err(QueueError::Closed), false),
        };
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        if result.is_ok() {
            (*state).capture_generation.fetch_add(1, Ordering::Release);
            if was_empty {
                let _ = claim_wait_for_passive_completion(
                    state,
                    ADAPTIVE_INTEREST_READABLE,
                    core::ptr::null_mut(),
                );
            }
        }
        result
    }
}

fn dequeue_injection_frame(state: *mut InstanceState) -> (Option<Frame>, bool) {
    let lock = unsafe { (*state).injection_lock };
    if lock.is_null() {
        return (None, false);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let (frame, was_full) = match (*state).injection_queue.as_mut() {
            Some(queue) => {
                let was_full =
                    queue.state() == QueueState::Open && queue.len() == FRAME_QUEUE_LIMIT;
                (queue.dequeue(), was_full)
            }
            None => (None, false),
        };
        let was_full_transition = frame.is_some() && was_full;
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        if frame.is_some() {
            (*state)
                .injection_generation
                .fetch_add(1, Ordering::Release);
        }
        (frame, was_full_transition)
    }
}

fn requeue_injection_frame(state: *mut InstanceState, frame: Frame, owner_generation: u64) {
    // Called from the RX advance callback, which already holds a receive
    // lease. The owner check and the enqueue are therefore atomic with respect
    // to owner cleanup, so an old owner's frame can never land in a new
    // owner's queue; otherwise the frame is released here.
    if !state.is_null()
        && unsafe { (*state).lifecycle.load(Ordering::Acquire) } == INSTANCE_OPEN
        && unsafe { (*state).owner_generation.load(Ordering::Acquire) } == owner_generation
    {
        let _ = enqueue_existing_injection_frame(state, frame);
    }
}

fn requeue_capture_frame_and_schedule_wait(
    state: *mut InstanceState,
    frame: Frame,
    owner_generation: u64,
) {
    // The passive READ paths are not packet callbacks, so they take the
    // capture lease here. Owner cleanup cannot clear or reopen the capture
    // queue between the ownership check and the enqueue; a frame that fails
    // the check belongs to a retired owner and is released.
    let Some(_capture_lease) = acquire_capture_lease(state) else {
        return;
    };
    if unsafe { (*state).lifecycle.load(Ordering::Acquire) } == INSTANCE_OPEN
        && unsafe { (*state).owner_generation.load(Ordering::Acquire) } == owner_generation
    {
        let _ = enqueue_existing_capture_frame(state, frame);
    }
}

fn dequeue_capture_frame(state: *mut InstanceState) -> Option<Frame> {
    let lock = unsafe { (*state).capture_lock };
    if lock.is_null() {
        return None;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let frame = (*state)
            .capture_queue
            .as_mut()
            .and_then(FrameQueue::dequeue);
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        if frame.is_some() {
            (*state).capture_generation.fetch_add(1, Ordering::Release);
        }
        frame
    }
}

fn readiness_snapshot(state: *mut InstanceState) -> (u32, u64, u64) {
    if state.is_null() {
        return (0, 0, 0);
    }
    let capture_generation = unsafe { (*state).capture_generation.load(Ordering::Acquire) };
    let injection_generation = unsafe { (*state).injection_generation.load(Ordering::Acquire) };
    let mut ready = 0;
    let capture_lock = unsafe { (*state).capture_lock };
    if !capture_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, capture_lock);
            if (*state)
                .capture_queue
                .as_ref()
                .is_some_and(|queue| !queue.is_empty())
            {
                ready |= ADAPTIVE_INTEREST_READABLE;
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, capture_lock);
        }
    }
    let injection_lock = unsafe { (*state).injection_lock };
    if !injection_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, injection_lock);
            if (*state).injection_queue.as_ref().is_some_and(|queue| {
                queue.state() == QueueState::Open && queue.len() < FRAME_QUEUE_LIMIT
            }) {
                ready |= ADAPTIVE_INTEREST_WRITABLE;
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, injection_lock);
        }
    }
    (ready, capture_generation, injection_generation)
}

/// Reads the single published adaptive-wait request slot.
///
/// The slot is claimed by registration before any wait state becomes visible
/// and is released only after the wait reaches a terminal outcome, so a
/// nonnull match identifies the request that the wait protocol owns.
fn wait_request_slot(state: *mut InstanceState) -> WDFREQUEST {
    if state.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { (*state).wait_request.load(Ordering::SeqCst).cast() }
}

fn wait_request_is_active(state: *mut InstanceState, request: WDFREQUEST) -> bool {
    !request.is_null() && wait_request_slot(state) == request
}

/// Atomically claims a published wait for a readiness transition.
///
/// This is the only wait operation reachable from `EVT_PACKET_QUEUE_ADVANCE`.
/// It performs a single atomic claim and schedules passive work; it never
/// acquires a lock and never calls a WDF request API, so TX and RX advance
/// remain lock-free with respect to adaptive waits and to each other.
///
/// The satisfied mask, the record state, and the registration sequence share
/// one atomic word, so the scheduling transition publishes the mask
/// indivisibly: the passive worker can never observe `WAIT_SCHEDULED` without
/// the mask, and a claimant that stalled across a retirement cannot contribute
/// to or schedule a later registration.
fn claim_wait_for_passive_completion(
    state: *mut InstanceState,
    condition: u32,
    expected_request: WDFREQUEST,
) -> bool {
    if state.is_null()
        || condition == 0
        || !unsafe { (*state).adaptive_enabled.load(Ordering::Acquire) }
    {
        return false;
    }
    let record = unsafe { &(*state).wait_state };
    loop {
        let observed = record.load(Ordering::SeqCst);
        let observed_state = wait_record_state(observed);
        if observed_state != WAIT_PENDING && observed_state != WAIT_SCHEDULED {
            return false;
        }
        let request = wait_request_slot(state);
        if request.is_null() || (!expected_request.is_null() && request != expected_request) {
            return false;
        }
        let satisfied = unsafe { (*state).wait_interest.load(Ordering::Acquire) } & condition;
        if satisfied == 0 {
            return false;
        }
        let next = wait_record_word(
            wait_record_sequence(observed),
            wait_record_satisfied(observed) | satisfied,
            WAIT_SCHEDULED,
        );
        if next == observed {
            // Already scheduled and the published mask already covers this
            // transition.
            return false;
        }
        if record
            .compare_exchange_weak(observed, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            continue;
        }
        if observed_state == WAIT_PENDING {
            enqueue_work_item(unsafe { (*state).read_work_item });
            return true;
        }
        // A concurrent transition already scheduled the worker; this claim only
        // contributed its readiness bits, preserving the multi-transition OR.
        return false;
    }
}

/// Retires the wait record and releases the registration slot last so the next
/// registration can only be admitted after every field is reset.
fn finish_wait(state: *mut InstanceState) {
    if state.is_null() {
        return;
    }
    unsafe {
        (*state).wait_interest.store(0, Ordering::Release);
        (*state)
            .wait_registration_capture_generation
            .store(0, Ordering::Release);
        (*state)
            .wait_registration_injection_generation
            .store(0, Ordering::Release);
        (*state)
            .wait_cancel_handoff
            .store(WAIT_HANDOFF_NONE, Ordering::Release);
        let record = &(*state).wait_state;
        let observed = record.load(Ordering::SeqCst);
        // Advancing the sequence while clearing the state and satisfied mask
        // rejects any claim still in flight against the retired record.
        record.store(
            wait_record_word(wait_record_sequence(observed) + 1, 0, WAIT_FREE),
            Ordering::SeqCst,
        );
        (*state)
            .wait_request
            .store(core::ptr::null_mut(), Ordering::SeqCst);
    }
}

/// Resolves WDF cancellation ownership for the exclusive `WAIT_UNMARKING`
/// owner and assigns terminal completion for the request exactly once.
///
/// Exactly one path reaches this function for a marked request. `STATUS_SUCCESS`
/// keeps completion here; `STATUS_CANCELLED` resolves the winner through the
/// handoff word so the request is completed once and `WdfRequestUnmarkCancelable`
/// is never called after the cancellation callback completed it; any other
/// status keeps completion here because the cancellation callback will not run.
/// On return the request always has exactly one terminal owner.
///
/// Every path that has already resolved ownership retires the wait record
/// before it completes the request, so the published slot cannot still name a
/// handle that WDF is free to recycle.
fn unmark_and_complete_wait(
    state: *mut InstanceState,
    request: WDFREQUEST,
    terminal_status: NTSTATUS,
    satisfied: u32,
) {
    let status = unsafe { call_unsafe_wdf_function_binding!(WdfRequestUnmarkCancelable, request) };
    if status == STATUS_SUCCESS {
        finish_wait(state);
        complete_wait_response(request, terminal_status, satisfied);
        return;
    }
    if status == STATUS_CANCELLED {
        let previous = unsafe {
            (*state)
                .wait_cancel_handoff
                .swap(WAIT_HANDOFF_UNMARK_CANCELLED, Ordering::SeqCst)
        };
        if previous == WAIT_HANDOFF_CANCEL_ARRIVED {
            // The cancellation callback already ran and handed completion here.
            finish_wait(state);
            complete_request(request, STATUS_CANCELLED);
        }
        // Otherwise the cancellation callback has not run yet and owns
        // terminal completion; this path must not touch the request again and
        // must leave the record published so that callback can resolve it.
        return;
    }
    // WDF rejected the unmark for an unexpected reason, so no cancellation
    // callback will own this request. Terminally complete it and retire the
    // wait so a later WAIT_FOR_CHANGE is not rejected as busy.
    debug_status(b"WdfRequestUnmarkCancelable", status);
    finish_wait(state);
    complete_request(request, status);
}

fn cancel_wait_for_teardown(state: *mut InstanceState) -> bool {
    cancel_wait_request_for_teardown(state, core::ptr::null_mut())
}

/// Claims a published wait on behalf of stop, cancel, power, or owner
/// teardown. Returns true only when this call assigned a terminal owner to
/// `expected_request`, so the caller must neither complete nor acknowledge it.
/// Every other outcome leaves the request with the registration, cancellation,
/// or passive-completion path that already owns its completion.
fn cancel_wait_request_for_teardown(
    state: *mut InstanceState,
    expected_request: WDFREQUEST,
) -> bool {
    if state.is_null() {
        return false;
    }
    loop {
        let observed = wait_record_state(load_wait_record(state));
        let published = wait_request_slot(state);
        if published.is_null() || (!expected_request.is_null() && published != expected_request) {
            return false;
        }
        match observed {
            WAIT_REGISTERING => {
                if transition_wait_record(state, WAIT_REGISTERING, WAIT_TEARDOWN).is_ok() {
                    // Registration has not finished marking the request, so it
                    // retains the unmark handshake and terminal completion.
                    return false;
                }
            }
            WAIT_PENDING | WAIT_SCHEDULED => {
                if transition_wait_record(state, observed, WAIT_UNMARKING).is_ok() {
                    // Re-read the slot only after winning the transition. The
                    // pre-check above is advisory; the wait record could have
                    // been retired and replaced between the load and this CAS.
                    let request = wait_request_slot(state);
                    if request.is_null() {
                        finish_wait(state);
                        return false;
                    }
                    unmark_and_complete_wait(state, request, STATUS_CANCELLED, 0);
                    return expected_request.is_null() || request == expected_request;
                }
            }
            _ => return false,
        }
    }
}

/// WDF cancellation callback body.
///
/// WDF invokes this only while the request is still marked cancelable, so this
/// path owns terminal completion unless an unmarking claimant explicitly takes
/// it through the handoff word.
fn cancel_wait_from_wdf(state: *mut InstanceState, request: WDFREQUEST) {
    if !wait_request_is_active(state, request) {
        complete_request(request, STATUS_CANCELLED);
        return;
    }
    loop {
        let observed = wait_record_state(load_wait_record(state));
        match observed {
            WAIT_REGISTERING | WAIT_TEARDOWN => {
                if transition_wait_record(state, observed, WAIT_CANCELLED).is_ok() {
                    // Registration observes WAIT_CANCELLED, retires the wait,
                    // and does not complete the request again. The record must
                    // stay published until registration retires it, so this
                    // path cannot reorder retirement before completion.
                    complete_request(request, STATUS_CANCELLED);
                    return;
                }
            }
            WAIT_PENDING | WAIT_SCHEDULED => {
                if transition_wait_record(state, observed, WAIT_CANCELLED).is_ok() {
                    // No claimant can reach WdfRequestUnmarkCancelable now, so
                    // ownership is resolved: retire the record before the
                    // handle can be completed and recycled.
                    finish_wait(state);
                    complete_request(request, STATUS_CANCELLED);
                    return;
                }
            }
            WAIT_UNMARKING => {
                let previous = unsafe {
                    (*state)
                        .wait_cancel_handoff
                        .swap(WAIT_HANDOFF_CANCEL_ARRIVED, Ordering::SeqCst)
                };
                if previous == WAIT_HANDOFF_UNMARK_CANCELLED {
                    // The claimant already observed STATUS_CANCELLED and handed
                    // completion to this callback.
                    finish_wait(state);
                    complete_request(request, STATUS_CANCELLED);
                }
                return;
            }
            _ => return,
        }
    }
}

fn complete_wait_response(request: WDFREQUEST, status: NTSTATUS, satisfied: u32) {
    if status != STATUS_SUCCESS {
        complete_request(request, status);
        return;
    }
    let mut output = core::ptr::null_mut::<c_void>();
    let mut output_length = 0usize;
    let output_status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveOutputBuffer,
            request,
            core::mem::size_of::<AdaptiveWaitResponse>(),
            &mut output,
            &mut output_length,
        )
    };
    if output_status != STATUS_SUCCESS
        || output_length < core::mem::size_of::<AdaptiveWaitResponse>()
    {
        complete_request(
            request,
            if output_status == STATUS_SUCCESS {
                STATUS_BUFFER_TOO_SMALL
            } else {
                output_status
            },
        );
        return;
    }
    unsafe {
        output
            .cast::<AdaptiveWaitResponse>()
            .write(AdaptiveWaitResponse { satisfied });
    }
    complete_request_with_information(
        request,
        STATUS_SUCCESS,
        core::mem::size_of::<AdaptiveWaitResponse>(),
    );
}

/// Completes a wait that a queue transition scheduled for passive completion.
///
/// The caller owns `WAIT_UNMARKING`, so this path performs the single unmark
/// handshake for the request.
fn complete_scheduled_wait_at_passive(
    state: *mut InstanceState,
    request: WDFREQUEST,
    satisfied: u32,
) {
    debug_assert!(at_passive_level());
    if state.is_null() {
        complete_request(request, STATUS_CANCELLED);
        return;
    }
    let cancelled = unsafe { (*state).lifecycle.load(Ordering::Acquire) } != INSTANCE_OPEN
        || !unsafe { (*state).rx_queue_started.load(Ordering::Acquire) };
    if cancelled {
        unmark_and_complete_wait(state, request, STATUS_CANCELLED, 0);
    } else {
        unmark_and_complete_wait(state, request, STATUS_SUCCESS, satisfied);
    }
}

/// Postpones a stopped request that no driver path owns.
///
/// The request is requeued so the framework redelivers it once the device
/// returns to D0; the driver keeps no reference to the handle afterwards.
fn acknowledge_stopped_request(request: WDFREQUEST) {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestStopAcknowledge, request, 1);
    }
}

/// Takes exclusive ownership of a wait that a queue transition scheduled.
///
/// The satisfied mask is read from the same atomic word that carried the
/// scheduling transition, so it is never observed before publication, and it is
/// refreshed from the current level-sensitive readiness so a transition that
/// publishes concurrently is never reported as an empty wakeup.
fn take_scheduled_wait(state: *mut InstanceState) -> Option<(WDFREQUEST, u32)> {
    if state.is_null() {
        return None;
    }
    let claimed = transition_wait_record(state, WAIT_SCHEDULED, WAIT_UNMARKING).ok()?;
    let request = wait_request_slot(state);
    if request.is_null() {
        finish_wait(state);
        return None;
    }
    let interest = unsafe { (*state).wait_interest.load(Ordering::Acquire) };
    let recorded = wait_record_satisfied(claimed);
    let (ready, _, _) = readiness_snapshot(state);
    Some((request, recorded | (ready & interest)))
}

fn has_queued_injection_frame(state: *mut InstanceState) -> bool {
    let lock = unsafe { (*state).injection_lock };
    if lock.is_null() {
        return false;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let has_frame = (*state)
            .injection_queue
            .as_ref()
            .is_some_and(|queue| !queue.is_empty());
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        has_frame
    }
}

fn clear_frame_queues(state: *mut InstanceState) {
    if state.is_null() {
        return;
    }
    let injection_lock = unsafe { (*state).injection_lock };
    if !injection_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, injection_lock);
            if let Some(queue) = (*state).injection_queue.as_mut() {
                queue.close();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, injection_lock);
            (*state)
                .injection_generation
                .fetch_add(1, Ordering::Release);
        }
    }
    let capture_lock = unsafe { (*state).capture_lock };
    if !capture_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, capture_lock);
            if let Some(queue) = (*state).capture_queue.as_mut() {
                queue.close();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, capture_lock);
            (*state).capture_generation.fetch_add(1, Ordering::Release);
        }
    }
}

fn reopen_frame_queues(state: *mut InstanceState) {
    if state.is_null() {
        return;
    }
    let injection_lock = unsafe { (*state).injection_lock };
    if !injection_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, injection_lock);
            if let Some(queue) = (*state).injection_queue.as_mut() {
                queue.reopen();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, injection_lock);
            (*state)
                .injection_generation
                .fetch_add(1, Ordering::Release);
        }
    }
    let capture_lock = unsafe { (*state).capture_lock };
    if !capture_lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, capture_lock);
            if let Some(queue) = (*state).capture_queue.as_mut() {
                queue.reopen();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, capture_lock);
            (*state).capture_generation.fetch_add(1, Ordering::Release);
        }
    }
}
