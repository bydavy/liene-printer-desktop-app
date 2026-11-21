//! Wire protocol for Hannto-based Liene photo printers.
//!
//! The Liene Pearl N200 Pro speaks a framed binary protocol over Bluetooth
//! Classic SPP. Frames contain either JSON control messages or chunks of a
//! print job's image data.
//!
//! Frame layout (all integers little-endian):
//!
//! ```text
//! 0x7E | version | reserve | connect_type | interactive | encoding
//!      | terminal_id u32 | msg_sn u32 | pkg_total u16 | pkg_num u16
//!      | attributes u16 | body ... | checksum u8 | 0x7E
//! ```
//!
//! `attributes` packs the body length in bits 0..=9, the encryption mode in
//! bits 10..=12, and a "split" flag in bit 13. The checksum is the wrapping
//! sum of every byte after the leading `0x7E` up to (excluding) the checksum.

pub mod frame;
pub mod job;
pub mod rpc;

pub use frame::{Frame, FrameError, FrameReader, FRAME_END, FRAME_START, VERSION};
pub use job::{data_frames, frame_count, DATA_CHUNK_SIZE};
pub use rpc::{
    methods, BuildError, JobInfoParams, PrintJob, PrintJobParams, RpcRequest, RpcResponse,
    CHANNEL_BLUETOOTH, DOCUMENT_FORMAT_JPEG, DOCUMENT_FORMAT_PNG, JOB_TYPE_PRINT, MEDIA_SIZE_2X3,
    MEDIA_TYPE_PHOTO,
};
