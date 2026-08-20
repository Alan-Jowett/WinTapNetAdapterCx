#![no_std]

extern crate alloc;

use alloc::alloc::alloc;
use alloc::boxed::Box;
use alloc::vec::Vec;

#[cfg(not(test))]
extern crate wdk_panic;

mod frame_queue;
mod ring;
use frame_queue::{Frame, FrameQueue, QueueError};
use ring::{advance_index, fragment_at, fragment_virtual_address, increment_index, packet_at};

use core::alloc::Layout;
use core::ffi::c_void;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    DRIVER_OBJECT, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, ULONG, UNICODE_STRING,
    WDF_DRIVER_CONFIG, WDF_FILEOBJECT_CONFIG, WDF_IO_QUEUE_CONFIG,
    WDF_NO_OBJECT_ATTRIBUTES, WDF_OBJECT_ATTRIBUTES, WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_WORKITEM_CONFIG, WDFCMRESLIST, WDFDEVICE, WDFDEVICE_INIT, WDFDRIVER, WDFFILEOBJECT,
    WDFOBJECT, WDFQUEUE, WDFREQUEST, WDFSPINLOCK, WDFWORKITEM,
    call_unsafe_wdf_function_binding,
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
        DbgPrintEx(
            DPFLTR_IHVDRIVER_ID,
            0,
            format.as_ptr(),
            status as u32,
        );
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
const STATUS_DEVICE_BUSY: NTSTATUS = 0xC000_00E8_u32 as i32;
const STATUS_DEVICE_NOT_READY: NTSTATUS = 0xC000_00A3_u32 as i32;
const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009A_u32 as i32;
const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000D_u32 as i32;
const STATUS_INVALID_BUFFER_SIZE: NTSTATUS = 0xC000_0206_u32 as i32;
const STATUS_NOT_SUPPORTED: NTSTATUS = 0xC000_00BB_u32 as i32;
const INSTANCE_OPEN: u8 = 0;
const INSTANCE_SUSPENDED: u8 = 1;
const INSTANCE_CLOSING: u8 = 2;
const INSTANCE_CLOSED: u8 = 3;
const PENDING_READ_LIMIT: usize = 256;
const PENDING_WRITE_LIMIT: usize = 256;
const FRAME_QUEUE_LIMIT: usize = 256;
const FRAME_MINIMUM: usize = 14;
const FRAME_MAXIMUM: usize = 1514;
const MAXIMUM_MULTICAST_ADDRESSES: usize = 64;
const ETHERNET_ADDRESS_LENGTH: usize = 6;
const CONTROL_SDDL: [u16; 16] = [
    b'D' as u16,
    b':' as u16,
    b'P' as u16,
    b'(' as u16,
    b'A' as u16,
    b';' as u16,
    b';' as u16,
    b'G' as u16,
    b'A' as u16,
    b';' as u16,
    b';' as u16,
    b';' as u16,
    b'B' as u16,
    b'A' as u16,
    b')' as u16,
    0,
];

static INSTANCE_IDS: [AtomicBool; 2] =
    [const { AtomicBool::new(false) }; 2];
static INSTANCE_STATES: [core::sync::atomic::AtomicPtr<InstanceState>; 2] =
    [const { core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()) }; 2];
static FRAGMENT_VIRTUAL_ADDRESS_NAME: [u16; 27] = [
    109, 115, 95, 102, 114, 97, 103, 109, 101, 110, 116, 95, 118, 105, 114, 116, 117, 97, 108,
    97, 100, 100, 114, 101, 115, 115, 0,
];
static QUEUE_CONTEXT_NAME: &[u8] = b"WINTAP_QUEUE_CONTEXT\0";
static DEVICE_CONTEXT_NAME: &[u8] = b"WINTAP_DEVICE_CONTEXT\0";

#[repr(C)]
struct InstanceState {
    instance_id: usize,
    pnp_device: WDFDEVICE,
    adapter: netadaptercx_sys::NETADAPTER,
    control_device: WDFDEVICE,
    read_queue: WDFQUEUE,
    write_queue: WDFQUEUE,
    frame_lock: WDFSPINLOCK,
    state_lock: WDFSPINLOCK,
    injection_queue: Option<FrameQueue>,
    capture_queue: Option<FrameQueue>,
    active_packet_filters: netadaptercx_sys::_NET_PACKET_FILTER_FLAGS,
    active_multicast_address_count: usize,
    active_multicast_addresses: [[u8; ETHERNET_ADDRESS_LENGTH];
        MAXIMUM_MULTICAST_ADDRESSES],
    read_work_item: WDFWORKITEM,
    write_work_item: WDFWORKITEM,
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
    pending_writes: AtomicUsize,
    lifecycle: core::sync::atomic::AtomicU8,
}

impl InstanceState {
    fn new(instance_id: usize) -> Self {
        Self {
            instance_id,
            pnp_device: core::ptr::null_mut(),
            adapter: core::ptr::null_mut(),
            control_device: core::ptr::null_mut(),
            read_queue: core::ptr::null_mut(),
            write_queue: core::ptr::null_mut(),
            frame_lock: core::ptr::null_mut(),
            state_lock: core::ptr::null_mut(),
            injection_queue: None,
            capture_queue: None,
            active_packet_filters: 0,
            active_multicast_address_count: 0,
            active_multicast_addresses: [[0; ETHERNET_ADDRESS_LENGTH];
                MAXIMUM_MULTICAST_ADDRESSES],
            read_work_item: core::ptr::null_mut(),
            write_work_item: core::ptr::null_mut(),
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
            pending_writes: AtomicUsize::new(0),
            lifecycle: core::sync::atomic::AtomicU8::new(INSTANCE_OPEN),
        }
    }
}

fn allocate_instance_state(instance_id: usize) -> *mut InstanceState {
    let layout = Layout::new::<InstanceState>();
    let state = unsafe {
        // SAFETY: The layout exactly describes the InstanceState allocation.
        alloc(layout).cast::<InstanceState>()
    };
    if !state.is_null() {
        unsafe {
            // SAFETY: The allocation is uniquely owned and properly aligned for InstanceState.
            state.write(InstanceState::new(instance_id));
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

struct InstanceRegistry {
    adapter: core::sync::atomic::AtomicPtr<c_void>,
    state: core::sync::atomic::AtomicPtr<InstanceState>,
}

impl InstanceRegistry {
    const fn new() -> Self {
        Self {
            adapter: core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()),
            state: core::sync::atomic::AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

static INSTANCE_REGISTRY: [InstanceRegistry; 2] =
    [const { InstanceRegistry::new() }; 2];

fn reserve_instance_id() -> Option<usize> {
    for (index, in_use) in INSTANCE_IDS.iter().enumerate() {
        if in_use
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some(index + 1);
        }
    }
    None
}

fn release_instance_id(instance_id: usize) {
    if let Some(in_use) = INSTANCE_IDS.get(instance_id.saturating_sub(1)) {
        in_use.store(false, Ordering::Release);
    }
}

const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_LINK_STATE>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_PACKET_QUEUE_CONFIG>();
const _: () = {
    assert!(
        core::mem::offset_of!(netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES, MappingRequirement)
            == 4
    );
    assert!(
        core::mem::offset_of!(
            netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES,
            FragmentBufferAlignment
        ) == 24
    );
    assert!(
        core::mem::offset_of!(netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES, DmaCapabilities)
            == 48
    );
};

unsafe fn object_context<T>(
    object: WDFOBJECT,
    type_info: *const wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO,
) -> *mut T {
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfObjectGetTypedContextWorker,
            object,
            type_info
        )
        .cast()
    }
}

unsafe fn instance_from_device(device: WDFDEVICE) -> Option<*mut InstanceState> {
    for entry in &INSTANCE_STATES {
        let state = entry.load(Ordering::Acquire);
        if !state.is_null()
            && (unsafe { (*state).pnp_device == device }
                || unsafe { (*state).control_device == device })
        {
            return Some(state);
        }
    }
    None
}

unsafe fn instance_from_pnp_device(device: WDFDEVICE) -> Option<*mut InstanceState> {
    for entry in &INSTANCE_STATES {
        let state = entry.load(Ordering::Acquire);
        if !state.is_null() && unsafe { (*state).pnp_device == device } {
            return Some(state);
        }
    }
    None
}

unsafe fn instance_from_adapter(
    adapter: netadaptercx_sys::NETADAPTER,
) -> Option<*mut InstanceState> {
    let adapter = adapter.cast::<c_void>();
    for entry in &INSTANCE_REGISTRY {
        if entry.adapter.load(Ordering::Acquire) == adapter {
            let state = entry.state.load(Ordering::Acquire);
            if !state.is_null() {
                return Some(state);
            }
        }
    }
    None
}

fn register_instance(state: &mut InstanceState) -> bool {
    let adapter = state.adapter.cast::<c_void>();
    for entry in &INSTANCE_REGISTRY {
        if entry
            .adapter
            .compare_exchange(
                core::ptr::null_mut(),
                adapter,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            entry.state.store(state, Ordering::Release);
            return true;
        }
    }
    false
}

fn unregister_instance(adapter: netadaptercx_sys::NETADAPTER) {
    let adapter = adapter.cast::<c_void>();
    for entry in &INSTANCE_REGISTRY {
        if entry.adapter.load(Ordering::Acquire) == adapter {
            entry.state.store(core::ptr::null_mut(), Ordering::Release);
            entry.adapter.store(core::ptr::null_mut(), Ordering::Release);
        }
    }
}

unsafe fn instance_from_io_queue(queue: WDFQUEUE) -> Option<*mut InstanceState> {
    let device = unsafe {
        call_unsafe_wdf_function_binding!(WdfIoQueueGetDevice, queue)
    };
    unsafe { instance_from_device(device) }
}

unsafe fn instance_from_work_item(work_item: WDFWORKITEM) -> Option<*mut InstanceState> {
    for entry in &INSTANCE_STATES {
        let state = entry.load(Ordering::Acquire);
        if !state.is_null()
            && (unsafe { (*state).read_work_item == work_item }
                || unsafe { (*state).write_work_item == work_item })
        {
            return Some(state);
        }
    }
    None
}

unsafe fn instance_from_packet_queue(
    queue: netadaptercx_sys::NETPACKETQUEUE,
) -> Option<*mut InstanceState> {
    let context = unsafe {
        object_context::<QueueContext>(
            queue.cast(),
            &raw const QUEUE_CONTEXT_TYPE_INFO,
        )
    };
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
    driver: WDFDRIVER,
    device_init: *mut WDFDEVICE_INIT,
) -> NTSTATUS {
    let Some(instance_id) = reserve_instance_id() else {
        debug_status(b"EvtDriverDeviceAdd instance limit", STATUS_DEVICE_BUSY);
        return STATUS_DEVICE_BUSY;
    };
    let state = allocate_instance_state(instance_id);
    if state.is_null() {
        release_instance_id(instance_id);
        debug_status(
            b"EvtDriverDeviceAdd state allocation",
            STATUS_INSUFFICIENT_RESOURCES,
        );
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    INSTANCE_STATES[instance_id - 1].store(state, Ordering::Release);

    debug_marker(b"EvtDriverDeviceAdd enter");
    let status = unsafe {
        // SAFETY: NetDeviceInitConfig is called once at PASSIVE_LEVEL before
        // the WDF device is created.
        net_call_device_init_config(device_init)
    };
    debug_status(b"NetDeviceInitConfig", status);
    if status != STATUS_SUCCESS {
        INSTANCE_STATES[instance_id - 1].store(core::ptr::null_mut(), Ordering::Release);
        release_instance_id(instance_id);
        unsafe { drop(Box::from_raw(state)); }
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
        FileObjectClass:
            wdk_sys::_WDF_FILEOBJECT_CLASS::WdfFileObjectWdfCannotUseFsContexts,
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
        ExecutionLevel:
            wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
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
        INSTANCE_STATES[instance_id - 1].store(core::ptr::null_mut(), Ordering::Release);
        release_instance_id(instance_id);
        unsafe { drop(Box::from_raw(state)); }
        return status;
    }
    let device_context = unsafe {
        object_context::<DeviceContext>(
            _pnp_device.cast(),
            &raw const DEVICE_CONTEXT_TYPE_INFO,
        )
    };
    if device_context.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, _pnp_device.cast());
            INSTANCE_STATES[instance_id - 1].store(core::ptr::null_mut(), Ordering::Release);
            release_instance_id(instance_id);
            drop(Box::from_raw(state));
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*device_context).instance = state;
    }
    unsafe {
        (*state).pnp_device = _pnp_device;
    }

    let status = create_control_device(driver, unsafe { &mut *state });
    debug_status(b"CreateControlDevice", status);
    if status != STATUS_SUCCESS {
        return status;
    }

    // SAFETY: The WDF device has been created and this callback runs at
    // PASSIVE_LEVEL during device addition.
    let status = unsafe { create_adapter(_pnp_device, &mut *state) };
    debug_status(b"CreateAdapter", status);
    status
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
            core::ptr::null_mut(),
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
    if !register_instance(state) {
        return STATUS_DEVICE_BUSY;
    }
    STATUS_SUCCESS
}

fn configure_adapter_link_state(
    adapter: netadaptercx_sys::NETADAPTER,
    instance_id: usize,
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
        net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterSetLinkLayerMtuSizeTableIndex as usize)
    };
    unsafe {
        set_mtu(
            netadaptercx_sys::NetDriverGlobals,
            adapter,
            (FRAME_MAXIMUM - FRAME_MINIMUM) as ULONG,
        );
    }

    let address = netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS {
        Length: 6,
        Address: [
            0x02, 0x57, 0x54, 0x41, 0x50, instance_id as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
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
        ExecutionLevel:
            wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
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
            object_context::<QueueContext>(
                packet_queue.cast(),
                &raw const QUEUE_CONTEXT_TYPE_INFO,
            )
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
        ) -> *const netadaptercx_sys::NET_RING_COLLECTION = unsafe {
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
        if queue == state.tx_queue {
            state.tx_queue_started.store(false, Ordering::Release);
        } else if queue == state.rx_queue {
            state.rx_queue_started.store(false, Ordering::Release);
            state.rx_notification_armed.store(false, Ordering::Release);
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
    let state = &mut *state_guard;
    let rx_rings = state.rx_rings;
    let rx_extension = state.rx_fragment_extension;
    if queue == state.rx_queue && !rx_rings.is_null() {
        inject_receive_frames(state, rx_rings, &rx_extension);
    }
    if queue != state.tx_queue || state.tx_rings.is_null() {
        return;
    }
    let tx_rings = state.tx_rings;
    let tx_extension = state.tx_fragment_extension;
    capture_transmit_packets(state, tx_rings, &tx_extension);
}

fn inject_receive_frames(
    state: &mut InstanceState,
    rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    extension: &netadaptercx_sys::NET_EXTENSION,
) {
    let (packet_ring, fragment_ring) = unsafe {
        let collection = match rings.as_ref() {
            Some(collection) => collection,
            None => return,
        };
        (collection.Rings[ring::PACKET_RING_INDEX], collection.Rings[ring::FRAGMENT_RING_INDEX])
    };
    if packet_ring.is_null() || fragment_ring.is_null() {
        return;
    }

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

        let frame = match dequeue_injection_frame(state) {
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
    }
}

fn capture_transmit_packets(
    state: &mut InstanceState,
    rings: *const netadaptercx_sys::NET_RING_COLLECTION,
    extension: &netadaptercx_sys::NET_EXTENSION,
) {
    let (packet_ring, fragment_ring) = unsafe {
        let collection = match rings.as_ref() {
            Some(collection) => collection,
            None => return,
        };
        (collection.Rings[ring::PACKET_RING_INDEX], collection.Rings[ring::FRAGMENT_RING_INDEX])
    };
    if packet_ring.is_null() || fragment_ring.is_null() {
        return;
    }

    let mut captured = false;
    loop {
        let (packet_begin, packet_end, fragment_end) = unsafe {
            ((*packet_ring).BeginIndex, (*packet_ring).EndIndex, (*fragment_ring).EndIndex)
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
            if !validate_fragment(fragment, address, &mut total_length) {
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
        if !valid || !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&total_length) {
            break;
        }

        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(total_length).is_ok() {
            fragment_index = fragment_begin;
            for _ in 0..fragment_count {
                let fragment = match unsafe { fragment_at(fragment_ring, fragment_index) } {
                    Some(fragment) => unsafe { &*fragment },
                    None => return,
                };
                let address = match unsafe { fragment_virtual_address(extension, fragment_index) } {
                    Some(address) => unsafe { &*address },
                    None => return,
                };
                let start = unsafe {
                    (address.VirtualAddress as *const u8).add(fragment.Offset() as usize)
                };
                let length = fragment.ValidLength() as usize;
                let data = unsafe { core::slice::from_raw_parts(start, length) };
                bytes.extend_from_slice(data);
                fragment_index = match unsafe { increment_index(&*fragment_ring, fragment_index) } {
                    Some(index) => index,
                    None => return,
                };
            }

            if let Ok(frame) = Frame::from_vec(bytes) {
                if enqueue_existing_capture_frame(state, frame).is_ok() {
                    captured = true;
                }
            }
        } else {
            debug_status(b"Tx capture allocation", STATUS_INSUFFICIENT_RESOURCES);
        }

        let next_packet = match unsafe { increment_index(&*packet_ring, packet_begin) } {
            Some(index) => index,
            None => break,
        };
        let next_fragment = match unsafe {
            advance_index(&*fragment_ring, fragment_begin, fragment_count)
        } {
            Some(index) => index,
            None => break,
        };
        unsafe {
            (*packet_ring).BeginIndex = next_packet;
            (*fragment_ring).BeginIndex = next_fragment;
        }
    }

    if captured {
        enqueue_work_item(state.read_work_item);
    }
}

fn validate_fragment(
    fragment: &netadaptercx_sys::NET_FRAGMENT,
    address: &netadaptercx_sys::NET_FRAGMENT_VIRTUAL_ADDRESS,
    total_length: &mut usize,
) -> bool {
    if address.VirtualAddress.is_null() {
        return false;
    }
    let offset = fragment.Offset() as usize;
    let capacity = fragment.Capacity() as usize;
    let valid_length = fragment.ValidLength() as usize;
    offset <= capacity
        && valid_length <= capacity - offset
        && valid_length <= FRAME_MAXIMUM
        && *total_length <= FRAME_MAXIMUM - valid_length
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
        state.rx_notification_armed.store(enabled != 0, Ordering::Release);
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
    ) -> netadaptercx_sys::_NET_PACKET_FILTER_FLAGS = unsafe {
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
    let requested_count = unsafe {
        get_multicast_address_count(netadaptercx_sys::NetDriverGlobals, receive_filter)
    };
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
        let (read_queue, write_queue) = {
            let state = &mut *state_guard;
            (state.read_queue, state.write_queue)
        };
        drop(state_guard);
        resume_manual_queues(read_queue, write_queue);
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
        let (read_queue, write_queue) = {
            let state = &mut *state_guard;
            state.lifecycle.store(INSTANCE_SUSPENDED, Ordering::Release);
            (state.read_queue, state.write_queue)
        };
        drop(state_guard);
        purge_queue(read_queue);
        purge_queue(write_queue);
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
    let (adapter, instance_id) = {
        let state = &mut *state_guard;
        state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
        (state.adapter, state.instance_id)
    };
    drop(state_guard);
    if adapter.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    configure_adapter_link_state(adapter, instance_id);

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
        MaximumFrameSize: FRAME_MAXIMUM as u64,
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
        Size: core::mem::size_of::<
            netadaptercx_sys::NET_ADAPTER_RECEIVE_FILTER_CAPABILITIES,
        >() as ULONG,
        SupportedPacketFilters: netadaptercx_sys::_NET_PACKET_FILTER_FLAGS_NetPacketFilterFlagDirected
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
    let (adapter, read_queue, write_queue) = {
        let state = &mut *state_guard;
        (state.adapter, state.read_queue, state.write_queue)
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

    purge_queue(read_queue);
    purge_queue(write_queue);
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return STATUS_DEVICE_NOT_READY;
    };
    let state = &mut *state_guard;
    clear_frame_queues(state);
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
    unregister_instance(adapter);
    state.adapter = core::ptr::null_mut();
    state.lifecycle.store(INSTANCE_CLOSED, Ordering::Release);

    STATUS_SUCCESS
}

fn create_control_device(driver: WDFDRIVER, state: &mut InstanceState) -> NTSTATUS {
    let sddl = unicode_string(&CONTROL_SDDL);
    let mut control_init = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfControlDeviceInitAllocate,
            driver,
            &sddl as *const UNICODE_STRING,
        )
    };
    if control_init.is_null() {
        debug_status(b"WdfControlDeviceInitAllocate", STATUS_INSUFFICIENT_RESOURCES);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    debug_marker(b"WdfControlDeviceInitAllocate");

    let mut file_config = WDF_FILEOBJECT_CONFIG {
        Size: core::mem::size_of::<WDF_FILEOBJECT_CONFIG>() as ULONG,
        EvtDeviceFileCreate: Some(evt_file_create),
        EvtFileClose: Some(evt_file_close),
        EvtFileCleanup: Some(evt_file_cleanup),
        FileObjectClass:
            wdk_sys::_WDF_FILEOBJECT_CLASS::WdfFileObjectWdfCannotUseFsContexts,
        AutoForwardCleanupClose: wdk_sys::_WDF_TRI_STATE::WdfUseDefault,
        ..WDF_FILEOBJECT_CONFIG::default()
    };
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitSetFileObjectConfig,
            control_init,
            &mut file_config,
            WDF_NO_OBJECT_ATTRIBUTES,
        );
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfDeviceInitSetExclusive, control_init, 1);
    }

    let device_name_buffer = control_name(state.instance_id, false);
    let device_name = unicode_string(&device_name_buffer);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitAssignName,
            control_init,
            &device_name as *const UNICODE_STRING,
        )
    };
    debug_status(b"WdfDeviceInitAssignName", status);
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfDeviceInitFree, control_init);
        }
        return status;
    }

    let mut device: WDFDEVICE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreate,
            &mut control_init,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut device,
        )
    };
    debug_status(b"WdfControlDeviceCreate", status);
    if status != STATUS_SUCCESS {
        if !control_init.is_null() {
            unsafe {
                call_unsafe_wdf_function_binding!(WdfDeviceInitFree, control_init);
            }
        }
        return status;
    }
    let injection_queue = match FrameQueue::try_new(FRAME_QUEUE_LIMIT) {
        Ok(queue) => queue,
        Err(_) => {
            debug_status(b"InjectionQueueCreate", STATUS_INSUFFICIENT_RESOURCES);
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
            }
            return STATUS_INSUFFICIENT_RESOURCES;
        }
    };
    let capture_queue = match FrameQueue::try_new(FRAME_QUEUE_LIMIT) {
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
        ExecutionLevel:
            wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut frame_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfSpinLockCreate,
            &mut lock_attributes,
            &mut frame_lock,
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
        call_unsafe_wdf_function_binding!(
            WdfSpinLockCreate,
            &mut lock_attributes,
            &mut state_lock,
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
    let mut default_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut default_queue_config,
            WDF_NO_OBJECT_ATTRIBUTES,
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
        ExecutionLevel:
            wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
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
    state.read_work_item = read_work_item;

    let mut write_work_config = WDF_WORKITEM_CONFIG {
        Size: core::mem::size_of::<WDF_WORKITEM_CONFIG>() as ULONG,
        EvtWorkItemFunc: Some(evt_write_drain_work_item),
        ..WDF_WORKITEM_CONFIG::default()
    };
    let mut write_work_item: WDFWORKITEM = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfWorkItemCreate,
            &mut write_work_config,
            &mut work_item_attributes,
            &mut write_work_item,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }
    state.write_work_item = write_work_item;

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

    let mut write_queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchManual,
        AllowZeroLengthRequests: 1,
        EvtIoStop: Some(evt_io_stop),
        ..WDF_IO_QUEUE_CONFIG::default()
    };
    let mut write_queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut write_queue_config,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut write_queue,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let symbolic_link_buffer = control_name(state.instance_id, true);
    let symbolic_link = unicode_string(&symbolic_link_buffer);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreateSymbolicLink,
            device,
            &symbolic_link as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    unsafe {
        call_unsafe_wdf_function_binding!(WdfControlFinishInitializing, device);
        state.control_device = device;
        state.read_queue = read_queue;
        state.write_queue = write_queue;
        state.frame_lock = frame_lock;
        state.state_lock = state_lock;
        state.injection_queue = Some(injection_queue);
        state.capture_queue = Some(capture_queue);
        state.read_work_item = read_work_item;
        state.write_work_item = write_work_item;
    }
    STATUS_SUCCESS
}

unsafe extern "C" fn evt_instance_context_destroy(object: WDFOBJECT) {
    let context = unsafe {
        object_context::<DeviceContext>(
            object,
            &raw const DEVICE_CONTEXT_TYPE_INFO,
        )
    };
    if !context.is_null() && unsafe { !(*context).instance.is_null() } {
        let state = unsafe { (*context).instance };
        unsafe { (*context).instance = core::ptr::null_mut(); }
        let control_device = unsafe { (*state).control_device };
        unsafe { (*state).control_device = core::ptr::null_mut(); }
        if !control_device.is_null() {
            // Control devices cannot use a PnP device as their WDF parent.
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, control_device.cast());
            }
        }
        if !unsafe { (*state).adapter.is_null() } {
            unsafe { unregister_instance((*state).adapter); }
        }
        INSTANCE_STATES[unsafe { (*state).instance_id } - 1]
            .compare_exchange(
                state,
                core::ptr::null_mut(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok();
        unsafe { release_instance_id((*state).instance_id); }
        unsafe { drop(Box::from_raw(state)); }
    }
}

fn control_name(instance_id: usize, symbolic: bool) -> [u16; 32] {
    let prefix: &[u8] = if symbolic {
        b"\\DosDevices\\Global\\WinTapRust"
    } else {
        b"\\Device\\WinTapRust"
    };
    let mut result = [0u16; 32];
    let mut length = 0;
    for byte in prefix {
        result[length] = *byte as u16;
        length += 1;
    }
    if instance_id > 1 {
        result[length] = b'0' as u16 + instance_id as u16;
        length += 1;
    }
    result[length] = 0;
    result
}

extern "C" fn evt_file_create(
    _device: WDFDEVICE,
    request: WDFREQUEST,
    _file_object: WDFFILEOBJECT,
) {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestComplete, request, STATUS_SUCCESS);
    }
}

extern "C" fn evt_file_close(_file_object: WDFFILEOBJECT) {}

extern "C" fn evt_file_cleanup(file_object: WDFFILEOBJECT) {
    let device = unsafe {
        call_unsafe_wdf_function_binding!(WdfFileObjectGetDevice, file_object)
    };
    if let Some(state) = unsafe { instance_from_device(device) } {
        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let (was_suspended, read_queue, write_queue) = {
            let state = &mut *state_guard;
            let was_suspended =
                state.lifecycle.load(Ordering::Acquire) == INSTANCE_SUSPENDED;
            state.lifecycle.store(INSTANCE_CLOSING, Ordering::Release);
            (was_suspended, state.read_queue, state.write_queue)
        };
        drop(state_guard);
        purge_queue(read_queue);
        purge_queue(write_queue);
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
        if !should_resume {
            return;
        }

        // WdfIoQueueStart can synchronously dispatch request handlers.
        resume_manual_queues(read_queue, write_queue);

        let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
            return;
        };
        let state = &mut *state_guard;
        if state.lifecycle.load(Ordering::Acquire) == INSTANCE_CLOSING
            && !state.adapter.is_null()
        {
            reopen_frame_queues(state);
            state.lifecycle.store(INSTANCE_OPEN, Ordering::Release);
        }
    }
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
    let state = &mut *state_guard;
    if state.lifecycle.load(Ordering::Acquire) != INSTANCE_OPEN {
        complete_request(request, STATUS_DEVICE_NOT_READY);
        return;
    }
    if !try_admit(&state.pending_reads, PENDING_READ_LIMIT) {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let target = state.read_queue;
    if !forward_request(request, target) {
        release_request(&state.pending_reads);
    } else if has_queued_capture_frame(state) {
        enqueue_work_item(state.read_work_item);
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
    if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&length) {
        complete_request(request, STATUS_INVALID_PARAMETER);
        return;
    }
    if !try_admit(&state.pending_writes, PENDING_WRITE_LIMIT) {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let target = state.write_queue;
    if !forward_request(request, target) {
        release_request(&state.pending_writes);
    } else {
        enqueue_work_item(state.write_work_item);
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
    let read_queue = state.read_queue;
    let write_queue = state.write_queue;
    if queue == read_queue {
        release_request(&state.pending_reads);
    } else if queue == write_queue {
        release_request(&state.pending_writes);
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

fn resume_manual_queues(read_queue: WDFQUEUE, write_queue: WDFQUEUE) {
    for queue in [read_queue, write_queue] {
        if !queue.is_null() {
            unsafe {
                call_unsafe_wdf_function_binding!(WdfIoQueueStart, queue);
            }
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

extern "C" fn evt_write_drain_work_item(work_item: WDFWORKITEM) {
    let Some(state) = (unsafe { instance_from_work_item(work_item) }) else {
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return;
    };
    let state = &mut *state_guard;
    let mut notification_queue: netadaptercx_sys::NETPACKETQUEUE = core::ptr::null_mut();
    loop {
        let mut request = core::ptr::null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoQueueRetrieveNextRequest,
                state.write_queue,
                &mut request,
            )
        };
        if status != STATUS_SUCCESS {
            break;
        }
        release_request(&state.pending_writes);

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
            complete_request(request, status);
            continue;
        }
        if input_length > FRAME_MAXIMUM {
            complete_request(request, STATUS_INVALID_BUFFER_SIZE);
            continue;
        }

        let bytes = unsafe { core::slice::from_raw_parts(input.cast::<u8>(), input_length) };
        match enqueue_injection_frame(state, bytes) {
            Ok(()) => {
                complete_request_with_information(request, STATUS_SUCCESS, input_length);
                if notification_queue.is_null() {
                    notification_queue = take_rx_notification(state);
                }
            }
            Err(QueueError::Full) => complete_request(request, STATUS_DEVICE_BUSY),
            Err(QueueError::Closed) => complete_request(request, STATUS_DEVICE_NOT_READY),
            Err(QueueError::InvalidFrameLength) => {
                complete_request(request, STATUS_INVALID_BUFFER_SIZE)
            }
            Err(QueueError::InsufficientResources) => {
                complete_request(request, STATUS_INSUFFICIENT_RESOURCES)
            }
        }
    }
    drop(state_guard);
    if !notification_queue.is_null() {
        notify_more_received_packets(notification_queue);
    }
}

extern "C" fn evt_read_completion_work_item(work_item: WDFWORKITEM) {
    let Some(state) = (unsafe { instance_from_work_item(work_item) }) else {
        return;
    };
    let Some(mut state_guard) = (unsafe { InstanceStateGuard::new(state) }) else {
        return;
    };
    let state = &mut *state_guard;
    loop {
        let mut request = core::ptr::null_mut();
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
                if forward_request(request, target) {
                    return;
                } else {
                    release_request(&state.pending_reads);
                }
                return;
            }
        };
        release_request(&state.pending_reads);

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
            let _ = enqueue_existing_capture_frame(state, frame);
            complete_request(request, status);
            continue;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                frame.as_bytes().as_ptr(),
                output.cast::<u8>(),
                frame.as_bytes().len(),
            );
        }
        complete_request_with_information(request, STATUS_SUCCESS, frame.as_bytes().len());
    }
}

fn enqueue_injection_frame(state: &mut InstanceState, bytes: &[u8]) -> Result<(), QueueError> {
    let frame = Frame::from_bytes(bytes)?;
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

fn enqueue_existing_capture_frame(
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
            .capture_queue
            .as_mut()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.enqueue(frame));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        result
    }
}

fn dequeue_injection_frame(state: &mut InstanceState) -> Option<Frame> {
    let lock = state.frame_lock;
    if lock.is_null() {
        return None;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let frame = state
            .injection_queue
            .as_mut()
            .and_then(FrameQueue::dequeue);
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        frame
    }
}

fn dequeue_capture_frame(state: &mut InstanceState) -> Option<Frame> {
    let lock = state.frame_lock;
    if lock.is_null() {
        return None;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let frame = state
            .capture_queue
            .as_mut()
            .and_then(FrameQueue::dequeue);
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        frame
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

fn has_queued_capture_frame(state: &mut InstanceState) -> bool {
    let lock = state.frame_lock;
    if lock.is_null() {
        return false;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let has_frame = state
            .capture_queue
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

fn unicode_string(buffer: &[u16]) -> UNICODE_STRING {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    let byte_length = length * core::mem::size_of::<u16>();
    let maximum_length = if length < buffer.len() {
        byte_length + core::mem::size_of::<u16>()
    } else {
        byte_length
    };
    UNICODE_STRING {
        Length: byte_length as u16,
        MaximumLength: maximum_length as u16,
        Buffer: buffer.as_ptr() as *mut u16,
    }
}
