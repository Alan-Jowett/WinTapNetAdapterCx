#![no_std]

extern crate alloc;

#[cfg(not(test))]
extern crate wdk_panic;

mod frame_queue;

#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    DRIVER_OBJECT, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, ULONG, WDF_DRIVER_CONFIG,
    WDF_NO_HANDLE, WDF_NO_OBJECT_ATTRIBUTES, WDFDEVICE_INIT, WDFDRIVER,
    call_unsafe_wdf_function_binding,
};

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_NOT_SUPPORTED: NTSTATUS = 0xC000_00BB_u32 as i32;

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

/// Device-add callback for the Phase 5 Rust migration scaffold.
///
/// IRQL: PASSIVE_LEVEL. Returning `STATUS_NOT_SUPPORTED` prevents this
/// scaffold from binding to hardware until the full TAP/NetAdapterCx port is
/// completed and verified.
extern "C" fn evt_driver_device_add(
    _driver: WDFDRIVER,
    _device_init: *mut WDFDEVICE_INIT,
) -> NTSTATUS {
    STATUS_NOT_SUPPORTED
}
