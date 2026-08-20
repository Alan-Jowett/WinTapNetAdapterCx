#![forbid(unsafe_code)]

use std::collections::HashMap;

pub const FRAME_MINIMUM: usize = 14;
pub const FRAME_MAXIMUM: usize = 1514;
pub const FDB_CAPACITY: usize = 4096;
pub const IO_RING_BASELINE_VERSION: u32 = 300;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EndpointId(u32);

impl EndpointId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FdbKey {
    mac: [u8; 6],
    vlan: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameClass {
    Unicast,
    Broadcast,
    Multicast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParsedFrame {
    pub source: [u8; 6],
    pub destination: [u8; 6],
    pub vlan: Option<u16>,
    pub class: FrameClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    InvalidLength,
    InvalidVlan,
}

pub fn parse_frame(frame: &[u8]) -> Result<ParsedFrame, FrameError> {
    if !(FRAME_MINIMUM..=FRAME_MAXIMUM).contains(&frame.len()) {
        return Err(FrameError::InvalidLength);
    }

    let mut vlan = None;
    let ether_type = u16::from_be_bytes([frame[12], frame[13]]);
    if ether_type == 0x8100 || ether_type == 0x88a8 {
        if frame.len() < 18 {
            return Err(FrameError::InvalidVlan);
        }
        vlan = Some(u16::from_be_bytes([frame[14], frame[15]]) & 0x0fff);
    }

    let destination: [u8; 6] = frame[0..6].try_into().expect("validated Ethernet header");
    let source: [u8; 6] = frame[6..12].try_into().expect("validated Ethernet header");
    let class = if destination == [0xff; 6] {
        FrameClass::Broadcast
    } else if destination[0] & 1 != 0 {
        FrameClass::Multicast
    } else {
        FrameClass::Unicast
    };

    Ok(ParsedFrame {
        source,
        destination,
        vlan,
        class,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardingError {
    InvalidFrame(FrameError),
    UnknownEndpoint,
}

pub struct ForwardingDatabase {
    entries: HashMap<FdbKey, EndpointId>,
    capacity: usize,
}

impl ForwardingDatabase {
    pub fn new() -> Self {
        Self {
            entries: HashMap::with_capacity(FDB_CAPACITY),
            capacity: FDB_CAPACITY,
        }
    }

    pub fn learn(&mut self, parsed: ParsedFrame, endpoint: EndpointId) {
        let key = FdbKey {
            mac: parsed.source,
            vlan: parsed.vlan,
        };
        if let Some(existing) = self.entries.get_mut(&key) {
            *existing = endpoint;
        } else if self.entries.len() < self.capacity {
            self.entries.insert(key, endpoint);
        }
    }

    fn lookup(&self, parsed: ParsedFrame) -> Option<EndpointId> {
        self.entries
            .get(&FdbKey {
                mac: parsed.destination,
                vlan: parsed.vlan,
            })
            .copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Default for ForwardingDatabase {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EndpointSet {
    endpoints: Vec<EndpointId>,
}

impl EndpointSet {
    pub fn static_pair() -> Self {
        Self {
            endpoints: vec![EndpointId::new(1), EndpointId::new(2)],
        }
    }

    pub fn from_ids<I>(ids: I) -> Result<Self, ForwardingError>
    where
        I: IntoIterator<Item = EndpointId>,
    {
        let endpoints: Vec<_> = ids.into_iter().collect();
        if endpoints.is_empty() {
            return Err(ForwardingError::UnknownEndpoint);
        }
        Ok(Self { endpoints })
    }

    fn contains(&self, endpoint: EndpointId) -> bool {
        self.endpoints.contains(&endpoint)
    }

    fn peers(&self, source: EndpointId) -> impl Iterator<Item = EndpointId> + '_ {
        self.endpoints
            .iter()
            .copied()
            .filter(move |endpoint| *endpoint != source)
    }
}

pub struct Switch {
    endpoints: EndpointSet,
    fdb: ForwardingDatabase,
}

impl Switch {
    pub fn static_pair() -> Self {
        Self {
            endpoints: EndpointSet::static_pair(),
            fdb: ForwardingDatabase::new(),
        }
    }

    pub fn forward(
        &mut self,
        source_endpoint: EndpointId,
        frame: &[u8],
    ) -> Result<Vec<EndpointId>, ForwardingError> {
        if !self.endpoints.contains(source_endpoint) {
            return Err(ForwardingError::UnknownEndpoint);
        }
        let parsed = parse_frame(frame).map_err(ForwardingError::InvalidFrame)?;
        self.fdb.learn(parsed, source_endpoint);

        let mut recipients = Vec::new();
        if parsed.class == FrameClass::Unicast {
            if let Some(destination) = self.fdb.lookup(parsed) {
                if destination != source_endpoint && self.endpoints.contains(destination) {
                    recipients.push(destination);
                }
                return Ok(recipients);
            }
        }

        recipients.extend(self.endpoints.peers(source_endpoint));
        Ok(recipients)
    }

    pub fn fdb_len(&self) -> usize {
        self.fdb.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoRingCapabilities {
    pub maximum_version: u32,
    pub supports_read: bool,
    pub supports_write: bool,
    pub supports_read_scatter: bool,
    pub supports_write_gather: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoRingVersion {
    V3,
    V4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoRingSelectionError {
    RequiredOperationsUnavailable,
    UnsupportedVersion,
}

pub fn select_io_ring_version(
    capabilities: IoRingCapabilities,
) -> Result<IoRingVersion, IoRingSelectionError> {
    if !capabilities.supports_read || !capabilities.supports_write {
        return Err(IoRingSelectionError::RequiredOperationsUnavailable);
    }
    if capabilities.maximum_version < IO_RING_BASELINE_VERSION {
        return Err(IoRingSelectionError::UnsupportedVersion);
    }
    if capabilities.maximum_version >= 400
        && capabilities.supports_read_scatter
        && capabilities.supports_write_gather
    {
        Ok(IoRingVersion::V4)
    } else {
        Ok(IoRingVersion::V3)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotState {
    Free,
    ReadPending,
    Dispatching,
    WritePending(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotCompletion {
    pub slot: usize,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotError {
    InvalidSlot,
    NotFree,
    StaleCompletion,
    InvalidTransition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BufferSlot {
    generation: u64,
    state: SlotState,
}

pub struct BufferPool {
    slots: Vec<BufferSlot>,
}

impl BufferPool {
    pub fn new(count: usize) -> Self {
        Self {
            slots: vec![
                BufferSlot {
                    generation: 0,
                    state: SlotState::Free
                };
                count
            ],
        }
    }

    pub fn begin_read(&mut self, slot: usize) -> Result<SlotCompletion, SlotError> {
        let slot_ref = self.slots.get_mut(slot).ok_or(SlotError::InvalidSlot)?;
        if slot_ref.state != SlotState::Free {
            return Err(SlotError::NotFree);
        }
        slot_ref.generation = slot_ref.generation.wrapping_add(1);
        slot_ref.state = SlotState::ReadPending;
        Ok(SlotCompletion {
            slot,
            generation: slot_ref.generation,
        })
    }

    pub fn begin_dispatch(&mut self, completion: SlotCompletion) -> Result<(), SlotError> {
        let slot = self.current(completion)?;
        if slot.state != SlotState::ReadPending {
            return Err(SlotError::InvalidTransition);
        }
        slot.state = SlotState::Dispatching;
        Ok(())
    }

    pub fn begin_writes(
        &mut self,
        completion: SlotCompletion,
        count: u16,
    ) -> Result<(), SlotError> {
        let slot = self.current(completion)?;
        if slot.state != SlotState::Dispatching || count == 0 {
            return Err(SlotError::InvalidTransition);
        }
        slot.state = SlotState::WritePending(count);
        Ok(())
    }

    pub fn complete_dispatch(&mut self, completion: SlotCompletion) -> Result<(), SlotError> {
        let slot = self.current(completion)?;
        if slot.state != SlotState::Dispatching {
            return Err(SlotError::InvalidTransition);
        }
        slot.state = SlotState::Free;
        Ok(())
    }

    pub fn complete_write(&mut self, completion: SlotCompletion) -> Result<(), SlotError> {
        let slot = self.current(completion)?;
        match slot.state {
            SlotState::WritePending(remaining) if remaining > 1 => {
                slot.state = SlotState::WritePending(remaining - 1);
            }
            SlotState::WritePending(1) => slot.state = SlotState::Free,
            _ => return Err(SlotError::InvalidTransition),
        }
        Ok(())
    }

    pub fn cancel(&mut self, completion: SlotCompletion) -> Result<(), SlotError> {
        let slot = self.current(completion)?;
        if slot.state == SlotState::Free {
            return Err(SlotError::InvalidTransition);
        }
        slot.state = SlotState::Free;
        Ok(())
    }

    fn current(&mut self, completion: SlotCompletion) -> Result<&mut BufferSlot, SlotError> {
        let slot = self
            .slots
            .get_mut(completion.slot)
            .ok_or(SlotError::InvalidSlot)?;
        if slot.generation != completion.generation {
            return Err(SlotError::StaleCompletion);
        }
        Ok(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(destination: [u8; 6], source: [u8; 6]) -> Vec<u8> {
        let mut frame = vec![0; FRAME_MINIMUM];
        frame[..6].copy_from_slice(&destination);
        frame[6..12].copy_from_slice(&source);
        frame
    }

    #[test]
    fn static_pair_forwards_known_unicast_and_floods_unknown() {
        let a = EndpointId::new(1);
        let b = EndpointId::new(2);
        let mut switch = Switch::static_pair();
        let source_a = [2, 0, 0, 0, 0, 1];
        let source_b = [2, 0, 0, 0, 0, 2];

        assert_eq!(
            switch.forward(a, &frame([0xff; 6], source_a)).unwrap(),
            vec![b]
        );
        assert_eq!(
            switch.forward(b, &frame(source_a, source_b)).unwrap(),
            vec![a]
        );
    }

    #[test]
    fn source_move_updates_destination_without_reflection() {
        let a = EndpointId::new(1);
        let b = EndpointId::new(2);
        let mut switch = Switch::static_pair();
        let source = [2, 0, 0, 0, 0, 1];
        let other = [2, 0, 0, 0, 0, 2];

        switch.forward(a, &frame(other, source)).unwrap();
        switch.forward(b, &frame(other, source)).unwrap();
        assert!(switch.forward(b, &frame(source, other)).unwrap().is_empty());
    }

    #[test]
    fn full_fdb_preserves_existing_entries() {
        let a = EndpointId::new(1);
        let mut switch = Switch::static_pair();
        for value in 0..FDB_CAPACITY {
            let mac = [
                2,
                0,
                0,
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
            ];
            switch.forward(a, &frame([0xff; 6], mac)).unwrap();
        }
        assert_eq!(switch.fdb_len(), FDB_CAPACITY);
        let existing = [2, 0, 0, 0, 0, 1];
        switch
            .forward(EndpointId::new(2), &frame(existing, [2, 0, 0, 0, 1, 1]))
            .unwrap();
        assert_eq!(switch.fdb_len(), FDB_CAPACITY);
    }

    #[test]
    fn io_ring_requires_v3_read_and_write() {
        assert_eq!(
            select_io_ring_version(IoRingCapabilities {
                maximum_version: 200,
                supports_read: true,
                supports_write: true,
                supports_read_scatter: false,
                supports_write_gather: false,
            }),
            Err(IoRingSelectionError::UnsupportedVersion)
        );
        assert_eq!(
            select_io_ring_version(IoRingCapabilities {
                maximum_version: 300,
                supports_read: true,
                supports_write: true,
                supports_read_scatter: false,
                supports_write_gather: false,
            }),
            Ok(IoRingVersion::V3)
        );
    }

    #[test]
    fn stale_completion_cannot_reclaim_reused_slot() {
        let mut pool = BufferPool::new(1);
        let first = pool.begin_read(0).unwrap();
        pool.begin_dispatch(first).unwrap();
        pool.begin_writes(first, 1).unwrap();
        pool.complete_write(first).unwrap();
        let second = pool.begin_read(0).unwrap();
        assert_eq!(pool.begin_dispatch(first), Err(SlotError::StaleCompletion));
        pool.begin_dispatch(second).unwrap();
    }
}
