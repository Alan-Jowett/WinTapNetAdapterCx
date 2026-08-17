use core::ffi::c_void;

use netadaptercx_sys::{NET_EXTENSION, NET_FRAGMENT, NET_FRAGMENT_VIRTUAL_ADDRESS, NET_PACKET, NET_RING};

pub const PACKET_RING_INDEX: usize = 0;
pub const FRAGMENT_RING_INDEX: usize = 1;

pub fn increment_index(ring: &NET_RING, index: u32) -> Option<u32> {
    if !ring_is_valid(ring) || index & !ring.ElementIndexMask != 0 {
        return None;
    }
    Some((index + 1) & ring.ElementIndexMask)
}

pub fn advance_index(ring: &NET_RING, index: u32, count: u32) -> Option<u32> {
    if !ring_is_valid(ring) || index & !ring.ElementIndexMask != 0 {
        return None;
    }
    Some(index.wrapping_add(count) & ring.ElementIndexMask)
}

pub unsafe fn packet_at(ring: *mut NET_RING, index: u32) -> Option<*mut NET_PACKET> {
    let element = unsafe { element_at(ring, index, core::mem::size_of::<NET_PACKET>()) }?;
    Some(element.cast())
}

pub unsafe fn fragment_at(ring: *mut NET_RING, index: u32) -> Option<*mut NET_FRAGMENT> {
    let element = unsafe { element_at(ring, index, core::mem::size_of::<NET_FRAGMENT>()) }?;
    Some(element.cast())
}

pub unsafe fn fragment_virtual_address(
    extension: &NET_EXTENSION,
    index: u32,
) -> Option<*mut NET_FRAGMENT_VIRTUAL_ADDRESS> {
    if unsafe { extension.__bindgen_anon_1.Enabled } == 0 {
        return None;
    }
    let base = extension.Reserved[0].cast::<NET_FRAGMENT_VIRTUAL_ADDRESS>();
    if base.is_null() {
        return None;
    }
    // NetExtensionGetData uses Reserved[1] as the extension's byte stride.
    let stride = extension.Reserved[1] as usize;
    let offset = (index as usize).checked_mul(stride)?;
    Some(unsafe { base.cast::<u8>().add(offset).cast() })
}

pub unsafe fn element_at(
    ring: *mut NET_RING,
    index: u32,
    minimum_size: usize,
) -> Option<*mut c_void> {
    let ring = unsafe { ring.as_ref()? };
    if !ring_is_valid(ring)
        || minimum_size > ring.ElementStride as usize
        || index & !ring.ElementIndexMask != 0
    {
        return None;
    }

    let slot = (index & ring.ElementIndexMask) as usize;
    let offset = slot.checked_mul(ring.ElementStride as usize)?;
    let buffer = ring.Buffer.as_ptr() as *mut u8;
    Some(unsafe { buffer.add(offset).cast() })
}

fn ring_is_valid(ring: &NET_RING) -> bool {
    ring.NumberOfElements != 0
        && ring.ElementIndexMask == ring.NumberOfElements - 1
        && ring.ElementStride != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> NET_RING {
        NET_RING {
            ElementStride: 16,
            NumberOfElements: 4,
            ElementIndexMask: 3,
            Buffer: [0],
            ..NET_RING::default()
        }
    }

    #[test]
    fn wraps_indices_at_ring_mask() {
        let ring = ring();
        assert_eq!(increment_index(&ring, 3), Some(0));
        assert_eq!(advance_index(&ring, 3, 5), Some(0));
    }

    #[test]
    fn rejects_invalid_indices_and_ring_shape() {
        let mut ring = ring();
        assert_eq!(increment_index(&ring, 4), None);
        ring.NumberOfElements = 3;
        assert_eq!(increment_index(&ring, 0), None);
    }

    #[test]
    fn uses_the_extension_byte_stride() {
        let mut storage = [0_u64; 6];
        let extension = NET_EXTENSION {
            Reserved: [
                storage.as_mut_ptr().cast(),
                16_usize as *mut c_void,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            ],
            __bindgen_anon_1: netadaptercx_sys::_NET_EXTENSION__bindgen_ty_1 { Enabled: 1 },
        };

        let address = unsafe { fragment_virtual_address(&extension, 2) }.unwrap();
        assert_eq!(
            address.cast::<u8>(),
            unsafe { storage.as_mut_ptr().cast::<u8>().add(32) }
        );
    }
}
