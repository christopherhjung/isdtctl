//! Link layer of the ISDT serial-over-BLE protocol.
//!
//! Reconstructed from `IsdtPackBase` in the ISDT Android application
//! (`com.isdt.hubin.isdtapp`, version 1.3.8).
//!
//! # Frame layout
//!
//! ```text
//! AA  ADDR  LEN  DATA[LEN]  CHK
//! ```
//!
//! * `AA` is a single unescaped sync byte that opens the frame.
//! * `ADDR` is [`ADDR_TO_DEVICE`] (0x12) for host to device and
//!   [`ADDR_TO_HOST`] (0x21) for device to host.
//! * `LEN` counts `DATA` only. `DATA[0]` is the command word.
//! * `CHK` is `(ADDR + LEN + sum(DATA)) & 0xFF`.
//! * Every byte after the opening sync byte (`ADDR`, `LEN`, `DATA`, `CHK`)
//!   is byte stuffed: a literal `0xAA` is transmitted as `AA AA`.
//!
//! # GATT transport
//!
//! Frames are not written to the characteristic directly. Each GATT write and
//! each notification is `[n][payload; n]`, where `payload` is a slice of the
//! stuffed frame. With a 20 byte MTU a frame longer than 16 bytes is split
//! across several writes of up to 19 payload bytes each.

/// Sync byte that opens a frame, and the byte that gets stuffed inside one.
pub const SYNC: u8 = 0xAA;

/// Address byte the host puts in frames it sends to the charger.
pub const ADDR_TO_DEVICE: u8 = 0x12;

/// Address byte the charger puts in frames it sends to the host.
pub const ADDR_TO_HOST: u8 = 0x21;

/// Largest `LEN` the one-byte length field can express.
pub const MAX_DATA_LEN: usize = u8::MAX as usize;

/// Builds the stuffed on-air frame for `data`, addressed to the charger.
///
/// `data` starts with the command word and is at most [`MAX_DATA_LEN`] bytes.
pub fn encode(data: &[u8]) -> Result<Vec<u8>, FrameError> {
    encode_to(ADDR_TO_DEVICE, data)
}

/// Builds the stuffed on-air frame for `data` with an explicit address byte.
pub fn encode_to(addr: u8, data: &[u8]) -> Result<Vec<u8>, FrameError> {
    if data.len() > MAX_DATA_LEN {
        return Err(FrameError::TooLong(data.len()));
    }
    let len = data.len() as u8;
    let checksum = data
        .iter()
        .fold(addr.wrapping_add(len), |acc, b| acc.wrapping_add(*b));

    let mut out = Vec::with_capacity(data.len() + 8);
    out.push(SYNC);
    push_stuffed(&mut out, addr);
    push_stuffed(&mut out, len);
    for b in data {
        push_stuffed(&mut out, *b);
    }
    push_stuffed(&mut out, checksum);
    Ok(out)
}

fn push_stuffed(out: &mut Vec<u8>, b: u8) {
    if b == SYNC {
        out.push(SYNC);
    }
    out.push(b);
}

/// Splits a stuffed frame into GATT write payloads, each prefixed with its
/// own length byte.
///
/// `mtu` is the negotiated ATT MTU as the app tracks it: 20 for the legacy
/// FFF6 channel, 140 or 160 for the FFF7 and FFF8 channels. At most
/// `mtu - 1` frame bytes travel in one write.
pub fn chunk(frame: &[u8], mtu: usize) -> Vec<Vec<u8>> {
    let budget = mtu.saturating_sub(1).max(1);

    // The app keeps short frames in a single write even on the 20 byte channel.
    if mtu >= 21 || frame.len() <= 16 {
        let mut packet = Vec::with_capacity(frame.len() + 1);
        packet.push(frame.len() as u8);
        packet.extend_from_slice(frame);
        return vec![packet];
    }

    frame
        .chunks(budget)
        .map(|c| {
            let mut packet = Vec::with_capacity(c.len() + 1);
            packet.push(c.len() as u8);
            packet.extend_from_slice(c);
            packet
        })
        .collect()
}

/// Errors raised while building a frame.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The payload exceeds the one-byte length field.
    #[error("frame data is {0} bytes, the length field holds at most 255")]
    TooLong(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum State {
    WaitSync,
    WaitAddress,
    WaitLength,
    WaitData,
    WaitChecksum,
}

/// Incremental decoder for the byte stream arriving on the notify
/// characteristic.
///
/// Feed it the payload of each notification (the notification minus its
/// leading length byte) and it yields the `DATA` field of every frame whose
/// address and checksum are valid. Frames split across notifications are
/// reassembled.
#[derive(Debug)]
pub struct Decoder {
    address: u8,
    state: State,
    sync_run: usize,
    remaining: usize,
    checksum: u8,
    buffer: Vec<u8>,
    max_data_len: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Creates a decoder for frames the charger addresses to the host.
    pub fn new() -> Self {
        Self::with_address(ADDR_TO_HOST)
    }

    /// Creates a decoder that accepts frames carrying `address`.
    pub fn with_address(address: u8) -> Self {
        Self {
            address,
            state: State::WaitSync,
            sync_run: 0,
            remaining: 0,
            checksum: 0,
            buffer: Vec::new(),
            max_data_len: MAX_DATA_LEN,
        }
    }

    /// Drops any partially decoded frame and returns to hunting for a sync byte.
    pub fn reset(&mut self) {
        self.state = State::WaitSync;
        self.sync_run = 0;
        self.remaining = 0;
        self.checksum = 0;
        self.buffer.clear();
    }

    /// Strips the leading length byte from a raw notification and decodes the
    /// payload it announces.
    ///
    /// A notification whose length byte overruns the buffer is ignored, which
    /// is what the app does.
    pub fn push_notification(&mut self, raw: &[u8]) -> Vec<Vec<u8>> {
        let Some((&len, rest)) = raw.split_first() else {
            return Vec::new();
        };
        let len = len as usize;
        if len > rest.len() {
            return Vec::new();
        }
        self.push(&rest[..len])
    }

    /// Decodes a slice of the stuffed byte stream, returning the `DATA` field
    /// of each frame that completed.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut done = Vec::new();
        for &b in bytes {
            if let Some(frame) = self.push_byte(b) {
                done.push(frame);
            }
        }
        done
    }

    fn push_byte(&mut self, b: u8) -> Option<Vec<u8>> {
        if b == SYNC {
            self.sync_run += 1;
            // The first byte of a stuffed pair, or the frame's opening sync
            // byte. Either way it carries no data of its own.
            if self.sync_run % 2 == 1 {
                return None;
            }
        } else {
            // A lone sync byte just ended: the frame starts here.
            if self.sync_run % 2 == 1 {
                self.state = State::WaitAddress;
            }
            self.sync_run = 0;
        }

        match self.state {
            State::WaitAddress => {
                if b == self.address {
                    self.checksum = b;
                    self.state = State::WaitLength;
                } else {
                    self.state = State::WaitSync;
                }
                None
            }
            State::WaitLength => {
                self.remaining = b as usize;
                self.checksum = self.checksum.wrapping_add(b);
                self.buffer.clear();
                if self.remaining == 0 || self.remaining > self.max_data_len {
                    self.state = State::WaitSync;
                } else {
                    self.state = State::WaitData;
                }
                None
            }
            State::WaitData => {
                self.buffer.push(b);
                self.checksum = self.checksum.wrapping_add(b);
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.state = State::WaitChecksum;
                }
                None
            }
            State::WaitChecksum => {
                self.state = State::WaitSync;
                if b == self.checksum {
                    Some(std::mem::take(&mut self.buffer))
                } else {
                    self.buffer.clear();
                    None
                }
            }
            State::WaitSync => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_electrical_query_the_app_sends() {
        // IsdtPackBleElecQueryReq for channel 0.
        assert_eq!(
            encode(&[0xE4, 0x00]).unwrap(),
            vec![0xAA, 0x12, 0x02, 0xE4, 0x00, 0xF8]
        );
    }

    #[test]
    fn encodes_every_fixed_request_with_the_apps_checksum() {
        // Each expected checksum is the literal the decompiled app carries.
        let cases: [(&[u8], u8); 6] = [
            (&[0xE6, 0x00], 0xFA), // work state
            (&[0xE8, 0x00], 0xFC), // temperature
            (&[0xFA, 0x00], 0x0E), // inner resistance
            (&[0xE0], 0xF3),       // hardware info
            (&[0xE2], 0xF5),       // limit parameters
            (&[0xFC, 0xCA], 0xDA), // reboot
        ];
        for (data, checksum) in cases {
            let frame = encode(data).unwrap();
            assert_eq!(*frame.last().unwrap(), checksum, "data {data:02X?}");
        }
    }

    #[test]
    fn stuffs_a_literal_sync_byte() {
        // 0xAA anywhere after the opening sync byte is doubled.
        let frame = encode(&[0xAA, 0x01]).unwrap();
        assert_eq!(frame[..4], [0xAA, 0x12, 0x02, 0xAA]);
        assert_eq!(frame[4], 0xAA);
        assert_eq!(frame[5], 0x01);
    }

    #[test]
    fn round_trips_through_the_decoder() {
        let payload = [0xE7u8, 0x00, 0xAA, 0x12, 0xAA, 0xAA];
        let frame = encode_to(ADDR_TO_HOST, &payload).unwrap();
        let mut decoder = Decoder::new();
        assert_eq!(decoder.push(&frame), vec![payload.to_vec()]);
    }

    #[test]
    fn reassembles_a_frame_split_across_notifications() {
        let payload: Vec<u8> = (0u8..40).collect();
        let frame = encode_to(ADDR_TO_HOST, &payload).unwrap();
        let mut decoder = Decoder::new();
        let mut out = Vec::new();
        for packet in chunk(&frame, 20) {
            out.extend(decoder.push_notification(&packet));
        }
        assert_eq!(out, vec![payload]);
    }

    #[test]
    fn rejects_a_frame_with_a_bad_checksum() {
        let mut frame = encode_to(ADDR_TO_HOST, &[0xE7, 0x00]).unwrap();
        *frame.last_mut().unwrap() ^= 0xFF;
        assert!(Decoder::new().push(&frame).is_empty());
    }

    #[test]
    fn resynchronises_after_garbage() {
        let frame = encode_to(ADDR_TO_HOST, &[0xE9, 0x00, 0x1A, 0x1B, 0x40]).unwrap();
        let mut stream = vec![0x00, 0x01, 0x02, 0xAA, 0x99];
        stream.extend_from_slice(&frame);
        let mut decoder = Decoder::new();
        assert_eq!(
            decoder.push(&stream),
            vec![vec![0xE9, 0x00, 0x1A, 0x1B, 0x40]]
        );
    }

    #[test]
    fn short_frames_stay_in_one_write() {
        let frame = encode(&[0xE4, 0x00]).unwrap();
        assert_eq!(chunk(&frame, 20).len(), 1);
    }

    #[test]
    fn long_frames_split_on_the_small_mtu_only() {
        let frame = encode(&[0x11; 60]).unwrap();
        assert!(chunk(&frame, 20).len() > 1);
        assert_eq!(chunk(&frame, 160).len(), 1);
    }
}
