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
use frame_queue::{Frame, FrameQueue, QueueError, QueueState, FRAME_MAXIMUM, FRAME_STORAGE_SIZE};
use ring::{advance_index, fragment_at, fragment_virtual_address, increment_index, packet_at};

use core::alloc::Layout;
use core::ffi::c_void;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    DRIVER_OBJECT, GUID, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, STATUS_DEVICE_BUSY, ULONG,
    UNICODE_STRING, WDF_DRIVER_CONFIG, WDF_FILEOBJECT_CONFIG, WDF_IO_QUEUE_CONFIG,
    WDF_NO_OBJECT_ATTRIBUTES, WDF_OBJECT_ATTRIBUTES, WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_WORKITEM_CONFIG, WDFCMRESLIST, WDFDEVICE, WDFDEVICE_INIT, WDFDRIVER, WDFFILEOBJECT,
    WDFOBJECT, WDFQUEUE, WDFREQUEST, WDFSPINLOCK, WDFWORKITEM, call_unsafe_wdf_function_binding,
};

unsafe extern "C" {
    fn DbgPrintEx(component_id: ULONG, level: ULONG, format: *const i8, ...) -> ULONG;
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
    frame_lock: WDFSPINLOCK,
    frame_pool: wdk_sys::WDFLOOKASIDE,
    state_lock: WDFSPINLOCK,
    injection_queue: Option<FrameQueue>,
    capture_queue: Option<FrameQueue>,
    active_packet_filters: netadaptercx_sys::_NET_PACKET_FILTER_FLAGS,
    active_multicast_address_count: usize,
    active_multicast_addresses: [[u8; ETHERNET_ADDRESS_LENGTH]; MAXIMUM_MULTICAST_ADDRESSES],
    read_work_item: WDFWORKITEM,
    tx_queue: netadaptercx_sys::NETPACKETQUEUE,
    rx_queue: netadaptercx_sys::NETPACKETQUEUE,
    tx_rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    rx_rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    tx_fragment_extension: netadaptercx_sys::NET_EXTENSION,
    rx_fragment_extension: netadaptercx_sys::NET_EXTENSION,
    tx_queue_started: AtomicBool,
    rx_queue_started: AtomicBool,
    rx_notification_armed: AtomicBool,
    pending_reads: AtomicUsize,
    legacy_direct_read_claims: AtomicUsize,
    pending_writes: AtomicUsize,
    control_open: AtomicBool,
    adaptive_enabled: AtomicBool,
    pending_wait_request: WDFREQUEST,
    pending_wait_interest: u32,
    pending_wait_cancelable: bool,
    ready_wait_request: WDFREQUEST,
    ready_wait_satisfied: u32,
    lifecycle: core::sync::atomic::AtomicU8,
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
            frame_lock: core::ptr::null_mut(),
            frame_pool: core::ptr::null_mut(),
            state_lock: core::ptr::null_mut(),
            injection_queue: None,
            capture_queue: None,
            active_packet_filters: 0,
            active_multicast_address_count: 0,
            active_multicast_addresses: [[0; ETHERNET_ADDRESS_LENGTH]; MAXIMUM_MULTICAST_ADDRESSES],
            read_work_item: core::ptr::null_mut(),
            tx_queue: core::ptr::null_mut(),
            rx_queue: core::ptr::null_mut(),
            tx_rings: core::ptr::null(),
            rx_rings: core::ptr::null(),
            tx_fragment_extension: netadaptercx_sys::NET_EXTENSION::default(),
            rx_fragment_extension: netadaptercx_sys::NET_EXTENSION::default(),
            tx_queue_started: AtomicBool::new(false),
            rx_queue_started: AtomicBool::new(false),
            rx_notification_armed: AtomicBool::new(false),
            pending_reads: AtomicUsize::new(0),
            legacy_direct_read_claims: AtomicUsize::new(0),
            pending_writes: AtomicUsize::new(0),
            control_open: AtomicBool::new(false),
            adaptive_enabled: AtomicBool::new(false),
            pending_wait_request: core::ptr::null_mut(),
            pending_wait_interest: 0,
            pending_wait_cancelable: false,
            ready_wait_request: core::ptr::null_mut(),
            ready_wait_satisfied: 0,
            lifecycle: core::sync::atomic::AtomicU8::new(INSTANCE_OPEN),
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
    started: bool,
    _padding: [u8; 5],
    instance: *mut InstanceState,
    rings: netadaptercx_sys::NET_RING_COLLECTION,
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

unsafe fn instance_from_packet_queue(
    queue: netadaptercx_sys::NETPACKETQUEUE,
) -> Option<*mut InstanceState> {
    let context =
        unsafe { object_context::<QueueContext>(queue.cast(), &raw const QUEUE_CONTEXT_TYPE_INFO) };
    if context.is_null() || unsafe { (*context).instance.is_null() } {
        None
    } else {
        Some(unsafe { (*context).instance })
    }
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
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
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
        unsafe {
            (*queue_context).is_transmit = is_transmit;
            (*queue_context).instance = state;
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
        // Queue-ring discovery is a PASSIVE_LEVEL NetAdapterCx operation.
        // InstanceStateGuard owns a WDF spin lock and therefore cannot cover it.
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let state = &mut *state_guard;
        if is_transmit {
            state.tx_queue = packet_queue;
            state.tx_rings = rings;
            state.tx_fragment_extension = extension;
        } else {
            state.rx_queue = packet_queue;
            state.rx_rings = rings;
            state.rx_fragment_extension = extension;
        }
    }
    status
}

extern "C" fn evt_packet_queue_start(queue: netadaptercx_sys::NETPACKETQUEUE) {
    if let Some(state) = unsafe { instance_from_packet_queue(queue) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        if queue == state.tx_queue {
            state.tx_queue_started.store(true, Ordering::Release);
        } else if queue == state.rx_queue {
            state.rx_queue_started.store(true, Ordering::Release);
        }
    }
}

extern "C" fn evt_packet_queue_stop(queue: netadaptercx_sys::NETPACKETQUEUE) {
    if let Some(state) = unsafe { instance_from_packet_queue(queue) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        let pending_wait = take_wait_for_cancellation_locked(state);
        if queue == state.tx_queue {
            state.tx_queue_started.store(false, Ordering::Release);
        } else if queue == state.rx_queue {
            state.rx_queue_started.store(false, Ordering::Release);
            state.rx_notification_armed.store(false, Ordering::Release);
        }
        drop(state_guard);
        if let Some(request) = pending_wait {
            cancel_claimed_wait(request);
        }
    }
}

extern "C" fn evt_packet_queue_advance(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let Some(state) = (unsafe { instance_from_packet_queue(queue) }) else {
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return;
    };
    let (
        is_transmit,
        tx_rings,
        tx_extension,
        read_queue,
        read_work_item,
        wait_completion_scheduled,
    ) = {
        let state_ref = &mut *state_guard;
        let rx_rings = state_ref.rx_rings;
        let rx_extension = state_ref.rx_fragment_extension;
        let wait_completion_scheduled = if queue == state_ref.rx_queue && !rx_rings.is_null() {
            inject_receive_frames(state_ref, rx_rings, &rx_extension)
        } else {
            false
        };
        (
            queue == state_ref.tx_queue && !state_ref.tx_rings.is_null(),
            state_ref.tx_rings,
            state_ref.tx_fragment_extension,
            state_ref.read_queue,
            state_ref.read_work_item,
            wait_completion_scheduled,
        )
    };
    drop(state_guard);
    if wait_completion_scheduled {
        enqueue_work_item(read_work_item);
    }
    if is_transmit {
        capture_transmit_packets(state, read_queue, read_work_item, tx_rings, &tx_extension);
    }
}

fn inject_receive_frames(
    state: &mut InstanceState,
    rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    extension: &netadaptercx_sys::NET_EXTENSION,
) -> bool {
    let (packet_ring, fragment_ring) = unsafe {
        let collection = match rings.as_ref() {
            Some(collection) => collection,
            None => return false,
        };
        (
            collection.Rings[ring::PACKET_RING_INDEX],
            collection.Rings[ring::FRAGMENT_RING_INDEX],
        )
    };
    if packet_ring.is_null() || fragment_ring.is_null() {
        return false;
    }

    let mut wait_completion_scheduled = false;
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
                let _ = enqueue_existing_injection_frame(state, frame);
                break;
            }
        };
        let fragment = match unsafe { fragment_at(fragment_ring, fragment_begin) } {
            Some(fragment) => unsafe { &mut *fragment },
            None => {
                let _ = enqueue_existing_injection_frame(state, frame);
                break;
            }
        };
        let address = match unsafe { fragment_virtual_address(extension, fragment_begin) } {
            Some(address) => unsafe { &*address },
            None => {
                let _ = enqueue_existing_injection_frame(state, frame);
                break;
            }
        };
        if address.VirtualAddress.is_null() {
            let _ = enqueue_existing_injection_frame(state, frame);
            break;
        }

        // RX descriptors are reused by NetAdapterCx. Do not inherit a prior
        // frame's byte offset or valid length when indicating this frame.
        fragment.set_Offset(0);
        let frame_length = frame.as_bytes().len();
        let capacity = fragment.Capacity() as usize;
        if frame_length > capacity {
            let _ = enqueue_existing_injection_frame(state, frame);
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
            wait_completion_scheduled |=
                claim_wait_for_passive_completion_locked(state, ADAPTIVE_INTEREST_WRITABLE);
        }
    }
    wait_completion_scheduled
}

fn capture_transmit_packets(
    state: *mut InstanceState,
    read_queue: WDFQUEUE,
    read_work_item: WDFWORKITEM,
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

    let mut captured_for_legacy_read = false;
    let mut wait_completion_scheduled = false;
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
            Some(packet) => unsafe { &*packet },
            None => break,
        };
        let fragment_count = packet.FragmentCount as u32;
        if fragment_count == 0 {
            break;
        }
        let fragment_begin = packet.FragmentIndex;
        if fragment_begin == fragment_end
            || fragment_count > unsafe { (*fragment_ring).NumberOfElements }
        {
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
            if !validate_fragment(fragment, address, &mut total_length, unsafe {
                (*state).frame_maximum
            }) {
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
        if !valid || !(FRAME_MINIMUM..=unsafe { (*state).frame_maximum }).contains(&total_length) {
            break;
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

        let pool = match unsafe { state.as_ref() }.map(|state| state.frame_pool) {
            Some(pool) if !pool.is_null() => pool,
            _ => {
                debug_status(b"Tx capture frame pool", STATUS_DEVICE_NOT_READY);
                break;
            }
        };
        let mut frame = match Frame::new(pool) {
            Ok(frame) => frame,
            Err(_) => {
                debug_status(b"Tx capture frame allocation", STATUS_INSUFFICIENT_RESOURCES);
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
            let start = unsafe {
                (address.VirtualAddress as *const u8).add(fragment.Offset() as usize)
            };
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
            if let Some(mut state_guard) = unsafe { InstanceStateGuard::new(state) } {
                let adaptive = state_guard.adaptive_enabled.load(Ordering::Acquire);
                match enqueue_existing_capture_frame_locked(&mut state_guard, frame) {
                    Ok(wait_completion_queued) => {
                        if !adaptive {
                            captured_for_legacy_read = true;
                        }
                        wait_completion_scheduled |= wait_completion_queued;
                    }
                    Err(_) => {}
                }
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

    if captured_for_legacy_read || wait_completion_scheduled {
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
) -> bool {
    let Some(state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return false;
    };
    if state_guard.adaptive_enabled.load(Ordering::Acquire)
        || state_guard.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN
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
    release_request(&state_guard.pending_reads);
    state_guard
        .legacy_direct_read_claims
        .fetch_add(1, Ordering::AcqRel);
    drop(state_guard);

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
    if let Some(state) = unsafe { instance_from_packet_queue(queue) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        if queue != state.rx_queue {
            return;
        }
        state
            .rx_notification_armed
            .store(enabled != 0, Ordering::Release);
        let notification_queue = if enabled != 0 && has_queued_injection_frame(state) {
            take_rx_notification(state)
        } else {
            core::ptr::null_mut()
        };
        drop(state_guard);
        if !notification_queue.is_null() {
            notify_more_received_packets(notification_queue);
        }
    }
}

extern "C" fn evt_packet_queue_cancel(queue: netadaptercx_sys::NETPACKETQUEUE) {
    let Some(state) = (unsafe { instance_from_packet_queue(queue) }) else {
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return;
    };
    let state = &mut *state_guard;
    let (rings, is_receive) = if queue == state.tx_queue {
        (state.tx_rings, false)
    } else if queue == state.rx_queue {
        (state.rx_rings, true)
    } else {
        return;
    };

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
        state.rx_notification_armed.store(false, Ordering::Release);
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
    let lock = state.frame_lock;
    if lock.is_null() {
        debug_status(b"ReceiveFilter state lock", STATUS_DEVICE_NOT_READY);
        return;
    }

    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        state.active_packet_filters = packet_filters;
        state.active_multicast_address_count = multicast_addresses.len();
        for (index, address) in multicast_addresses.iter().enumerate() {
            state.active_multicast_addresses[index] = *address;
        }
        for index in multicast_addresses.len()..MAXIMUM_MULTICAST_ADDRESSES {
            state.active_multicast_addresses[index] = [0; ETHERNET_ADDRESS_LENGTH];
        }
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
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
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let state = &mut *state_guard;
        reopen_frame_queues(state);
        state.lifecycle.store(INSTANCE_OPEN, Ordering::Release);
    }
    STATUS_SUCCESS
}

unsafe extern "C" fn evt_device_d0_exit(
    device: WDFDEVICE,
    _target_state: wdk_sys::WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    if let Some(state) = unsafe { instance_from_pnp_device(device) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let (read_queue, adaptive, pending_wait) = {
            let state = &mut *state_guard;
            state.lifecycle.store(INSTANCE_SUSPENDED, Ordering::Release);
            (
                state.read_queue,
                state.adaptive_enabled.load(Ordering::Acquire),
                take_wait_for_cancellation_locked(state),
            )
        };
        drop(state_guard);
        if let Some(request) = pending_wait {
            cancel_claimed_wait(request);
        }
        if !adaptive {
            purge_queue(read_queue);
        }
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return STATUS_DEVICE_NOT_READY;
        };
        let state = &mut *state_guard;
        clear_frame_queues(state);
        state.pending_reads.store(0, Ordering::Release);
        state.pending_writes.store(0, Ordering::Release);
        state.rx_notification_armed.store(false, Ordering::Release);
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
    let (adapter, read_queue, read_work_item, adaptive, pending_wait) = {
        let state = &mut *state_guard;
        state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
        (
            state.adapter,
            state.read_queue,
            state.read_work_item,
            state.adaptive_enabled.load(Ordering::Acquire),
            take_wait_for_cancellation_locked(state),
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

    if let Some(request) = pending_wait {
        cancel_claimed_wait(request);
    }
    if !adaptive {
        purge_queue(read_queue);
    }
    flush_work_item(read_work_item);
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let state = &mut *state_guard;
    clear_frame_queues(state);
    // The lookaside list is parented to the device; leave it alive until WDF
    // tears down the device so any in-flight Frame can release its memory.
    clear_receive_filter_state(state);
    state.pending_reads.store(0, Ordering::Release);
    state.pending_writes.store(0, Ordering::Release);
    state.tx_queue = core::ptr::null_mut();
    state.rx_queue = core::ptr::null_mut();
    state.tx_rings = core::ptr::null();
    state.rx_rings = core::ptr::null();
    state.tx_fragment_extension = netadaptercx_sys::NET_EXTENSION::default();
    state.rx_fragment_extension = netadaptercx_sys::NET_EXTENSION::default();
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
            wdk_sys::_POOL_TYPE::NonPagedPool,
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
    let mut frame_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockCreate, &mut lock_attributes, &mut frame_lock,)
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
    let mut control_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut control_queue_config,
            WDF_NO_OBJECT_ATTRIBUTES,
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
    state.frame_lock = frame_lock;
    state.frame_pool = frame_pool;
    state.state_lock = state_lock;
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
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let (was_suspended, read_queue, adaptive, pending_wait) = {
            let state = &mut *state_guard;
            let was_suspended = state.lifecycle.load(Ordering::Acquire) == INSTANCE_SUSPENDED;
            state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
            (
                was_suspended,
                state.read_queue,
                state.adaptive_enabled.load(Ordering::Acquire),
                take_wait_for_cancellation_locked(state),
            )
        };
        drop(state_guard);
        if let Some(request) = pending_wait {
            cancel_claimed_wait(request);
        }
        if !adaptive {
            purge_queue(read_queue);
        }
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let should_resume = {
            let state = &mut *state_guard;
            clear_frame_queues(state);
            state.pending_reads.store(0, Ordering::Release);
            state.pending_writes.store(0, Ordering::Release);
            !state.adapter.is_null() && !was_suspended
        };
        drop(state_guard);
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        if should_resume
            && state.lifecycle.load(Ordering::Acquire) == INSTANCE_CLOSING
            && !state.adapter.is_null()
        {
            reopen_frame_queues(state);
            state.lifecycle.store(INSTANCE_OPEN, Ordering::Release);
        }
        state.adaptive_enabled.store(false, Ordering::Release);
        state.control_open.store(false, Ordering::Release);
        drop(state_guard);
        if should_resume {
            // A reopened manual queue belongs to a future legacy owner, never to this adaptive owner.
            resume_manual_queue(read_queue);
        }
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

    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if state_guard.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN {
        drop(state_guard);
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if !state_guard.adaptive_enabled.load(Ordering::Acquire) {
        drop(state_guard);
        complete_request(request, STATUS_INVALID_DEVICE_REQUEST);
        return;
    }
    if !state_guard.rx_queue_started.load(Ordering::Acquire) {
        drop(state_guard);
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if !state_guard.pending_wait_request.is_null() || !state_guard.ready_wait_request.is_null() {
        drop(state_guard);
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }

    let satisfied = readiness_mask_locked(&mut state_guard) & wait.interest;
    if satisfied != 0 {
        drop(state_guard);
        complete_wait_response(request, STATUS_SUCCESS, satisfied);
        return;
    }

    state_guard.pending_wait_request = request;
    state_guard.pending_wait_interest = wait.interest;
    state_guard.pending_wait_cancelable = false;
    drop(state_guard);

    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestMarkCancelableEx,
            request,
            Some(evt_wait_for_change_cancel),
        )
    };
    let mut complete_cancelled = status != STATUS_SUCCESS;
    let mut work_item = core::ptr::null_mut();
    if status == STATUS_SUCCESS {
        if let Some(mut state_guard) = unsafe { InstanceStateGuard::new(state) } {
            if state_guard.pending_wait_request == request {
                state_guard.pending_wait_cancelable = true;
                if readiness_mask_locked(&mut state_guard) & wait.interest != 0 {
                    if claim_wait_for_passive_completion_locked(
                        &mut state_guard,
                        wait.interest,
                    ) {
                        work_item = state_guard.read_work_item;
                    }
                }
            }
        }
        complete_cancelled = false;
    } else if let Some(mut state_guard) = unsafe { InstanceStateGuard::new(state) } {
        if state_guard.pending_wait_request == request {
            state_guard.pending_wait_request = core::ptr::null_mut();
            state_guard.pending_wait_interest = 0;
            state_guard.pending_wait_cancelable = false;
        }
    }
    if complete_cancelled {
        complete_request(request, STATUS_CANCELLED);
    } else if !work_item.is_null() {
        enqueue_work_item(work_item);
    }
}

unsafe extern "C" fn evt_wait_for_change_cancel(request: WDFREQUEST) {
    let queue = unsafe { call_unsafe_wdf_function_binding!(WdfRequestGetIoQueue, request) };
    if let Some(state) = unsafe { instance_from_io_queue(queue) } {
        if let Some(mut state_guard) = unsafe { InstanceStateGuard::new(state) } {
            let _ = take_wait_request_locked(&mut state_guard, request);
        }
    }
    complete_request(request, STATUS_CANCELLED);
}

extern "C" fn evt_io_read(_queue: WDFQUEUE, request: WDFREQUEST, _length: usize) {
    let Some(state) = (unsafe { instance_from_io_queue(_queue) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    if state_guard.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if state_guard.adaptive_enabled.load(Ordering::Acquire) {
        let frame = dequeue_capture_frame(&mut state_guard);
        drop(state_guard);
        if let Some(frame) = frame {
            let status = complete_captured_frame_to_read(request, &frame);
            if status != STATUS_SUCCESS {
                requeue_capture_frame_and_schedule_wait(state, frame);
                complete_request(request, status);
            }
        } else {
            complete_request(request, STATUS_NO_MORE_ENTRIES);
        }
        return;
    }
    if !try_admit(&state_guard.pending_reads, PENDING_READ_LIMIT) {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let frame = dequeue_capture_frame(&mut *state_guard);
    if let Some(frame) = frame {
        release_request(&state_guard.pending_reads);
        drop(state_guard);

        let status = complete_captured_frame_to_read(request, &frame);
        if status != STATUS_SUCCESS {
            requeue_capture_frame_and_schedule_wait(state, frame);
            complete_request(request, status);
        }
        return;
    }
    let target = state_guard.read_queue;
    if !forward_request(request, target) {
        release_request(&state_guard.pending_reads);
    }
}

extern "C" fn evt_io_write(_queue: WDFQUEUE, request: WDFREQUEST, length: usize) {
    let Some(state) = (unsafe { instance_from_io_queue(_queue) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    let state = &mut *state_guard;
    if state.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if length == 0 {
        complete_request(request, STATUS_SUCCESS);
        return;
    }
    if !(FRAME_MINIMUM..=state.frame_maximum).contains(&length) {
        complete_request(request, STATUS_INVALID_PARAMETER);
        return;
    }
    if !try_admit(&state.pending_writes, PENDING_WRITE_LIMIT) {
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
        release_request(&state.pending_writes);
        complete_request(request, status);
        return;
    }
    if input_length > state.frame_maximum {
        release_request(&state.pending_writes);
        complete_request(request, STATUS_INVALID_BUFFER_SIZE);
        return;
    }

    let bytes = unsafe { core::slice::from_raw_parts(input.cast::<u8>(), input_length) };
    let notification_queue = match enqueue_injection_frame(state, bytes) {
        Ok(()) => {
            let notification_queue = take_rx_notification(state);
            release_request(&state.pending_writes);
            complete_request_with_information(request, STATUS_SUCCESS, input_length);
            notification_queue
        }
        Err(QueueError::Full) => {
            release_request(&state.pending_writes);
            complete_request(request, STATUS_DEVICE_BUSY);
            core::ptr::null_mut()
        }
        Err(QueueError::Closed) => {
            release_request(&state.pending_writes);
            complete_request(request, STATUS_DEVICE_NOT_READY);
            core::ptr::null_mut()
        }
        Err(QueueError::InvalidFrameLength) => {
            release_request(&state.pending_writes);
            complete_request(request, STATUS_INVALID_BUFFER_SIZE);
            core::ptr::null_mut()
        }
        Err(QueueError::InsufficientResources) => {
            release_request(&state.pending_writes);
            complete_request(request, STATUS_INSUFFICIENT_RESOURCES);
            core::ptr::null_mut()
        }
    };
    drop(state_guard);
    if !notification_queue.is_null() {
        notify_more_received_packets(notification_queue);
    }
}

extern "C" fn evt_io_stop(queue: WDFQUEUE, request: WDFREQUEST, _action_flags: ULONG) {
    let Some(state) = (unsafe { instance_from_io_queue(queue) }) else {
        complete_request(request, STATUS_CANCELLED);
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        complete_request(request, STATUS_CANCELLED);
        return;
    };
    let state = &mut *state_guard;
    let is_wait = take_wait_request_locked(state, request);
    if !is_wait && queue == state.read_queue {
        release_request(&state.pending_reads);
    }
    drop(state_guard);
    if is_wait {
        cancel_claimed_wait(request);
        return;
    }
    complete_request(request, STATUS_CANCELLED);
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

fn take_rx_notification(state: &InstanceState) -> netadaptercx_sys::NETPACKETQUEUE {
    if state.rx_notification_armed.swap(false, Ordering::AcqRel) {
        state.rx_queue
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
        let ready_wait = {
            let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return;
            };
            take_ready_wait_locked(&mut state_guard)
        };
        if let Some((request, satisfied)) = ready_wait {
            complete_claimed_wait_at_passive(request, satisfied);
            continue;
        }

        let mut request = core::ptr::null_mut();
        let frame = {
            let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
                return;
            };
            let state = &mut *state_guard;
            let status = unsafe {
                call_unsafe_wdf_function_binding!(
                    WdfIoQueueRetrieveNextRequest,
                    state.read_queue,
                    &mut request,
                )
            };
            if status != STATUS_SUCCESS {
                return;
            }
            let frame = match dequeue_capture_frame(state) {
                Some(frame) => frame,
                None => {
                    let target = state.read_queue;
                    if !forward_request(request, target) {
                        release_request(&state.pending_reads);
                    }
                    return;
                }
            };
            release_request(&state.pending_reads);
            frame
        };
        let status = complete_captured_frame_to_read(request, &frame);
        if status != STATUS_SUCCESS {
            requeue_capture_frame_and_schedule_wait(state, frame);
            complete_request(request, status);
        }
    }
}

fn enqueue_injection_frame(state: &mut InstanceState, bytes: &[u8]) -> Result<(), QueueError> {
    let frame = Frame::from_bytes(state.frame_pool, bytes)?;
    enqueue_existing_injection_frame(state, frame)
}

fn enqueue_existing_injection_frame(
    state: &mut InstanceState,
    frame: Frame,
) -> Result<(), QueueError> {
    let lock = state.frame_lock;
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let result = state
            .injection_queue
            .as_mut()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.enqueue(frame));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        result
    }
}

fn enqueue_existing_capture_frame_locked(
    state: &mut InstanceState,
    frame: Frame,
) -> Result<bool, QueueError> {
    let lock = state.frame_lock;
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let (result, was_empty) = match state.capture_queue.as_mut() {
            Some(queue) => {
                let was_empty = queue.is_empty();
                (queue.enqueue(frame), was_empty)
            }
            None => (Err(QueueError::Closed), false),
        };
        let wait_completion_scheduled = if result.is_ok() && was_empty {
            claim_wait_for_passive_completion_locked(state, ADAPTIVE_INTEREST_READABLE)
        } else {
            false
        };
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        result.map(|()| wait_completion_scheduled)
    }
}

fn dequeue_injection_frame(state: &mut InstanceState) -> (Option<Frame>, bool) {
    let lock = state.frame_lock;
    if lock.is_null() {
        return (None, false);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let (frame, was_full) = match state.injection_queue.as_mut() {
            Some(queue) => {
                let was_full =
                    queue.state() == QueueState::Open && queue.len() == FRAME_QUEUE_LIMIT;
                (queue.dequeue(), was_full)
            }
            None => (None, false),
        };
        let was_full_transition = frame.is_some() && was_full;
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        (frame, was_full_transition)
    }
}

fn requeue_capture_frame_and_schedule_wait(state: *mut InstanceState, frame: Frame) {
    let work_item = if let Some(mut state_guard) = unsafe { InstanceStateGuard::new(state) } {
        if enqueue_existing_capture_frame_locked(&mut state_guard, frame).unwrap_or(false) {
            state_guard.read_work_item
        } else {
            core::ptr::null_mut()
        }
    } else {
        core::ptr::null_mut()
    };
    enqueue_work_item(work_item);
}

fn dequeue_capture_frame(state: &mut InstanceState) -> Option<Frame> {
    let lock = state.frame_lock;
    if lock.is_null() {
        return None;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let frame = state.capture_queue.as_mut().and_then(FrameQueue::dequeue);
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        frame
    }
}

fn readiness_mask_locked(state: &mut InstanceState) -> u32 {
    let lock = state.frame_lock;
    if lock.is_null() {
        return 0;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let mut ready = 0;
        if state
            .capture_queue
            .as_ref()
            .is_some_and(|queue| !queue.is_empty())
        {
            ready |= ADAPTIVE_INTEREST_READABLE;
        }
        if state.injection_queue.as_ref().is_some_and(|queue| {
            queue.state() == QueueState::Open && queue.len() < FRAME_QUEUE_LIMIT
        }) {
            ready |= ADAPTIVE_INTEREST_WRITABLE;
        }
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        ready
    }
}

fn claim_wait_for_passive_completion_locked(state: &mut InstanceState, condition: u32) -> bool {
    if !state.adaptive_enabled.load(Ordering::Acquire)
        || state.pending_wait_request.is_null()
        || !state.pending_wait_cancelable
        || !state.ready_wait_request.is_null()
    {
        return false;
    }
    let satisfied = state.pending_wait_interest & condition;
    if satisfied == 0 {
        return false;
    }
    let request = state.pending_wait_request;
    state.pending_wait_request = core::ptr::null_mut();
    state.pending_wait_interest = 0;
    state.pending_wait_cancelable = false;
    state.ready_wait_request = request;
    state.ready_wait_satisfied = satisfied;
    true
}

fn take_wait_for_cancellation_locked(state: &mut InstanceState) -> Option<WDFREQUEST> {
    if !state.pending_wait_request.is_null() {
        let request = state.pending_wait_request;
        state.pending_wait_request = core::ptr::null_mut();
        state.pending_wait_interest = 0;
        state.pending_wait_cancelable = false;
        return Some(request);
    }
    take_ready_wait_locked(state).map(|(request, _)| request)
}

fn take_ready_wait_locked(state: &mut InstanceState) -> Option<(WDFREQUEST, u32)> {
    if state.ready_wait_request.is_null() {
        return None;
    }
    let request = state.ready_wait_request;
    let satisfied = state.ready_wait_satisfied;
    state.ready_wait_request = core::ptr::null_mut();
    state.ready_wait_satisfied = 0;
    Some((request, satisfied))
}

fn take_wait_request_locked(state: &mut InstanceState, request: WDFREQUEST) -> bool {
    if state.pending_wait_request == request {
        state.pending_wait_request = core::ptr::null_mut();
        state.pending_wait_interest = 0;
        state.pending_wait_cancelable = false;
        return true;
    }
    if state.ready_wait_request == request {
        state.ready_wait_request = core::ptr::null_mut();
        state.ready_wait_satisfied = 0;
        return true;
    }
    false
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

fn complete_claimed_wait_at_passive(request: WDFREQUEST, satisfied: u32) {
    debug_assert!(at_passive_level());
    let cancel_status =
        unsafe { call_unsafe_wdf_function_binding!(WdfRequestUnmarkCancelable, request) };
    if cancel_status == STATUS_SUCCESS {
        complete_wait_response(request, STATUS_SUCCESS, satisfied);
    } else if cancel_status != STATUS_CANCELLED {
        complete_request(request, cancel_status);
    }
}

fn cancel_claimed_wait(request: WDFREQUEST) {
    let cancel_status =
        unsafe { call_unsafe_wdf_function_binding!(WdfRequestUnmarkCancelable, request) };
    if cancel_status == STATUS_SUCCESS {
        complete_request(request, STATUS_CANCELLED);
    } else if cancel_status != STATUS_CANCELLED {
        complete_request(request, cancel_status);
    }
}

fn has_queued_injection_frame(state: &mut InstanceState) -> bool {
    let lock = state.frame_lock;
    if lock.is_null() {
        return false;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let has_frame = state
            .injection_queue
            .as_ref()
            .is_some_and(|queue| !queue.is_empty());
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        has_frame
    }
}

fn clear_frame_queues(state: &mut InstanceState) {
    let lock = state.frame_lock;
    if !lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
            if let Some(queue) = state.injection_queue.as_mut() {
                queue.close();
            }
            if let Some(queue) = state.capture_queue.as_mut() {
                queue.close();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        }
    }
}

fn reopen_frame_queues(state: &mut InstanceState) {
    let lock = state.frame_lock;
    if !lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
            if let Some(queue) = state.injection_queue.as_mut() {
                queue.reopen();
            }
            if let Some(queue) = state.capture_queue.as_mut() {
                queue.reopen();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        }
    }
}
