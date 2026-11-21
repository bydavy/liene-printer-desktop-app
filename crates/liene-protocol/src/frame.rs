use thiserror::Error;

/// Frame delimiter / start-of-frame byte.
pub const FRAME_START: u8 = 0x7E;
/// Frame delimiter / end-of-frame byte.
pub const FRAME_END: u8 = 0x7E;
/// Protocol version used by the current Liene app (`PackageBuilder.version`).
pub const VERSION: u8 = 100;

/// Number of header bytes before the body.
pub const HEADER_LEN: usize = 20;
/// Header + checksum + trailing delimiter.
pub const OVERHEAD: usize = HEADER_LEN + 2;

const ATTR_LEN_MASK: u16 = 0x03FF;
const ATTR_ENCRYPT_SHIFT: u16 = 10;
const ATTR_ENCRYPT_MASK: u16 = 0x1C00;
const ATTR_SPLIT: u16 = 0x2000;

/// Largest body that fits in the 10-bit length field.
pub const MAX_BODY_LEN: usize = ATTR_LEN_MASK as usize;

/// A single Hannto protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: u8,
    pub reserve: u8,
    pub connect_type: u8,
    pub interactive: u8,
    pub encoding: u8,
    pub terminal_id: u32,
    pub msg_sn: u32,
    pub pkg_total: u16,
    pub pkg_num: u16,
    /// 0 = none, 1 = PSK, 2 = RC4, 3 = RSA, 4 = MiJia, 5 = AES-128-ECB,
    /// 6 = AES-128-CBC.
    pub encrypt_type: u8,
    pub split: bool,
    pub body: Vec<u8>,
}

impl Frame {
    /// A JSON control frame, as sent by `PackageBuilder` defaults:
    /// connect type 1 (message), interactive 6 (request), encoding 3 (JSON).
    pub fn json(body: impl Into<Vec<u8>>) -> Self {
        Self {
            version: VERSION,
            reserve: 0,
            connect_type: 1,
            interactive: 6,
            encoding: 3,
            terminal_id: 0,
            msg_sn: 0,
            pkg_total: 0,
            pkg_num: 0,
            encrypt_type: 0,
            split: false,
            body: body.into(),
        }
    }

    /// A job-data frame, as built by `RequestController.sendData`:
    /// connect type 2 (data), interactive 6, encoding 2 (binary).
    pub fn data(body: impl Into<Vec<u8>>, msg_sn: u32, pkg_total: u16, pkg_num: u16) -> Self {
        Self {
            version: VERSION,
            reserve: 0,
            connect_type: 2,
            interactive: 6,
            encoding: 2,
            terminal_id: 0,
            msg_sn,
            pkg_total,
            pkg_num,
            encrypt_type: 0,
            split: false,
            body: body.into(),
        }
    }

    pub fn attribute(&self) -> u16 {
        let len = (self.body.len() as u16) & ATTR_LEN_MASK;
        let enc = (u16::from(self.encrypt_type) << ATTR_ENCRYPT_SHIFT) & ATTR_ENCRYPT_MASK;
        let split = if self.split { ATTR_SPLIT } else { 0 };
        len | enc | split
    }

    /// Encode the frame, including delimiters and checksum.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        if self.body.len() > MAX_BODY_LEN {
            return Err(FrameError::BodyTooLong(self.body.len()));
        }
        if self.encrypt_type > 6 {
            return Err(FrameError::InvalidEncryptType(self.encrypt_type));
        }

        let mut out = Vec::with_capacity(self.body.len() + OVERHEAD);
        out.push(FRAME_START);
        out.push(self.version);
        out.push(self.reserve);
        out.push(self.connect_type);
        out.push(self.interactive);
        out.push(self.encoding);
        out.extend_from_slice(&self.terminal_id.to_le_bytes());
        out.extend_from_slice(&self.msg_sn.to_le_bytes());
        out.extend_from_slice(&self.pkg_total.to_le_bytes());
        out.extend_from_slice(&self.pkg_num.to_le_bytes());
        out.extend_from_slice(&self.attribute().to_le_bytes());
        out.extend_from_slice(&self.body);

        let checksum = checksum(&out[1..]);
        out.push(checksum);
        out.push(FRAME_END);
        Ok(out)
    }

    /// Decode exactly one frame (no leading/trailing garbage).
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < OVERHEAD {
            return Err(FrameError::TooShort);
        }
        if bytes[0] != FRAME_START || bytes[bytes.len() - 1] != FRAME_END {
            return Err(FrameError::BadDelimiter);
        }

        let attr = u16::from_le_bytes([bytes[18], bytes[19]]);
        let body_len = usize::from(attr & ATTR_LEN_MASK);
        if bytes.len() != body_len + OVERHEAD {
            return Err(FrameError::LengthMismatch {
                declared: body_len,
                actual: bytes.len().saturating_sub(OVERHEAD),
            });
        }

        let expected = checksum(&bytes[1..HEADER_LEN + body_len]);
        let found = bytes[HEADER_LEN + body_len];
        if expected != found {
            return Err(FrameError::BadChecksum { expected, found });
        }

        Ok(Self {
            version: bytes[1],
            reserve: bytes[2],
            connect_type: bytes[3],
            interactive: bytes[4],
            encoding: bytes[5],
            terminal_id: u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            msg_sn: u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
            pkg_total: u16::from_le_bytes([bytes[14], bytes[15]]),
            pkg_num: u16::from_le_bytes([bytes[16], bytes[17]]),
            encrypt_type: ((attr & ATTR_ENCRYPT_MASK) >> ATTR_ENCRYPT_SHIFT) as u8,
            split: attr & ATTR_SPLIT != 0,
            body: bytes[HEADER_LEN..HEADER_LEN + body_len].to_vec(),
        })
    }
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, byte| acc.wrapping_add(*byte))
}

/// Incremental frame reader for a byte stream.
#[derive(Debug, Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn buffered(&self) -> &[u8] {
        &self.buffer
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Return the next complete frame, or `None` if more bytes are needed.
    pub fn next_frame(&mut self) -> Option<Result<Frame, FrameError>> {
        let start = self.find_start()?;
        if start > 0 {
            self.buffer.drain(..start);
        }
        if self.buffer.len() < OVERHEAD {
            return None;
        }

        let attr = u16::from_le_bytes([self.buffer[18], self.buffer[19]]);
        let body_len = usize::from(attr & ATTR_LEN_MASK);
        let total = body_len + OVERHEAD;
        if self.buffer.len() < total {
            return None;
        }

        if self.buffer[total - 1] != FRAME_END {
            // Not actually a frame (or a truncated one); skip one byte and resync.
            self.buffer.drain(..1);
            return Some(Err(FrameError::BadDelimiter));
        }

        let frame = Frame::decode(&self.buffer[..total]);
        self.buffer.drain(..total);
        Some(frame)
    }

    /// Find a plausible frame start: `0x7E`, a known version byte, and a zero
    /// reserve byte, matching the app's `FrameUtils.getFrameHeadIndex`.
    fn find_start(&self) -> Option<usize> {
        self.buffer
            .iter()
            .enumerate()
            .find(|(i, byte)| {
                **byte == FRAME_START
                    && self.buffer.len() > i + 2
                    && matches!(self.buffer[i + 1], VERSION | 1)
                    && self.buffer[i + 2] == 0
            })
            .map(|(i, _)| i)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short")]
    TooShort,
    #[error("frame delimiter missing")]
    BadDelimiter,
    #[error("body length {0} exceeds 10-bit length field")]
    BodyTooLong(usize),
    #[error("unsupported encryption type {0}")]
    InvalidEncryptType(u8),
    #[error("length mismatch: header says {declared}, frame has {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("bad checksum: expected {expected:#04x}, found {found:#04x}")]
    BadChecksum { expected: u8, found: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_frame_matches_known_layout() {
        let frame = Frame::json(br#"{"id":1}"#.to_vec());
        let bytes = frame.encode().unwrap();

        assert_eq!(bytes[0], 0x7E);
        assert_eq!(bytes[1], 100);
        assert_eq!(bytes[2], 0);
        assert_eq!(bytes[3], 1);
        assert_eq!(bytes[4], 6);
        assert_eq!(bytes[5], 3);
        assert_eq!(&bytes[6..10], &[0, 0, 0, 0]);
        assert_eq!(&bytes[10..14], &[0, 0, 0, 0]);
        assert_eq!(&bytes[14..16], &[0, 0]);
        assert_eq!(&bytes[16..18], &[0, 0]);
        // Body length in the low 10 bits, encryption 0, no split.
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 8);
        assert_eq!(&bytes[20..28], br#"{"id":1}"#);
        assert_eq!(*bytes.last().unwrap(), 0x7E);
        assert_eq!(bytes.len(), 8 + OVERHEAD);

        let decoded = Frame::decode(&bytes).unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn data_frame_has_job_id_and_ordering_fields() {
        let frame = Frame::data(vec![0xAA; 896], 7, 12, 3);
        let bytes = frame.encode().unwrap();
        assert_eq!(bytes[3], 2);
        assert_eq!(bytes[5], 2);
        assert_eq!(
            u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
            7
        );
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 12);
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 3);
        assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 896);
        assert_eq!(bytes.len(), 896 + OVERHEAD);
    }

    #[test]
    fn rejects_oversized_body() {
        let frame = Frame::json(vec![0; MAX_BODY_LEN + 1]);
        assert!(matches!(frame.encode(), Err(FrameError::BodyTooLong(_))));
    }

    #[test]
    fn reader_handles_fragmentation_and_garbage() {
        let a = Frame::json(br#"{"id":1,"result":{}}"#.to_vec())
            .encode()
            .unwrap();
        let b = Frame::json(br#"{"id":2}"#.to_vec()).encode().unwrap();

        let mut reader = FrameReader::new();
        reader.push(&[0x00, 0x11, 0x22]);
        reader.push(&a[..5]);
        assert!(reader.next_frame().is_none());
        reader.push(&a[5..]);
        let first = reader.next_frame().unwrap().unwrap();
        assert_eq!(first.body, br#"{"id":1,"result":{}}"#);

        // Both frames in one read.
        reader.push(&b);
        let second = reader.next_frame().unwrap().unwrap();
        assert_eq!(second.body, br#"{"id":2}"#);
        assert!(reader.next_frame().is_none());
    }

    #[test]
    fn reader_resyncs_after_corruption() {
        let good = Frame::json(br#"{"id":9}"#.to_vec()).encode().unwrap();
        let mut corrupted = good.clone();
        corrupted[20] = b'X'; // breaks the JSON but not the checksum
        let last = corrupted.len() - 3;
        corrupted[last] ^= 0xFF; // break checksum

        let mut reader = FrameReader::new();
        reader.push(&corrupted);
        let err = reader.next_frame().unwrap().unwrap_err();
        assert!(matches!(err, FrameError::BadChecksum { .. }));

        reader.push(&good);
        assert_eq!(reader.next_frame().unwrap().unwrap().body, br#"{"id":9}"#);
    }
}
