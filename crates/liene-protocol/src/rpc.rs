use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::frame::{Frame, FrameError};

/// Wire method names for protocol type 1 devices (the N200 Pro's `cumin`
/// entry is `protocolType: 1`). The Android SDK derives these by replacing
/// `_` with `-` in the legacy method constants.
pub mod methods {
    pub const GET_PROP: &str = "get-prop";
    pub const PRINT_JOB: &str = "print-job";
    pub const CANCEL_JOB: &str = "cancel-job";
    pub const GET_JOB_INFO: &str = "get-job-info";
    pub const JOB_ID_LIST: &str = "job-id-list";
    pub const GET_JOB_ID_LIST: &str = "get-job-id-list";
    pub const RESUME_PRINTER: &str = "resume-printer";
    pub const COMBO_JOB: &str = "combo-job";
    pub const DEVICE_STATUS: &str = "device-status";
}

/// Paper size code for 2x3 inch media (the N200 Pro's only size).
pub const MEDIA_SIZE_2X3: u32 = 5011;
/// Media type code for photo paper.
pub const MEDIA_TYPE_PHOTO: u32 = 2010;
/// Logical channel reported by the app for Liene Bluetooth printers.
pub const CHANNEL_BLUETOOTH: u32 = 14864;
/// `document-format` value for JPEG.
pub const DOCUMENT_FORMAT_JPEG: u32 = 9;
/// `document-format` value for PNG.
pub const DOCUMENT_FORMAT_PNG: u32 = 10;
/// `job-type` for a normal photo print.
pub const JOB_TYPE_PRINT: u32 = 0;

/// A JSON-RPC style request as serialized by the app's `PrintJobEvent`.
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest<T> {
    pub id: u32,
    pub method: String,
    pub params: T,
    #[serde(rename = "spec-type")]
    pub spec_type: u8,
}

impl<T: Serialize> RpcRequest<T> {
    pub fn new(id: u32, method: impl Into<String>, params: T) -> Self {
        Self {
            id,
            method: method.into(),
            params,
            // Protocol type 1 devices use the spec-style envelope.
            spec_type: 1,
        }
    }

    pub fn to_json(&self) -> Result<Vec<u8>, BuildError> {
        Ok(serde_json::to_vec(self)?)
    }

    /// Wrap the request in a Hannto JSON frame.
    pub fn to_frame(&self) -> Result<Frame, BuildError> {
        Ok(Frame::json(self.to_json()?))
    }
}

/// Parameters for the print-only `print-job` request.
///
/// Mirrors `DeviceComboJobBean` / `PrintJobBean` serialization used by the
/// app for protocol type 1 devices.
#[derive(Debug, Clone, Serialize)]
pub struct PrintJobParams {
    pub copies: u32,
    #[serde(rename = "document-name")]
    pub document_name: String,
    #[serde(rename = "file-size")]
    pub file_size: u64,
    #[serde(rename = "media-size")]
    pub media_size: u32,
    #[serde(rename = "media-type")]
    pub media_type: u32,
    pub channel: u32,
    #[serde(rename = "document-format")]
    pub document_format: u32,
    #[serde(rename = "job-type")]
    pub job_type: u32,
}

impl PrintJobParams {
    pub fn photo(job: PrintJob, document_name: impl Into<String>, file_size: u64) -> Self {
        Self {
            copies: job.copies,
            document_name: document_name.into(),
            file_size,
            media_size: job.media_size,
            media_type: job.media_type,
            channel: job.channel,
            document_format: job.document_format,
            job_type: job.job_type,
        }
    }
}

/// Print configuration shared by CLI and GUI callers.
#[derive(Debug, Clone)]
pub struct PrintJob {
    pub copies: u32,
    pub media_size: u32,
    pub media_type: u32,
    pub channel: u32,
    pub document_format: u32,
    pub job_type: u32,
}

impl Default for PrintJob {
    fn default() -> Self {
        Self {
            copies: 1,
            media_size: MEDIA_SIZE_2X3,
            media_type: MEDIA_TYPE_PHOTO,
            channel: CHANNEL_BLUETOOTH,
            document_format: DOCUMENT_FORMAT_JPEG,
            job_type: JOB_TYPE_PRINT,
        }
    }
}

/// Parameters for `get-job-info`: the app's `ParamsJobIdBean` serializes a
/// single hyphenated field.
#[derive(Debug, Clone, Serialize)]
pub struct JobInfoParams {
    #[serde(rename = "job-id")]
    pub job_id: u32,
}

/// A decoded JSON response from the printer.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcResponse {
    pub id: u32,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
}

impl RpcResponse {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, BuildError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    /// Extract the printer-assigned job id from `result`.
    ///
    /// Firmware versions have been observed using both `job-id` and
    /// `job_id`; the app accepts either.
    pub fn job_id(&self) -> Option<u32> {
        let result = self.result.as_ref()?;
        job_id_from_value(result)
    }

    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

pub fn job_id_from_value(value: &Value) -> Option<u32> {
    if let Some(n) = value.as_u64() {
        return u32::try_from(n).ok();
    }
    let obj = value.as_object()?;
    for key in ["job-id", "job_id", "jobId"] {
        if let Some(n) = obj.get(key).and_then(Value::as_u64) {
            return u32::try_from(n).ok();
        }
    }
    None
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_matches_app_json_shape() {
        let params = PrintJobParams::photo(PrintJob::default(), "photo.jpg", 123_456);
        let req = RpcRequest::new(42, methods::PRINT_JOB, params);
        let json: Value = serde_json::from_slice(&req.to_json().unwrap()).unwrap();

        assert_eq!(json["id"], 42);
        assert_eq!(json["method"], "print-job");
        assert_eq!(json["spec-type"], 1);
        assert_eq!(json["params"]["copies"], 1);
        assert_eq!(json["params"]["document-name"], "photo.jpg");
        assert_eq!(json["params"]["file-size"], 123_456);
        assert_eq!(json["params"]["media-size"], 5011);
        assert_eq!(json["params"]["media-type"], 2010);
        assert_eq!(json["params"]["channel"], 14864);
        assert_eq!(json["params"]["document-format"], 9);
        assert_eq!(json["params"]["job-type"], 0);
    }

    #[test]
    fn parses_both_job_id_spellings() {
        let a = RpcResponse::from_slice(br#"{"id":1,"result":{"job-id":7}}"#).unwrap();
        let b = RpcResponse::from_slice(br#"{"id":1,"result":{"job_id":8}}"#).unwrap();
        assert_eq!(a.job_id(), Some(7));
        assert_eq!(b.job_id(), Some(8));
    }

    #[test]
    fn parses_error_response() {
        let resp =
            RpcResponse::from_slice(br#"{"id":3,"error":{"code":-5001,"message":"unsupported"}}"#)
                .unwrap();
        assert!(resp.is_error());
        assert_eq!(resp.id, 3);
    }
}
