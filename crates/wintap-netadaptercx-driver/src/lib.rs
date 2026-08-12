#![no_std]

extern crate alloc;

#[cfg(not(test))]
extern crate wdk_panic;

mod frame_queue;
use frame_queue::{Frame, FrameQueue, QueueError};

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::_WDF_IO_QUEUE_DISPATCH_TYPE::WdfIoQueueDispatchParallel;
use wdk_sys::{
    DRIVER_OBJECT, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, ULONG, UNICODE_STRING,
    WDF_DRIVER_CONFIG, WDF_FILEOBJECT_CONFIG, WDF_IO_QUEUE_CONFIG, WDF_NO_HANDLE,
    WDF_NO_OBJECT_ATTRIBUTES, WDF_OBJECT_ATTRIBUTES, WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_WORKITEM_CONFIG, WDFCMRESLIST, WDFDEVICE, WDFDEVICE_INIT, WDFDRIVER, WDFFILEOBJECT,
    WDFQUEUE, WDFREQUEST, WDFSPINLOCK, WDFWORKITEM, call_unsafe_wdf_function_binding,
};

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_CANCELLED: NTSTATUS = 0xC000_0120_u32 as i32;
const STATUS_DEVICE_BUSY: NTSTATUS = 0xC000_00E8_u32 as i32;
const STATUS_DEVICE_NOT_READY: NTSTATUS = 0xC000_00A3_u32 as i32;
const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009A_u32 as i32;
const STATUS_INVALID_BUFFER_SIZE: NTSTATUS = 0xC000_0206_u32 as i32;
const STATUS_NOT_SUPPORTED: NTSTATUS = 0xC000_00BB_u32 as i32;
const PENDING_READ_LIMIT: usize = 64;
const PENDING_WRITE_LIMIT: usize = 64;
const FRAME_QUEUE_LIMIT: usize = 256;
const FRAME_MINIMUM: usize = 14;
const FRAME_MAXIMUM: usize = 1514;
const CONTROL_DEVICE_NAME: [u16; 27] = [
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
    b'N' as u16,
    b'e' as u16,
    b't' as u16,
    b'A' as u16,
    b'd' as u16,
    b'a' as u16,
    b'p' as u16,
    b't' as u16,
    b'e' as u16,
    b'r' as u16,
    b'C' as u16,
    b'x' as u16,
    0,
];
const CONTROL_SYMBOLIC_LINK: [u16; 31] = [
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
    b'W' as u16,
    b'i' as u16,
    b'n' as u16,
    b'T' as u16,
    b'a' as u16,
    b'p' as u16,
    b'N' as u16,
    b'e' as u16,
    b't' as u16,
    b'A' as u16,
    b'd' as u16,
    b'a' as u16,
    b'p' as u16,
    b't' as u16,
    b'e' as u16,
    b'r' as u16,
    b'C' as u16,
    b'x' as u16,
    0,
];
const CONTROL_SDDL: [u16; 15] = [
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
    b'B' as u16,
    b'A' as u16,
    b')' as u16,
    0,
];

static mut ADAPTER: netadaptercx_sys::NETADAPTER = core::ptr::null_mut();
static mut CONTROL_DEVICE: WDFDEVICE = core::ptr::null_mut();
static mut READ_QUEUE: WDFQUEUE = core::ptr::null_mut();
static mut WRITE_QUEUE: WDFQUEUE = core::ptr::null_mut();
static mut FRAME_LOCK: WDFSPINLOCK = core::ptr::null_mut();
static mut FRAME_QUEUE: Option<FrameQueue> = None;
static mut READ_WORK_ITEM: WDFWORKITEM = core::ptr::null_mut();
static mut WRITE_WORK_ITEM: WDFWORKITEM = core::ptr::null_mut();
static PENDING_READS: AtomicUsize = AtomicUsize::new(0);
static PENDING_WRITES: AtomicUsize = AtomicUsize::new(0);

const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_LINK_STATE>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_RX_CAPABILITIES>();
const _: usize = core::mem::size_of::<netadaptercx_sys::NET_PACKET_QUEUE_CONFIG>();

/// Required WDF driver entry point.
///
/// IRQL: PASSIVE_LEVEL. This scaffold may not unwind; Cargo profiles use
/// `panic = "abort"` and `wdk-panic` supplies the kernel panic handler.
#[unsafe(export_name = "DriverEntry")]
pub unsafe extern "system" fn driver_entry(
    driver: &mut DRIVER_OBJECT,
    registry_path: PCUNICODE_STRING,
) -> NTSTATUS {
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
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDriverCreate,
            driver as PDRIVER_OBJECT,
            registry_path,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut driver_config,
            WDF_NO_HANDLE.cast::<WDFDRIVER>(),
        )
    };

    if status == STATUS_SUCCESS {
        STATUS_SUCCESS
    } else {
        status
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
    let status = unsafe {
        // SAFETY: NetDeviceInitConfig is called once at PASSIVE_LEVEL before
        // the WDF device is created.
        net_call_device_init_config(device_init)
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut pnp_callbacks = WDF_PNPPOWER_EVENT_CALLBACKS {
        Size: core::mem::size_of::<WDF_PNPPOWER_EVENT_CALLBACKS>() as ULONG,
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
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreate,
            &mut pnp_init,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut _pnp_device,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    let status = create_control_device(driver);
    if status != STATUS_SUCCESS {
        return status;
    }

    // SAFETY: The WDF device has been created and this callback runs at
    // PASSIVE_LEVEL during device addition.
    unsafe { create_adapter(_pnp_device) }
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

unsafe fn create_adapter(device: WDFDEVICE) -> NTSTATUS {
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
    // SAFETY: The adapter-init object is consumed by NetAdapterCreate.
    let status = unsafe {
        create(
            netadaptercx_sys::NetDriverGlobals,
            adapter_init,
            core::ptr::null_mut(),
            &mut adapter,
        )
    };
    if status == STATUS_SUCCESS {
        unsafe {
            ADAPTER = adapter;
        }
    } else {
        let free: unsafe extern "system" fn(
            netadaptercx_sys::PNET_DRIVER_GLOBALS,
            *mut netadaptercx_sys::NETADAPTER_INIT,
        ) = unsafe {
            net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterInitFreeTableIndex as usize)
        };
        // SAFETY: AdapterCreate did not consume the failed init object.
        unsafe { free(netadaptercx_sys::NetDriverGlobals, adapter_init) };
    }
    status
}

extern "C" fn evt_create_tx_queue(
    _adapter: netadaptercx_sys::NETADAPTER,
    _queue_init: *mut netadaptercx_sys::NETTXQUEUE_INIT,
) -> NTSTATUS {
    STATUS_NOT_SUPPORTED
}

extern "C" fn evt_create_rx_queue(
    _adapter: netadaptercx_sys::NETADAPTER,
    _queue_init: *mut netadaptercx_sys::NETRXQUEUE_INIT,
) -> NTSTATUS {
    STATUS_NOT_SUPPORTED
}

extern "C" fn evt_device_prepare_hardware(
    _device: WDFDEVICE,
    _resources_raw: WDFCMRESLIST,
    _resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let adapter = unsafe { ADAPTER };
    if adapter.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    let mut tx = netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES {
        Size: core::mem::size_of::<netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES>() as ULONG,
        MaximumNumberOfFragments: u64::MAX,
        MaximumNumberOfQueues: 1,
        ..netadaptercx_sys::NET_ADAPTER_TX_CAPABILITIES::default()
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
        set_mtu(
            netadaptercx_sys::NetDriverGlobals,
            adapter,
            (FRAME_MAXIMUM - FRAME_MINIMUM) as ULONG,
        );
    }

    let address = netadaptercx_sys::NET_ADAPTER_LINK_LAYER_ADDRESS {
        Length: 6,
        Address: [
            0x02, 0x57, 0x54, 0x41, 0x50, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0,
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

    let start: unsafe extern "system" fn(
        netadaptercx_sys::PNET_DRIVER_GLOBALS,
        netadaptercx_sys::NETADAPTER,
    ) -> NTSTATUS =
        unsafe { net_function(netadaptercx_sys::_NETFUNCENUM_NetAdapterStartTableIndex as usize) };
    // SAFETY: Adapter was created by NetAdapterCx and is started once here.
    unsafe { start(netadaptercx_sys::NetDriverGlobals, adapter) }
}

extern "C" fn evt_device_release_hardware(
    _device: WDFDEVICE,
    _resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let adapter = unsafe {
        let adapter = ADAPTER;
        ADAPTER = core::ptr::null_mut();
        adapter
    };
    let (read_queue, write_queue) = unsafe { (READ_QUEUE, WRITE_QUEUE) };
    let control_device = unsafe {
        let device = CONTROL_DEVICE;
        CONTROL_DEVICE = core::ptr::null_mut();
        device
    };

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
    clear_frame_queue();
    PENDING_READS.store(0, Ordering::Release);
    PENDING_WRITES.store(0, Ordering::Release);

    if !control_device.is_null() {
        // SAFETY: The control device was created by this driver and is
        // deleted exactly once after its global handle is cleared.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, control_device.cast());
        }
    }
    unsafe {
        READ_QUEUE = core::ptr::null_mut();
        WRITE_QUEUE = core::ptr::null_mut();
        FRAME_LOCK = core::ptr::null_mut();
        READ_WORK_ITEM = core::ptr::null_mut();
        WRITE_WORK_ITEM = core::ptr::null_mut();
        FRAME_QUEUE = None;
    }

    STATUS_SUCCESS
}

fn create_control_device(driver: WDFDRIVER) -> NTSTATUS {
    let sddl = unicode_string(&CONTROL_SDDL);
    let mut control_init = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfControlDeviceInitAllocate,
            driver,
            &sddl as *const UNICODE_STRING,
        )
    };
    if control_init.is_null() {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    let device_name = unicode_string(&CONTROL_DEVICE_NAME);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceInitAssignName,
            control_init,
            &device_name as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfDeviceInitFree, control_init);
        }
        return status;
    }

    unsafe {
        call_unsafe_wdf_function_binding!(WdfDeviceInitSetExclusive, control_init, 1);
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
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut frame_lock: WDFSPINLOCK = core::ptr::null_mut();
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfSpinLockCreate,
            WDF_NO_OBJECT_ATTRIBUTES,
            &mut frame_lock,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    let mut default_queue_config = WDF_IO_QUEUE_CONFIG {
        Size: core::mem::size_of::<WDF_IO_QUEUE_CONFIG>() as ULONG,
        DispatchType: WdfIoQueueDispatchParallel,
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
        return status;
    }

    let mut work_item_attributes = WDF_OBJECT_ATTRIBUTES {
        Size: core::mem::size_of::<WDF_OBJECT_ATTRIBUTES>() as ULONG,
        ParentObject: device.cast(),
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
        return status;
    }

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
        return status;
    }

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
        return status;
    }

    let symbolic_link = unicode_string(&CONTROL_SYMBOLIC_LINK);
    let status = unsafe {
        call_unsafe_wdf_function_binding!(
            WdfDeviceCreateSymbolicLink,
            device,
            &symbolic_link as *const UNICODE_STRING,
        )
    };
    if status != STATUS_SUCCESS {
        return status;
    }

    unsafe {
        call_unsafe_wdf_function_binding!(WdfControlFinishInitializing, device);
        CONTROL_DEVICE = device;
        READ_QUEUE = read_queue;
        WRITE_QUEUE = write_queue;
        FRAME_LOCK = frame_lock;
        FRAME_QUEUE = Some(FrameQueue::new(FRAME_QUEUE_LIMIT));
        READ_WORK_ITEM = read_work_item;
        WRITE_WORK_ITEM = write_work_item;
    }
    STATUS_SUCCESS
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

extern "C" fn evt_file_cleanup(_file_object: WDFFILEOBJECT) {}

extern "C" fn evt_io_read(_queue: WDFQUEUE, request: WDFREQUEST, _length: usize) {
    if !try_admit(&PENDING_READS, PENDING_READ_LIMIT) {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let target = unsafe { READ_QUEUE };
    if !forward_request(request, target) {
        release_request(&PENDING_READS);
    } else {
        enqueue_work_item(unsafe { READ_WORK_ITEM });
    }
}

extern "C" fn evt_io_write(_queue: WDFQUEUE, request: WDFREQUEST, _length: usize) {
    if !try_admit(&PENDING_WRITES, PENDING_WRITE_LIMIT) {
        complete_request(request, STATUS_DEVICE_BUSY);
        return;
    }
    let target = unsafe { WRITE_QUEUE };
    if !forward_request(request, target) {
        release_request(&PENDING_WRITES);
    } else {
        enqueue_work_item(unsafe { WRITE_WORK_ITEM });
    }
}

extern "C" fn evt_io_stop(queue: WDFQUEUE, request: WDFREQUEST, _action_flags: ULONG) {
    let read_queue = unsafe { READ_QUEUE };
    let write_queue = unsafe { WRITE_QUEUE };
    if queue == read_queue {
        release_request(&PENDING_READS);
    } else if queue == write_queue {
        release_request(&PENDING_WRITES);
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

fn enqueue_work_item(work_item: WDFWORKITEM) {
    if !work_item.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemEnqueue, work_item);
        }
    }
}

extern "C" fn evt_write_drain_work_item(_work_item: WDFWORKITEM) {
    loop {
        let mut request = core::ptr::null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoQueueRetrieveNextRequest,
                WRITE_QUEUE,
                &mut request,
            )
        };
        if status != STATUS_SUCCESS {
            return;
        }
        release_request(&PENDING_WRITES);

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
        match enqueue_frame(bytes) {
            Ok(()) => {
                complete_request_with_information(request, STATUS_SUCCESS, input_length);
                enqueue_work_item(unsafe { READ_WORK_ITEM });
            }
            Err(QueueError::Full) => complete_request(request, STATUS_DEVICE_BUSY),
            Err(QueueError::Closed) => complete_request(request, STATUS_DEVICE_NOT_READY),
            Err(QueueError::InvalidFrameLength) => {
                complete_request(request, STATUS_INVALID_BUFFER_SIZE)
            }
        }
    }
}

extern "C" fn evt_read_completion_work_item(_work_item: WDFWORKITEM) {
    loop {
        let mut request = core::ptr::null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoQueueRetrieveNextRequest,
                READ_QUEUE,
                &mut request,
            )
        };
        if status != STATUS_SUCCESS {
            return;
        }
        release_request(&PENDING_READS);

        let frame = match dequeue_frame() {
            Some(frame) => frame,
            None => {
                let target = unsafe { READ_QUEUE };
                if forward_request(request, target) {
                    PENDING_READS.fetch_add(1, Ordering::AcqRel);
                } else {
                    complete_request(request, STATUS_DEVICE_NOT_READY);
                }
                return;
            }
        };

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
            let _ = enqueue_existing_frame(frame);
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

fn enqueue_frame(bytes: &[u8]) -> Result<(), QueueError> {
    let frame = Frame::from_bytes(bytes)?;
    let lock = unsafe { FRAME_LOCK };
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let result = (*core::ptr::addr_of_mut!(FRAME_QUEUE))
            .as_mut()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.enqueue(frame));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        result
    }
}

fn enqueue_existing_frame(frame: Frame) -> Result<(), QueueError> {
    let lock = unsafe { FRAME_LOCK };
    if lock.is_null() {
        return Err(QueueError::Closed);
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let result = (*core::ptr::addr_of_mut!(FRAME_QUEUE))
            .as_mut()
            .ok_or(QueueError::Closed)
            .and_then(|queue| queue.enqueue(frame));
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        result
    }
}

fn dequeue_frame() -> Option<Frame> {
    let lock = unsafe { FRAME_LOCK };
    if lock.is_null() {
        return None;
    }
    unsafe {
        call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
        let frame = (*core::ptr::addr_of_mut!(FRAME_QUEUE))
            .as_mut()
            .and_then(FrameQueue::dequeue);
        call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        frame
    }
}

fn clear_frame_queue() {
    let lock = unsafe { FRAME_LOCK };
    if !lock.is_null() {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, lock);
            if let Some(queue) = (*core::ptr::addr_of_mut!(FRAME_QUEUE)).as_mut() {
                queue.close();
            }
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, lock);
        }
    }
}
fn unicode_string(buffer: &[u16]) -> UNICODE_STRING {
    let byte_length = (buffer.len() - 1) * core::mem::size_of::<u16>();
    UNICODE_STRING {
        Length: byte_length as u16,
        MaximumLength: (byte_length + core::mem::size_of::<u16>()) as u16,
        Buffer: buffer.as_ptr() as *mut u16,
    }
}
