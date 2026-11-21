use thiserror::Error;

use crate::frame::Frame;

/// Bytes of file payload per data frame.
///
/// `RequestController.resolvePackage` allocates an 892-byte chunk buffer when
/// a job id is present; the 4-byte job id is prepended to each chunk, giving
/// an 896-byte body and a 918-byte frame.
pub const DATA_CHUNK_SIZE: usize = 892;

/// Number of data frames that will be used for a file of `len` bytes.
pub fn frame_count(len: usize) -> usize {
    len.div_ceil(DATA_CHUNK_SIZE)
}

/// Split `data` into the framed, job-id-prefixed chunks the printer expects.
///
/// Each frame carries:
/// - body = job id (u32 LE) + up to 892 bytes of file data
/// - `msg_sn` = 1, 2, 3, ...
/// - `pkg_total` = total frame count
/// - `pkg_num` = 1-based index
///
/// The printer starts the job automatically once all `pkg_total` frames have
/// arrived; no `confirm-job` call is used by protocol type 1 devices.
pub fn data_frames(job_id: u32, data: &[u8]) -> Result<Vec<Frame>, JobError> {
    if data.is_empty() {
        return Err(JobError::EmptyFile);
    }

    let total = frame_count(data.len());
    if total > usize::from(u16::MAX) {
        return Err(JobError::TooLarge {
            frames: total,
            max: usize::from(u16::MAX),
        });
    }
    let total_u16 = total as u16;

    let mut frames = Vec::with_capacity(total);
    for (index, chunk) in data.chunks(DATA_CHUNK_SIZE).enumerate() {
        let pkg_num = (index + 1) as u16;
        let mut body = Vec::with_capacity(4 + chunk.len());
        body.extend_from_slice(&job_id.to_le_bytes());
        body.extend_from_slice(chunk);
        frames.push(Frame::data(body, index as u32 + 1, total_u16, pkg_num));
    }
    Ok(frames)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JobError {
    #[error("file is empty")]
    EmptyFile,
    #[error("job needs {frames} frames, maximum is {max}")]
    TooLarge { frames: usize, max: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_892_byte_chunks_with_job_id() {
        let data = vec![0xAB; 2000];
        let frames = data_frames(0x11223344, &data).unwrap();
        assert_eq!(frames.len(), 3);

        assert_eq!(frames[0].body.len(), 4 + 892);
        assert_eq!(frames[1].body.len(), 4 + 892);
        assert_eq!(frames[2].body.len(), 4 + 216);
        assert_eq!(&frames[0].body[..4], &0x11223344u32.to_le_bytes());

        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.pkg_total, 3);
            assert_eq!(frame.pkg_num, (i + 1) as u16);
            assert_eq!(frame.msg_sn, (i + 1) as u32);
            assert_eq!(frame.connect_type, 2);
            assert_eq!(frame.interactive, 6);
            assert_eq!(frame.encoding, 2);
            assert!(!frame.split);
        }

        // Every payload byte appears exactly once.
        let payload: Vec<u8> = frames
            .iter()
            .flat_map(|f| f.body[4..].iter().copied())
            .collect();
        assert_eq!(payload, data);
    }

    #[test]
    fn exact_chunk_boundary_has_no_empty_tail() {
        let frames = data_frames(1, &vec![0; DATA_CHUNK_SIZE * 2]).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[1].body.len(), 4 + DATA_CHUNK_SIZE);
    }

    #[test]
    fn rejects_empty_file() {
        assert_eq!(data_frames(1, &[]), Err(JobError::EmptyFile));
    }

    #[test]
    fn frames_encode_within_918_bytes() {
        for frame in data_frames(1, &vec![0xFF; 5000]).unwrap() {
            let bytes = frame.encode().unwrap();
            assert!(bytes.len() <= 918, "frame was {} bytes", bytes.len());
        }
    }
}
