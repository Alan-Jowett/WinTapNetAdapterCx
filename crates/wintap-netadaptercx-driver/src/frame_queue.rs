// SPDX-License-Identifier: MIT
// Copyright (c) 2026 WinTapNetAdapterCx contributors
extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

use core::mem::MaybeUninit;
use core::ptr::null_mut;

use wdk_sys::WDFLOOKASIDE;
#[cfg(not(test))]
use wdk_sys::{WDFMEMORY, call_unsafe_wdf_function_binding};

pub const FRAME_MINIMUM: usize = 14;
pub const FRAME_MAXIMUM: usize = 65_535;
pub const FRAME_STORAGE_SIZE: usize = core::mem::size_of::<FrameStorage>();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueError {
    InvalidFrameLength,
    Full,
    Closed,
    InsufficientResources,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueState {
    Open,
    Closing,
    Closed,
}

#[repr(C)]
struct FrameStorage {
    length: usize,
    data: [u8; FRAME_MAXIMUM],
}

pub struct Frame {
    #[cfg(not(test))]
    memory: WDFMEMORY,
    #[cfg(not(test))]
    storage: *mut FrameStorage,
    #[cfg(test)]
    data: Vec<u8>,
    #[cfg(test)]
    length: usize,
}

impl Frame {
    #[cfg(test)]
    pub fn new(_pool: WDFLOOKASIDE) -> Result<Self, QueueError> {
        let mut data = Vec::new();
        data.try_reserve_exact(FRAME_MAXIMUM)
            .map_err(|_| QueueError::InsufficientResources)?;
        data.resize(FRAME_MAXIMUM, 0);
        Ok(Self { data, length: 0 })
    }

    #[cfg(not(test))]
    pub fn new(pool: WDFLOOKASIDE) -> Result<Self, QueueError> {
        if pool.is_null() {
            return Err(QueueError::Closed);
        }
        let mut memory = null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfMemoryCreateFromLookaside,
                pool,
                &mut memory,
            )
        };
        if status != 0 || memory.is_null() {
            return Err(QueueError::InsufficientResources);
        }

        let mut size = 0;
        let storage = unsafe {
            call_unsafe_wdf_function_binding!(WdfMemoryGetBuffer, memory, &mut size)
        } as *mut FrameStorage;
        if storage.is_null() || size < core::mem::size_of::<FrameStorage>() {
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, memory.cast());
            }
            return Err(QueueError::InsufficientResources);
        }

        unsafe {
            (*storage).length = 0;
        }
        Ok(Self {
            memory,
            storage,
        })
    }

    #[cfg(not(test))]
    pub fn from_bytes(pool: WDFLOOKASIDE, data: &[u8]) -> Result<Self, QueueError> {
        if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&data.len()) {
            return Err(QueueError::InvalidFrameLength);
        }
        let mut frame = Self::new(pool)?;
        frame.copy_from_slice(0, data)?;
        frame.set_length(data.len());
        Ok(frame)
    }

    #[cfg(not(test))]
    pub fn copy_from_slice(&mut self, offset: usize, data: &[u8]) -> Result<(), QueueError> {
        if offset > FRAME_MAXIMUM || data.len() > FRAME_MAXIMUM - offset {
            return Err(QueueError::InvalidFrameLength);
        }
        unsafe {
            (*self.storage).data[offset..offset + data.len()].copy_from_slice(data);
        }
        Ok(())
    }

    #[cfg(not(test))]
    pub fn set_length(&mut self, length: usize) {
        unsafe {
            (*self.storage).length = length;
        }
    }

    #[cfg(test)]
    pub fn from_bytes(_pool: WDFLOOKASIDE, data: &[u8]) -> Result<Self, QueueError> {
        if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&data.len()) {
            return Err(QueueError::InvalidFrameLength);
        }

        let mut copied = Vec::new();
        copied
            .try_reserve_exact(data.len())
            .map_err(|_| QueueError::InsufficientResources)?;
        copied.extend_from_slice(data);
        Ok(Self {
            data: copied,
            length: data.len(),
        })
    }

    #[cfg(test)]
    pub fn copy_from_slice(&mut self, offset: usize, data: &[u8]) -> Result<(), QueueError> {
        if offset > FRAME_MAXIMUM || data.len() > FRAME_MAXIMUM - offset {
            return Err(QueueError::InvalidFrameLength);
        }
        self.data[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    #[cfg(test)]
    pub fn set_length(&mut self, length: usize) {
        self.length = length;
    }

    pub fn as_bytes(&self) -> &[u8] {
        #[cfg(not(test))]
        unsafe {
            &(*self.storage).data[..(*self.storage).length]
        }
        #[cfg(test)]
        &self.data[..self.length]
    }
}

#[cfg(not(test))]
impl Drop for Frame {
    fn drop(&mut self) {
        if !self.memory.is_null() {
            unsafe {
                (*self.storage).data[..(*self.storage).length].fill(0);
                (*self.storage).length = 0;
                call_unsafe_wdf_function_binding!(WdfObjectDelete, self.memory.cast());
            }
        }
    }
}

pub struct FrameQueue {
    frames: Box<[MaybeUninit<Frame>]>,
    head: usize,
    length: usize,
    limit: usize,
    byte_limit: usize,
    bytes: usize,
    state: QueueState,
}

impl FrameQueue {
    pub fn try_new(limit: usize, byte_limit: usize) -> Result<Self, QueueError> {
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(limit)
            .map_err(|_| QueueError::InsufficientResources)?;
        frames.resize_with(limit, MaybeUninit::uninit);

        Ok(Self {
            frames: frames.into_boxed_slice(),
            head: 0,
            length: 0,
            limit,
            byte_limit,
            bytes: 0,
            state: QueueState::Open,
        })
    }

    pub fn enqueue(&mut self, frame: Frame) -> Result<(), QueueError> {
        if self.state != QueueState::Open {
            return Err(QueueError::Closed);
        }
        let remaining_bytes = match self.byte_limit.checked_sub(self.bytes) {
            Some(remaining) => remaining,
            None => return Err(QueueError::Full),
        };
        let frame_length = frame.as_bytes().len();
        if self.length >= self.limit || frame_length > remaining_bytes {
            return Err(QueueError::Full);
        }

        let index = (self.head + self.length) % self.limit;
        self.frames[index].write(frame);
        self.length += 1;
        self.bytes += frame_length;
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<Frame> {
        if self.length == 0 {
            return None;
        }
        let frame = unsafe { self.frames[self.head].assume_init_read() };
        self.head = (self.head + 1) % self.limit;
        self.length -= 1;
        self.bytes -= frame.as_bytes().len();
        Some(frame)
    }

    pub fn begin_close(&mut self) {
        if self.state == QueueState::Open {
            self.state = QueueState::Closing;
        }
    }

    pub fn close(&mut self) {
        self.clear_frames();
        self.state = QueueState::Closed;
    }

    pub fn reopen(&mut self) {
        self.clear_frames();
        self.state = QueueState::Open;
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn state(&self) -> QueueState {
        self.state
    }

    fn clear_frames(&mut self) {
        while self.dequeue().is_some() {}
    }
}

impl Drop for FrameQueue {
    fn drop(&mut self) {
        self.clear_frames();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::from_bytes(null_mut(), &[0; FRAME_MINIMUM]).unwrap()
    }

    #[test]
    fn validates_ethernet_frame_bounds() {
        assert!(matches!(
            Frame::from_bytes(null_mut(), &[0; FRAME_MINIMUM - 1]),
            Err(QueueError::InvalidFrameLength)
        ));
        assert!(Frame::from_bytes(null_mut(), &[0; FRAME_MINIMUM]).is_ok());
        assert!(Frame::from_bytes(null_mut(), &[0; FRAME_MAXIMUM]).is_ok());
        assert!(matches!(
            Frame::from_bytes(null_mut(), &[0; FRAME_MAXIMUM + 1]),
            Err(QueueError::InvalidFrameLength)
        ));
    }

    #[test]
    fn enforces_limit_and_preserves_fifo_ownership() {
        let mut queue = FrameQueue::try_new(1, FRAME_MAXIMUM).unwrap();
        queue.enqueue(frame()).unwrap();
        assert_eq!(queue.enqueue(frame()), Err(QueueError::Full));
        assert_eq!(queue.len(), 1);
        assert!(queue.dequeue().is_some());
        assert!(queue.is_empty());
    }

    #[test]
    fn closing_rejects_new_frames_and_releases_queued_frames() {
        let mut queue = FrameQueue::try_new(2, FRAME_MAXIMUM * 2).unwrap();
        queue.enqueue(frame()).unwrap();
        queue.begin_close();
        assert_eq!(queue.state(), QueueState::Closing);
        assert_eq!(queue.enqueue(frame()), Err(QueueError::Closed));
    }

    #[test]
    fn enforces_byte_budget_and_releases_bytes_on_dequeue() {
        let mut queue = FrameQueue::try_new(2, FRAME_MINIMUM).unwrap();
        queue.enqueue(frame()).unwrap();
        assert_eq!(queue.enqueue(frame()), Err(QueueError::Full));
        assert!(queue.dequeue().is_some());
        queue.enqueue(frame()).unwrap();
        queue.close();
        assert_eq!(queue.state(), QueueState::Closed);
        assert!(queue.is_empty());
    }

    #[test]
    fn reopen_discards_stale_frames_and_accepts_new_frames() {
        let mut queue = FrameQueue::try_new(2, FRAME_MAXIMUM * 2).unwrap();
        queue.enqueue(frame()).unwrap();
        queue.close();

        queue.reopen();

        assert_eq!(queue.state(), QueueState::Open);
        assert!(queue.is_empty());
        queue.enqueue(frame()).unwrap();
        assert_eq!(queue.len(), 1);
    }
}
