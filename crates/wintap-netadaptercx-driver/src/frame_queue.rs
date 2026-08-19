extern crate alloc;

use alloc::{collections::VecDeque, vec::Vec};

pub const FRAME_MINIMUM: usize = 14;
pub const FRAME_MAXIMUM: usize = 1514;

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

pub struct Frame {
    data: Vec<u8>,
}

impl Frame {
    pub fn from_bytes(data: &[u8]) -> Result<Self, QueueError> {
        if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&data.len()) {
            return Err(QueueError::InvalidFrameLength);
        }

        let mut copied = Vec::new();
        copied
            .try_reserve_exact(data.len())
            .map_err(|_| QueueError::InsufficientResources)?;
        copied.extend_from_slice(data);
        Self::from_vec(copied)
    }

    pub fn from_vec(data: Vec<u8>) -> Result<Self, QueueError> {
        if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&data.len()) {
            return Err(QueueError::InvalidFrameLength);
        }

        Ok(Self { data })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

pub struct FrameQueue {
    frames: VecDeque<Frame>,
    limit: usize,
    state: QueueState,
}

impl FrameQueue {
    pub fn try_new(limit: usize) -> Result<Self, QueueError> {
        let mut frames = VecDeque::new();
        frames
            .try_reserve_exact(limit)
            .map_err(|_| QueueError::InsufficientResources)?;

        Ok(Self {
            frames,
            limit,
            state: QueueState::Open,
        })
    }

    pub fn enqueue(&mut self, frame: Frame) -> Result<(), QueueError> {
        if self.state != QueueState::Open {
            return Err(QueueError::Closed);
        }
        if self.frames.len() >= self.limit {
            return Err(QueueError::Full);
        }

        self.frames.push_back(frame);
        Ok(())
    }

    pub fn dequeue(&mut self) -> Option<Frame> {
        self.frames.pop_front()
    }

    pub fn begin_close(&mut self) {
        if self.state == QueueState::Open {
            self.state = QueueState::Closing;
        }
    }

    pub fn close(&mut self) {
        self.frames.clear();
        self.state = QueueState::Closed;
    }

    pub fn reopen(&mut self) {
        self.frames.clear();
        self.state = QueueState::Open;
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn state(&self) -> QueueState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::from_bytes(&[0; FRAME_MINIMUM]).unwrap()
    }

    #[test]
    fn validates_ethernet_frame_bounds() {
        assert!(matches!(
            Frame::from_bytes(&[0; FRAME_MINIMUM - 1]),
            Err(QueueError::InvalidFrameLength)
        ));
        assert!(Frame::from_bytes(&[0; FRAME_MINIMUM]).is_ok());
        assert!(Frame::from_bytes(&[0; FRAME_MAXIMUM]).is_ok());
        assert!(matches!(
            Frame::from_bytes(&[0; FRAME_MAXIMUM + 1]),
            Err(QueueError::InvalidFrameLength)
        ));
    }

    #[test]
    fn enforces_limit_and_preserves_fifo_ownership() {
        let mut queue = FrameQueue::try_new(1).unwrap();
        queue.enqueue(frame()).unwrap();
        assert_eq!(queue.enqueue(frame()), Err(QueueError::Full));
        assert_eq!(queue.len(), 1);
        assert!(queue.dequeue().is_some());
        assert!(queue.is_empty());
    }

    #[test]
    fn closing_rejects_new_frames_and_releases_queued_frames() {
        let mut queue = FrameQueue::try_new(2).unwrap();
        queue.enqueue(frame()).unwrap();
        queue.begin_close();
        assert_eq!(queue.state(), QueueState::Closing);
        assert_eq!(queue.enqueue(frame()), Err(QueueError::Closed));
        queue.close();
        assert_eq!(queue.state(), QueueState::Closed);
        assert!(queue.is_empty());
    }

    #[test]
    fn reopen_discards_stale_frames_and_accepts_new_frames() {
        let mut queue = FrameQueue::try_new(2).unwrap();
        queue.enqueue(frame()).unwrap();
        queue.close();

        queue.reopen();

        assert_eq!(queue.state(), QueueState::Open);
        assert!(queue.is_empty());
        queue.enqueue(frame()).unwrap();
        assert_eq!(queue.len(), 1);
    }
}
