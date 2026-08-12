#![no_std]

extern crate alloc;

#[cfg(not(test))]
extern crate wdk_panic;

mod frame_queue;

#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk_sys::{
    DRIVER_OBJECT, NTSTATUS, PCUNICODE_STRING, PDRIVER_OBJECT, ULONG, UNICODE_STRING,
    WDF_DRIVER_CONFIG, WDF_NO_HANDLE, WDF_NO_OBJECT_ATTRIBUTES, WDFDEVICE, WDFDEVICE_INIT,
    WDFDRIVER,
    call_unsafe_wdf_function_binding,
};

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

const STATUS_SUCCESS: NTSTATUS = 0;
const STATUS_INSUFFICIENT_RESOURCES: NTSTATUS = 0xC000_009A_u32 as i32;
const CONTROL_DEVICE_NAME: [u16; 27] = [
    b'\\' as u16, b'D' as u16, b'e' as u16, b'v' as u16, b'i' as u16, b'c' as u16,
    b'e' as u16, b'\\' as u16, b'W' as u16, b'i' as u16, b'n' as u16, b'T' as u16,
    b'a' as u16, b'p' as u16, b'N' as u16, b'e' as u16, b't' as u16, b'A' as u16,
    b'd' as u16, b'a' as u16, b'p' as u16, b't' as u16, b'e' as u16, b'r' as u16,
    b'C' as u16, b'x' as u16, 0,
];
const CONTROL_SYMBOLIC_LINK: [u16; 31] = [
    b'\\' as u16, b'D' as u16, b'o' as u16, b's' as u16, b'D' as u16, b'e' as u16,
    b'v' as u16, b'i' as u16, b'c' as u16, b'e' as u16, b's' as u16, b'\\' as u16,
    b'W' as u16, b'i' as u16, b'n' as u16, b'T' as u16, b'a' as u16, b'p' as u16,
    b'N' as u16, b'e' as u16, b't' as u16, b'A' as u16, b'd' as u16, b'a' as u16,
    b'p' as u16, b't' as u16, b'e' as u16, b'r' as u16, b'C' as u16, b'x' as u16, 0,
];
const CONTROL_SDDL: [u16; 15] = [
    b'D' as u16, b':' as u16, b'P' as u16, b'(' as u16, b'A' as u16, b';' as u16,
    b';' as u16, b'G' as u16, b'A' as u16, b';' as u16, b';' as u16, b'B' as u16,
    b'A' as u16, b')' as u16, 0,
];

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

    create_control_device(driver)
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
    }
    STATUS_SUCCESS
}

fn unicode_string(buffer: &[u16]) -> UNICODE_STRING {
    let byte_length = (buffer.len() - 1) * core::mem::size_of::<u16>();
    UNICODE_STRING {
        Length: byte_length as u16,
        MaximumLength: (byte_length + core::mem::size_of::<u16>()) as u16,
        Buffer: buffer.as_ptr() as *mut u16,
    }
}
