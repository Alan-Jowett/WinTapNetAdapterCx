#![no_std]

extern crate alloc;
#[cfg(not(test))]
extern crate wdk_panic;

use alloc::alloc::{Layout, alloc};
use alloc::boxed::Box;
use core::ffi::c_void;

#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    DRIVER_OBJECT, GUID, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, ULONG, UNICODE_STRING,
    WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER, WDF_CHILD_LIST_CONFIG, WDF_DRIVER_CONFIG,
    WDF_IO_QUEUE_CONFIG, WDF_NO_OBJECT_ATTRIBUTES, WDF_OBJECT_ATTRIBUTES, WDFCHILDLIST, WDFDEVICE,
    WDFDEVICE_INIT, WDFDRIVER, WDFOBJECT, WDFQUEUE, WDFREQUEST, WDFWAITLOCK,
    call_unsafe_wdf_function_binding,
};

const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_PENDING: NTSTATUS = 0x0000_0103;
const STATUS_INVALID_PARAMETER: NTSTATUS = 0xC000_000D_u32 as i32;
const STATUS_INVALID_DEVICE_REQUEST: NTSTATUS = 0xC000_0010_u32 as i32;
const STATUS_INVALID_BUFFER_SIZE: NTSTATUS = 0xC000_0206_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: NTSTATUS = 0xC000_0023_u32 as i32;
const STATUS_DEVICE_BUSY: NTSTATUS = 0xC000_00E8_u32 as i32;
const STATUS_DEVICE_NOT_READY: NTSTATUS = 0xC000_00A3_u32 as i32;
const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009A_u32 as i32;
const STATUS_OBJECT_NAME_NOT_FOUND: NTSTATUS = 0xC000_0034_u32 as i32;
const STATUS_UNSUCCESSFUL: NTSTATUS = 0xC000_0001_u32 as i32;

const MANAGER_PROTOCOL_VERSION: u16 = 1;
const MANAGER_OPERATION_CREATE: u16 = 1;
const MANAGER_OPERATION_REMOVE: u16 = 2;
const MANAGER_OPERATION_ENUMERATE: u16 = 3;
const MANAGER_OPERATION_QUERY: u16 = 4;
const MANAGER_IOCTL: ULONG = 0x0022_2004;
const MAX_INTERFACE_CHARS: usize = 260;
const MAX_MANAGER_OUTPUT: usize = 64 * 1024;

const LIFECYCLE_ABSENT: u32 = 0;
const LIFECYCLE_CREATING: u32 = 1;
const LIFECYCLE_ACTIVE: u32 = 2;
const LIFECYCLE_REMOVING: u32 = 3;
const LIFECYCLE_FAILED: u32 = 4;

const ADMIN_SDDL: [u16; 16] = [
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
const MANAGER_DEVICE_NAME: [u16; 22] = [
    b'\\' as u16,
    b'D' as u16,
    b'e' as u16,
    b'v' as u16,
    b'i' as u16,
    b'c' as u16,
    b'e' as u16,
    b'\\' as u16,
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'T' as u16,
    b'a' as u16,
    b'p' as u16,
    b'B' as u16,
    b'u' as u16,
    b's' as u16,
    b'M' as u16,
    b'g' as u16,
    b'r' as u16,
    0,
    0,
];
const MANAGER_SYMBOLIC_LINK: [u16; 32] = [
    b'\\' as u16,
    b'D' as u16,
    b'o' as u16,
    b's' as u16,
    b'D' as u16,
    b'e' as u16,
    b'v' as u16,
    b'i' as u16,
    b'c' as u16,
    b'e' as u16,
    b's' as u16,
    b'\\' as u16,
    b'G' as u16,
    b'l' as u16,
    b'o' as u16,
    b'b' as u16,
    b'a' as u16,
    b'l' as u16,
    b'\\' as u16,
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'T' as u16,
    b'a' as u16,
    b'p' as u16,
    b'B' as u16,
    b'u' as u16,
    b's' as u16,
    b'M' as u16,
    b'g' as u16,
    b'r' as u16,
    0,
];
const CHILD_COMPATIBLE_ID: [u16; 22] = [
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
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'T' as u16,
    b'a' as u16,
    b'p' as u16,
    b'C' as u16,
    b'h' as u16,
    b'i' as u16,
    b'l' as u16,
    b'd' as u16,
    0,
];

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

static BUS_CONTEXT_NAME: &[u8] = b"WINTAP_BUS_CONTEXT\0";
static CHILD_PDO_CONTEXT_NAME: &[u8] = b"WINTAP_CHILD_PDO_CONTEXT\0";

#[repr(C)]
struct ChildIdentification {
    header: WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
    guid: GUID,
}

#[repr(C)]
struct ManagerRequest {
    version: u16,
    operation: u16,
    length: u32,
    request_id: u64,
    adapter_guid: GUID,
    cursor: u32,
    reserved: u32,
}

#[repr(C)]
struct ManagerResponseHeader {
    version: u16,
    operation: u16,
    length: u32,
    request_id: u64,
    operation_status: NTSTATUS,
    record_count: u32,
    next_cursor: u32,
    reserved: u32,
}

#[repr(C)]
struct ManagerRecord {
    adapter_guid: GUID,
    lifecycle: u32,
    terminal_status: NTSTATUS,
    request_id: u64,
    interface_length: u16,
    _padding: u16,
    interface_name: [u16; MAX_INTERFACE_CHARS],
}

const _: () = {
    assert!(core::mem::size_of::<ManagerRequest>() == 40);
    assert!(core::mem::size_of::<ManagerResponseHeader>() == 32);
    assert!(core::mem::size_of::<ManagerRecord>() == 560);
};

struct ChildNode {
    guid: GUID,
    lifecycle: u32,
    terminal_status: NTSTATUS,
    request_id: u64,
    pdo: WDFDEVICE,
    interface_length: u16,
    interface_name: [u16; MAX_INTERFACE_CHARS],
    next: *mut ChildNode,
}

struct BusState {
    child_list: WDFCHILDLIST,
    lock: WDFWAITLOCK,
    children: *mut ChildNode,
}

#[repr(C)]
struct BusDeviceContext {
    state: *mut BusState,
}

#[repr(C)]
struct ChildPdoContext {
    bus_state: *mut BusState,
    bus_device: WDFDEVICE,
    guid: GUID,
}

static mut BUS_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: BUS_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<BusDeviceContext>(),
        UniqueType: &raw const BUS_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

static mut CHILD_PDO_CONTEXT_TYPE_INFO: wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO =
    wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO {
        Size: core::mem::size_of::<wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO>() as ULONG,
        ContextName: CHILD_PDO_CONTEXT_NAME.as_ptr() as *const i8,
        ContextSize: core::mem::size_of::<ChildPdoContext>(),
        UniqueType: &raw const CHILD_PDO_CONTEXT_TYPE_INFO,
        EvtDriverGetUniqueContextType: None,
    };

struct BusStateGuard {
    state: *mut BusState,
}

impl BusStateGuard {
    unsafe fn acquire(state: *mut BusState) -> Option<Self> {
        if state.is_null() || unsafe { (*state).lock.is_null() } {
            return None;
        }
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockAcquire,
                (*state).lock,
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

impl Drop for BusStateGuard {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWaitLockRelease, (*self.state).lock);
        }
    }
}

unsafe fn object_context<T>(
    object: WDFOBJECT,
    type_info: *const wdk_sys::_WDF_OBJECT_CONTEXT_TYPE_INFO,
) -> *mut T {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfObjectGetTypedContextWorker, object, type_info).cast()
    }
}

unsafe fn state_from_device(device: WDFDEVICE) -> Option<*mut BusState> {
    let context = unsafe {
        object_context::<BusDeviceContext>(device.cast(), &raw const BUS_CONTEXT_TYPE_INFO)
    };
    if context.is_null() || unsafe { (*context).state.is_null() } {
        None
    } else {
        Some(unsafe { (*context).state })
    }
}

fn unicode_string(buffer: &[u16]) -> UNICODE_STRING {
    let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    let byte_length = length * core::mem::size_of::<u16>();
    UNICODE_STRING {
        Length: byte_length as u16,
        MaximumLength: (byte_length + core::mem::size_of::<u16>()) as u16,
        Buffer: buffer.as_ptr() as *mut u16,
    }
}

fn guid_equal(left: &GUID, right: &GUID) -> bool {
    left.Data1 == right.Data1
        && left.Data2 == right.Data2
        && left.Data3 == right.Data3
        && left.Data4 == right.Data4
}

fn valid_guid(guid: &GUID) -> bool {
    guid.Data1 != 0
        || guid.Data2 != 0
        || guid.Data3 != 0
        || guid.Data4.iter().any(|byte| *byte != 0)
}

fn allocate_value<T>(value: T) -> *mut T {
    let allocation = unsafe { alloc(Layout::new::<T>()) }.cast::<T>();
    if !allocation.is_null() {
        unsafe {
            allocation.write(value);
        }
    }
    allocation
}

unsafe fn find_child(state: *mut BusState, guid: &GUID) -> *mut ChildNode {
    let mut node = unsafe { (*state).children };
    while !node.is_null() {
        if guid_equal(unsafe { &(*node).guid }, guid) {
            return node;
        }
        node = unsafe { (*node).next };
    }
    core::ptr::null_mut()
}

/// The caller holds the bus state lock and does not use `node` afterward.
unsafe fn reclaim_child(state: *mut BusState, node: *mut ChildNode) {
    let mut link = unsafe { &mut (*state).children as *mut *mut ChildNode };
    while !unsafe { (*link).is_null() } {
        let current = unsafe { *link };
        if current == node {
            unsafe {
                *link = (*current).next;
                drop(Box::from_raw(current));
            }
            return;
        }
        link = unsafe { &mut (*current).next as *mut *mut ChildNode };
    }
}

fn make_identification(guid: GUID) -> ChildIdentification {
    ChildIdentification {
        header: WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER {
            IdentificationDescriptionSize: core::mem::size_of::<ChildIdentification>() as ULONG,
        },
        guid,
    }
}

fn hex(value: u8) -> u16 {
    match value {
        0..=9 => b'0' as u16 + value as u16,
        _ => b'A' as u16 + (value - 10) as u16,
    }
}

fn guid_text(guid: &GUID, prefix: &[u16]) -> [u16; 64] {
    let mut text = [0u16; 64];
    let mut offset = 0;
    for character in prefix {
        if *character == 0 {
            break;
        }
        text[offset] = *character;
        offset += 1;
    }
    let values = [
        ((guid.Data1 >> 28) & 0xf) as u8,
        ((guid.Data1 >> 24) & 0xf) as u8,
        ((guid.Data1 >> 20) & 0xf) as u8,
        ((guid.Data1 >> 16) & 0xf) as u8,
        ((guid.Data1 >> 12) & 0xf) as u8,
        ((guid.Data1 >> 8) & 0xf) as u8,
        ((guid.Data1 >> 4) & 0xf) as u8,
        (guid.Data1 & 0xf) as u8,
        ((guid.Data2 >> 12) & 0xf) as u8,
        ((guid.Data2 >> 8) & 0xf) as u8,
        ((guid.Data2 >> 4) & 0xf) as u8,
        (guid.Data2 & 0xf) as u8,
        ((guid.Data3 >> 12) & 0xf) as u8,
        ((guid.Data3 >> 8) & 0xf) as u8,
        ((guid.Data3 >> 4) & 0xf) as u8,
        (guid.Data3 & 0xf) as u8,
    ];
    text[offset] = b'{' as u16;
    offset += 1;
    for (index, value) in values.iter().enumerate() {
        text[offset] = hex(*value);
        offset += 1;
        if index == 7 || index == 11 || index == 15 {
            text[offset] = b'-' as u16;
            offset += 1;
        }
    }
    for (index, byte) in guid.Data4.iter().enumerate() {
        text[offset] = hex(byte >> 4);
        text[offset + 1] = hex(byte & 0xf);
        offset += 2;
        if index == 1 {
            text[offset] = b'-' as u16;
            offset += 1;
        }
    }
    text[offset] = b'}' as u16;
    text[offset + 1] = 0;
    text
}

fn child_device_id(guid: &GUID) -> [u16; 64] {
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
        0,
    ];
    guid_text(guid, &PREFIX)
}

unsafe fn zero_record(record: *mut ManagerRecord) {
    // ManagerRecord has four bytes of ABI tail padding.
    unsafe {
        core::ptr::write_bytes(
            record.cast::<u8>(),
            0,
            core::mem::size_of::<ManagerRecord>(),
        );
    }
}

unsafe fn write_record(record: *mut ManagerRecord, node: &ChildNode) {
    unsafe {
        zero_record(record);
        (*record).adapter_guid = node.guid;
        (*record).lifecycle = node.lifecycle;
        (*record).terminal_status = node.terminal_status;
        (*record).request_id = node.request_id;
        (*record).interface_length = node.interface_length;
        (*record)._padding = 0;
        (*record).interface_name = node.interface_name;
    }
}

unsafe fn copy_record(record: *mut ManagerRecord, source: &ManagerRecord) {
    unsafe {
        zero_record(record);
        (*record).adapter_guid = source.adapter_guid;
        (*record).lifecycle = source.lifecycle;
        (*record).terminal_status = source.terminal_status;
        (*record).request_id = source.request_id;
        (*record).interface_length = source.interface_length;
        (*record)._padding = 0;
        (*record).interface_name = source.interface_name;
    }
}

fn complete(request: WDFREQUEST, status: NTSTATUS) {
    unsafe {
        call_unsafe_wdf_function_binding!(WdfRequestComplete, request, status);
    }
}

fn complete_with_information(request: WDFREQUEST, status: NTSTATUS, information: usize) {
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestCompleteWithInformation,
            request,
            status,
            information as u64,
        );
    }
}

#[unsafe(export_name = "DriverEntry")]
pub unsafe extern "system" fn driver_entry(
    driver: &mut DRIVER_OBJECT,
    registry_path: PCUNICODE_STRING,
) -> NTSTATUS {
    let mut config = WDF_DRIVER_CONFIG {
        Size: core::mem::size_of::<WDF_DRIVER_CONFIG>() as ULONG,
        EvtDriverDeviceAdd: Some(evt_bus_device_add),
        ..WDF_DRIVER_CONFIG::default()
    };
    let mut wdf_driver: WDFDRIVER = core::ptr::null_mut();
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDriverCreate,
            driver as PDRIVER_OBJECT,
            registry_path,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut config,
            &mut wdf_driver,
        )
    }
}

extern "C" fn evt_bus_device_add(_driver: WDFDRIVER, device_init: *mut WDFDEVICE_INIT) -> NTSTATUS {
    let mut child_list_config = WDF_CHILD_LIST_CONFIG {
        Size: core::mem::size_of::<WDF_CHILD_LIST_CONFIG>() as ULONG,
        IdentificationDescriptionSize: core::mem::size_of::<ChildIdentification>() as ULONG,
        EvtChildListCreateDevice: Some(evt_child_list_create_device),
        EvtChildListIdentificationDescriptionCompare: Some(evt_child_identification_compare),
        EvtChildListIdentificationDescriptionDuplicate: Some(evt_child_identification_duplicate),
        EvtChildListIdentificationDescriptionCleanup: Some(evt_child_identification_cleanup),
        ..WDF_CHILD_LIST_CONFIG::default()
    };
    unsafe {
        call_unsafe_wdf_function_binding!(
            WdfFdoInitSetDefaultChildListConfig,
            device_init,
            &mut child_list_config,
            WDF_NO_OBJECT_ATTRIBUTES,
        );
    }

    let sddl = unicode_string(&ADMIN_SDDL);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitAssignSDDLString,
            device_init,
            &sddl as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }
    let name = unicode_string(&MANAGER_DEVICE_NAME);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitAssignName,
            device_init,
            &name as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        EvtCleanupCallback: Some(evt_bus_device_cleanup),
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ContextTypeInfo: &raw const BUS_CONTEXT_TYPE_INFO,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut device = core::ptr::null_mut();
    let mut device_init = device_init;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreate,
            &mut device_init,
            &mut attributes,
            &mut device,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut lock_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ParentObject: device.cast(),
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut lock = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(WdfWaitLockCreate, &mut lock_attributes, &mut lock)
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let child_list =
        unsafe { call_unsafe_wdf_function_binding!(WdfFdoGetDefaultChildList, device) };
    if child_list.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return STATUS_DEVICE_NOT_READY;
    }
    let context = unsafe {
        object_context::<BusDeviceContext>(device.cast(), &raw const BUS_CONTEXT_TYPE_INFO)
    };
    if context.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*context).state = core::ptr::null_mut();
    }
    let state = allocate_value(BusState {
        child_list,
        lock,
        children: core::ptr::null_mut(),
    });
    if state.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*context).state = state;
    }

    let mut queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchSequential,
        PowerManaged: wdk_sys::_WDF_TRI_STATE::WdfFalse,
        AllowZeroLengthRequests: 0,
        DefaultQueue: 1,
        EvtIoDeviceControl: Some(evt_manager_device_control),
        ..WDF_IO_QUEUE_CONFIG::default()
    };
    let mut queue_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelPassive,
        SynchronizationScope: wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeNone,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut queue: WDFQUEUE = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfIoQueueCreate,
            device,
            &mut queue_config,
            &mut queue_attributes,
            &mut queue,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, device.cast());
        }
        return status;
    }

    let symbolic_link = unicode_string(&MANAGER_SYMBOLIC_LINK);
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
    }
    status
}

extern "C" fn evt_child_identification_compare(
    _child_list: WDFCHILDLIST,
    first: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
    second: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
) -> u8 {
    if first.is_null() || second.is_null() {
        return 0;
    }
    let first = unsafe { &*(first.cast::<ChildIdentification>()) };
    let second = unsafe { &*(second.cast::<ChildIdentification>()) };
    guid_equal(&first.guid, &second.guid) as u8
}

extern "C" fn evt_child_identification_duplicate(
    _child_list: WDFCHILDLIST,
    source: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
    destination: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
) -> NTSTATUS {
    if source.is_null() || destination.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            source.cast::<ChildIdentification>(),
            destination.cast::<ChildIdentification>(),
            1,
        );
    }
    STATUS_SUCCESS
}

extern "C" fn evt_child_identification_cleanup(
    _child_list: WDFCHILDLIST,
    _identification: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
) {
}

extern "C" fn evt_child_list_create_device(
    child_list: WDFCHILDLIST,
    identification: *mut WDF_CHILD_IDENTIFICATION_DESCRIPTION_HEADER,
    device_init: *mut WDFDEVICE_INIT,
) -> NTSTATUS {
    if identification.is_null() || device_init.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let guid = unsafe { (*(identification.cast::<ChildIdentification>())).guid };
    if !valid_guid(&guid) {
        return STATUS_INVALID_PARAMETER;
    }
    let bus_device =
        unsafe { call_unsafe_wdf_function_binding!(WdfChildListGetDevice, child_list) };
    let Some(bus_state) = (unsafe { state_from_device(bus_device) }) else {
        return STATUS_DEVICE_NOT_READY;
    };

    let device_id_text = child_device_id(&guid);
    let device_id = unicode_string(&device_id_text);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfPdoInitAssignDeviceID,
            device_init,
            &device_id as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(bus_state, &guid, status);
        }
        return status;
    }
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfPdoInitAddHardwareID,
            device_init,
            &device_id as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(bus_state, &guid, status);
        }
        return status;
    }
    let compatible_id = unicode_string(&CHILD_COMPATIBLE_ID);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfPdoInitAddHardwareID,
            device_init,
            &compatible_id as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(bus_state, &guid, status);
        }
        return status;
    }

    let mut attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        EvtCleanupCallback: Some(evt_child_pdo_cleanup),
        ExecutionLevel: wdk_sys::_WDF_EXECUTION_LEVEL::WdfExecutionLevelInheritFromParent,
        SynchronizationScope:
            wdk_sys::_WDF_SYNCHRONIZATION_SCOPE::WdfSynchronizationScopeInheritFromParent,
        ContextTypeInfo: &raw const CHILD_PDO_CONTEXT_TYPE_INFO,
        ..WDF_OBJECT_ATTRIBUTES::default()
    };
    let mut pdo = core::ptr::null_mut();
    let mut child_device_init = device_init;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreate,
            &mut child_device_init,
            &mut attributes,
            &mut pdo,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(bus_state, &guid, status);
        }
        return status;
    }
    let pdo_context = unsafe {
        object_context::<ChildPdoContext>(pdo.cast(), &raw const CHILD_PDO_CONTEXT_TYPE_INFO)
    };
    if pdo_context.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, pdo.cast());
            mark_child_failure(bus_state, &guid, STATUS_INSUFFICIENT_RESOURCES);
        }
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    unsafe {
        (*pdo_context).bus_state = core::ptr::null_mut();
        (*pdo_context).bus_device = core::ptr::null_mut();
        (*pdo_context).guid = guid;
        call_unsafe_wdf_function_binding!(
            WdfObjectReferenceActual,
            bus_device.cast(),
            core::ptr::null_mut::<c_void>(),
            0,
            core::ptr::null::<i8>(),
        );
        (*pdo_context).bus_state = bus_state;
        (*pdo_context).bus_device = bus_device;
    }

    unsafe {
        let Some(_guard) = BusStateGuard::acquire(bus_state) else {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, pdo.cast());
            return STATUS_DEVICE_NOT_READY;
        };
        let node = find_child(bus_state, &guid);
        if node.is_null() {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, pdo.cast());
            return STATUS_OBJECT_NAME_NOT_FOUND;
        }
        (*node).pdo = pdo;
    }
    STATUS_SUCCESS
}

unsafe fn mark_child_failure(state: *mut BusState, guid: &GUID, status: NTSTATUS) {
    let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
        return;
    };
    let node = unsafe { find_child(state, guid) };
    if !node.is_null() {
        unsafe {
            (*node).lifecycle = LIFECYCLE_FAILED;
            (*node).terminal_status = status;
        }
    }
}

unsafe fn reclaim_removed_child_without_pdo(state: *mut BusState, guid: &GUID) {
    let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
        return;
    };
    let node = unsafe { find_child(state, guid) };
    if !node.is_null()
        && unsafe { (*node).lifecycle == LIFECYCLE_REMOVING && (*node).pdo.is_null() }
    {
        unsafe {
            reclaim_child(state, node);
        }
    }
}

unsafe extern "C" fn evt_child_pdo_cleanup(object: WDFOBJECT) {
    let context = unsafe {
        object_context::<ChildPdoContext>(object, &raw const CHILD_PDO_CONTEXT_TYPE_INFO)
    };
    if context.is_null() {
        return;
    }
    let state = unsafe { (*context).bus_state };
    let bus_device = unsafe { (*context).bus_device };
    let guid = unsafe { (*context).guid };
    unsafe {
        (*context).bus_state = core::ptr::null_mut();
        (*context).bus_device = core::ptr::null_mut();
    }
    if let Some(_guard) = unsafe { BusStateGuard::acquire(state) } {
        let node = unsafe { find_child(state, &guid) };
        if !node.is_null() {
            unsafe {
                (*node).pdo = core::ptr::null_mut();
                (*node).interface_name = [0; MAX_INTERFACE_CHARS];
                (*node).interface_length = 0;
                if (*node).lifecycle == LIFECYCLE_REMOVING {
                    reclaim_child(state, node);
                } else if (*node).lifecycle == LIFECYCLE_CREATING
                    || (*node).lifecycle == LIFECYCLE_ACTIVE
                {
                    (*node).lifecycle = LIFECYCLE_FAILED;
                    (*node).terminal_status = STATUS_UNSUCCESSFUL;
                }
            }
        }
    }
    if !bus_device.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfObjectDereferenceActual,
                bus_device.cast(),
                core::ptr::null_mut::<c_void>(),
                0,
                core::ptr::null::<i8>(),
            );
        }
    }
}

extern "C" fn evt_manager_device_control(
    queue: WDFQUEUE,
    request: WDFREQUEST,
    output_length: usize,
    input_length: usize,
    ioctl: ULONG,
) {
    if ioctl != MANAGER_IOCTL {
        complete(request, STATUS_INVALID_DEVICE_REQUEST);
        return;
    }
    if input_length != core::mem::size_of::<ManagerRequest>() {
        complete(request, STATUS_INVALID_BUFFER_SIZE);
        return;
    }
    let mut input: *mut c_void = core::ptr::null_mut();
    let mut actual_input_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveInputBuffer,
            request,
            core::mem::size_of::<ManagerRequest>(),
            &mut input,
            &mut actual_input_length,
        )
    };
    if status != STATUS_SUCCESS || actual_input_length != core::mem::size_of::<ManagerRequest>() {
        complete(
            request,
            if status == STATUS_SUCCESS {
                STATUS_INVALID_BUFFER_SIZE
            } else {
                status
            },
        );
        return;
    }
    // METHOD_BUFFERED aliases input and output. Copy before obtaining an output buffer.
    let manager_request = unsafe { input.cast::<ManagerRequest>().read() };
    if manager_request.version != MANAGER_PROTOCOL_VERSION
        || manager_request.length as usize != core::mem::size_of::<ManagerRequest>()
        || manager_request.reserved != 0
        || manager_request.request_id == 0
        || !matches!(
            manager_request.operation,
            MANAGER_OPERATION_CREATE
                | MANAGER_OPERATION_REMOVE
                | MANAGER_OPERATION_ENUMERATE
                | MANAGER_OPERATION_QUERY
        )
        || (manager_request.operation != MANAGER_OPERATION_ENUMERATE
            && (!valid_guid(&manager_request.adapter_guid) || manager_request.cursor != 0))
        || (manager_request.operation == MANAGER_OPERATION_ENUMERATE
            && valid_guid(&manager_request.adapter_guid))
    {
        complete(request, STATUS_INVALID_PARAMETER);
        return;
    }
    let device = unsafe { call_unsafe_wdf_function_binding!(WdfIoQueueGetDevice, queue) };
    let Some(state) = (unsafe { state_from_device(device) }) else {
        complete(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    match manager_request.operation {
        MANAGER_OPERATION_CREATE => manager_create(state, request, &manager_request, output_length),
        MANAGER_OPERATION_REMOVE => manager_remove(state, request, &manager_request, output_length),
        MANAGER_OPERATION_ENUMERATE => {
            manager_enumerate(state, request, &manager_request, output_length)
        }
        MANAGER_OPERATION_QUERY => manager_query(state, request, &manager_request, output_length),
        _ => complete(request, STATUS_INVALID_PARAMETER),
    }
}

fn response_buffer(
    request: WDFREQUEST,
    output_length: usize,
    minimum_length: usize,
) -> Result<*mut u8, NTSTATUS> {
    if output_length < minimum_length || output_length > MAX_MANAGER_OUTPUT {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    let mut output: *mut c_void = core::ptr::null_mut();
    let mut actual_output_length = 0usize;
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfRequestRetrieveOutputBuffer,
            request,
            minimum_length,
            &mut output,
            &mut actual_output_length,
        )
    };
    if status != STATUS_SUCCESS {
        return Err(status);
    }
    if actual_output_length < minimum_length || actual_output_length > MAX_MANAGER_OUTPUT {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    Ok(output.cast())
}

fn initialize_response(
    output: *mut u8,
    request: &ManagerRequest,
    status: NTSTATUS,
    record_count: u32,
    next_cursor: u32,
) {
    unsafe {
        output
            .cast::<ManagerResponseHeader>()
            .write(ManagerResponseHeader {
                version: MANAGER_PROTOCOL_VERSION,
                operation: request.operation,
                length: core::mem::size_of::<ManagerResponseHeader>() as u32,
                request_id: request.request_id,
                operation_status: status,
                record_count,
                next_cursor,
                reserved: 0,
            });
    }
}

fn manager_create(
    state: *mut BusState,
    request: WDFREQUEST,
    manager_request: &ManagerRequest,
    output_length: usize,
) {
    let output = match response_buffer(
        request,
        output_length,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    ) {
        Ok(output) => output,
        Err(status) => {
            complete(request, status);
            return;
        }
    };
    {
        let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
            complete(request, STATUS_DEVICE_NOT_READY);
            return;
        };
        let existing = unsafe { find_child(state, &manager_request.adapter_guid) };
        if !existing.is_null() && unsafe { (*existing).lifecycle != LIFECYCLE_ABSENT } {
            initialize_response(output, manager_request, STATUS_DEVICE_BUSY, 0, 0);
            complete_with_information(
                request,
                STATUS_SUCCESS,
                core::mem::size_of::<ManagerResponseHeader>(),
            );
            return;
        }
        if !existing.is_null() {
            unsafe {
                (*existing).lifecycle = LIFECYCLE_CREATING;
                (*existing).terminal_status = STATUS_PENDING;
                (*existing).request_id = manager_request.request_id;
            }
        } else {
            let node = allocate_value(ChildNode {
                guid: manager_request.adapter_guid,
                lifecycle: LIFECYCLE_CREATING,
                terminal_status: STATUS_PENDING,
                request_id: manager_request.request_id,
                pdo: core::ptr::null_mut(),
                interface_length: 0,
                interface_name: [0; MAX_INTERFACE_CHARS],
                next: unsafe { (*state).children },
            });
            if node.is_null() {
                initialize_response(output, manager_request, STATUS_INSUFFICIENT_RESOURCES, 0, 0);
                complete_with_information(
                    request,
                    STATUS_SUCCESS,
                    core::mem::size_of::<ManagerResponseHeader>(),
                );
                return;
            }
            unsafe {
                (*state).children = node;
            }
        }
    }

    let mut identification = make_identification(manager_request.adapter_guid);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfChildListAddOrUpdateChildDescriptionAsPresent,
            (*state).child_list,
            &mut identification.header,
            core::ptr::null_mut::<wdk_sys::WDF_CHILD_ADDRESS_DESCRIPTION_HEADER>(),
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(state, &manager_request.adapter_guid, status);
        }
    }
    let (operation_status, record) =
        unsafe { child_record_for(state, &manager_request.adapter_guid) };
    initialize_response(output, manager_request, operation_status, 1, 0);
    unsafe {
        let record_output = output
            .add(core::mem::size_of::<ManagerResponseHeader>())
            .cast::<ManagerRecord>();
        copy_record(record_output, &record);
        (*output.cast::<ManagerResponseHeader>()).length =
            (core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>())
                as u32;
    }
    complete_with_information(
        request,
        STATUS_SUCCESS,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    );
}

fn manager_remove(
    state: *mut BusState,
    request: WDFREQUEST,
    manager_request: &ManagerRequest,
    output_length: usize,
) {
    let output = match response_buffer(
        request,
        output_length,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    ) {
        Ok(output) => output,
        Err(status) => {
            complete(request, status);
            return;
        }
    };
    let remove_without_pdo;
    {
        let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
            complete(request, STATUS_DEVICE_NOT_READY);
            return;
        };
        let node = unsafe { find_child(state, &manager_request.adapter_guid) };
        if node.is_null() || unsafe { (*node).lifecycle == LIFECYCLE_ABSENT } {
            initialize_response(output, manager_request, STATUS_OBJECT_NAME_NOT_FOUND, 0, 0);
            complete_with_information(
                request,
                STATUS_SUCCESS,
                core::mem::size_of::<ManagerResponseHeader>(),
            );
            return;
        }
        if unsafe {
            (*node).lifecycle == LIFECYCLE_CREATING || (*node).lifecycle == LIFECYCLE_REMOVING
        } {
            initialize_response(output, manager_request, STATUS_DEVICE_BUSY, 0, 0);
            complete_with_information(
                request,
                STATUS_SUCCESS,
                core::mem::size_of::<ManagerResponseHeader>(),
            );
            return;
        }
        unsafe {
            remove_without_pdo = (*node).pdo.is_null();
            (*node).lifecycle = LIFECYCLE_REMOVING;
            (*node).terminal_status = STATUS_PENDING;
            (*node).request_id = manager_request.request_id;
        }
    }
    let mut identification = make_identification(manager_request.adapter_guid);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfChildListUpdateChildDescriptionAsMissing,
            (*state).child_list,
            &mut identification.header,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            mark_child_failure(state, &manager_request.adapter_guid, status);
        }
    } else if remove_without_pdo {
        unsafe {
            reclaim_removed_child_without_pdo(state, &manager_request.adapter_guid);
        }
    }
    let (operation_status, record) =
        unsafe { child_record_for(state, &manager_request.adapter_guid) };
    initialize_response(output, manager_request, operation_status, 1, 0);
    unsafe {
        let record_output = output
            .add(core::mem::size_of::<ManagerResponseHeader>())
            .cast::<ManagerRecord>();
        copy_record(record_output, &record);
        (*output.cast::<ManagerResponseHeader>()).length =
            (core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>())
                as u32;
    }
    complete_with_information(
        request,
        STATUS_SUCCESS,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    );
}

unsafe fn child_record_for(state: *mut BusState, guid: &GUID) -> (NTSTATUS, ManagerRecord) {
    let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
        return (STATUS_DEVICE_NOT_READY, empty_record());
    };
    let node = unsafe { find_child(state, guid) };
    if node.is_null() {
        return (STATUS_SUCCESS, absent_record(guid, STATUS_SUCCESS, 0));
    }
    let mut record = empty_record();
    unsafe {
        if (*node).lifecycle == LIFECYCLE_CREATING && !(*node).pdo.is_null() {
            (*node).lifecycle = LIFECYCLE_ACTIVE;
            (*node).terminal_status = STATUS_SUCCESS;
        }
        write_record(&mut record, &*node);
        ((*node).terminal_status, record)
    }
}

fn absent_record(guid: &GUID, terminal_status: NTSTATUS, request_id: u64) -> ManagerRecord {
    ManagerRecord {
        adapter_guid: *guid,
        lifecycle: LIFECYCLE_ABSENT,
        terminal_status,
        request_id,
        interface_length: 0,
        _padding: 0,
        interface_name: [0; MAX_INTERFACE_CHARS],
    }
}

fn empty_record() -> ManagerRecord {
    absent_record(&GUID::default(), STATUS_OBJECT_NAME_NOT_FOUND, 0)
}

fn manager_query(
    state: *mut BusState,
    request: WDFREQUEST,
    manager_request: &ManagerRequest,
    output_length: usize,
) {
    let output = match response_buffer(
        request,
        output_length,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    ) {
        Ok(output) => output,
        Err(status) => {
            complete(request, status);
            return;
        }
    };
    let (operation_status, record) =
        unsafe { child_record_for(state, &manager_request.adapter_guid) };
    initialize_response(output, manager_request, operation_status, 1, 0);
    unsafe {
        let record_output = output
            .add(core::mem::size_of::<ManagerResponseHeader>())
            .cast::<ManagerRecord>();
        copy_record(record_output, &record);
        (*output.cast::<ManagerResponseHeader>()).length =
            (core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>())
                as u32;
    }
    complete_with_information(
        request,
        STATUS_SUCCESS,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    );
}

fn manager_enumerate(
    state: *mut BusState,
    request: WDFREQUEST,
    manager_request: &ManagerRequest,
    output_length: usize,
) {
    let output = match response_buffer(
        request,
        output_length,
        core::mem::size_of::<ManagerResponseHeader>() + core::mem::size_of::<ManagerRecord>(),
    ) {
        Ok(output) => output,
        Err(status) => {
            complete(request, status);
            return;
        }
    };
    let header_size = core::mem::size_of::<ManagerResponseHeader>();
    let record_size = core::mem::size_of::<ManagerRecord>();
    let capacity = (output_length - header_size) / record_size;
    let Some(_guard) = (unsafe { BusStateGuard::acquire(state) }) else {
        complete(request, STATUS_DEVICE_NOT_READY);
        return;
    };
    let mut index = 0u32;
    let mut returned = 0usize;
    let mut node = unsafe { (*state).children };
    while !node.is_null() && returned < capacity {
        let include = unsafe { (*node).lifecycle != LIFECYCLE_ABSENT };
        if include && index >= manager_request.cursor {
            let record_output = unsafe {
                output
                    .add(header_size + returned * record_size)
                    .cast::<ManagerRecord>()
            };
            unsafe {
                write_record(record_output, &*node);
            }
            returned += 1;
        }
        if include {
            index = index.saturating_add(1);
        }
        node = unsafe { (*node).next };
    }
    let next_cursor = if node.is_null() { 0 } else { index };
    initialize_response(
        output,
        manager_request,
        STATUS_SUCCESS,
        returned as u32,
        next_cursor,
    );
    unsafe {
        (*output.cast::<ManagerResponseHeader>()).length =
            (header_size + returned * record_size) as u32;
    }
    complete_with_information(
        request,
        STATUS_SUCCESS,
        header_size + returned * record_size,
    );
}

unsafe extern "C" fn evt_bus_device_cleanup(object: WDFOBJECT) {
    let context =
        unsafe { object_context::<BusDeviceContext>(object, &raw const BUS_CONTEXT_TYPE_INFO) };
    if context.is_null() || unsafe { (*context).state.is_null() } {
        return;
    }
    let state = unsafe { (*context).state };
    unsafe {
        (*context).state = core::ptr::null_mut();
    }
    let mut node = unsafe { (*state).children };
    while !node.is_null() {
        let next = unsafe { (*node).next };
        unsafe {
            drop(Box::from_raw(node));
        }
        node = next;
    }
    unsafe {
        drop(Box::from_raw(state));
    }
}
