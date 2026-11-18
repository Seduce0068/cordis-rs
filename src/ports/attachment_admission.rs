//! Pure Rust port of `@deepseek-ai/dsh-attachment/admission`.
//!
//! Source: `packages/attachment/attachment/src/admission.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Canonical base64 validation before delegating to a storage seam.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// Error codes matching upstream `AttachmentError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentErrorCode {
    InvalidImageBase64,
    InvalidFileBase64,
}

impl AttachmentErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidImageBase64 => "INVALID_IMAGE_BASE64",
            Self::InvalidFileBase64 => "INVALID_FILE_BASE64",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentError {
    pub code: AttachmentErrorCode,
    pub message: String,
}

impl std::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AttachmentError {}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedImage {
    pub data: String,
    pub media_type: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedFile {
    pub data: String,
    pub name: Option<String>,
}

/// Decode canonical standard base64. `empty_reject` rejects an empty payload
/// (images); files accept an empty (zero-byte) payload. Canonicality is checked
/// by re-encoding the decoded bytes and comparing to the original string.
pub fn decode_canonical_base64(
    data: &str,
    empty_reject: bool,
    code: AttachmentErrorCode,
) -> std::result::Result<Vec<u8>, AttachmentError> {
    let invalid = || AttachmentError {
        code,
        message: match code {
            AttachmentErrorCode::InvalidImageBase64 => {
                "Image upload is not canonical base64.".to_string()
            }
            AttachmentErrorCode::InvalidFileBase64 => {
                "File upload is not canonical base64.".to_string()
            }
        },
    };
    if empty_reject && data.is_empty() {
        return Err(invalid());
    }
    let bytes = STANDARD.decode(data).map_err(|_| invalid())?;
    if STANDARD.encode(&bytes) != data {
        return Err(invalid());
    }
    Ok(bytes)
}

/// Decoded image ready for storage.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    pub data: Vec<u8>,
    pub media_type: String,
    pub name: Option<String>,
}

/// Validate an image batch, preserving caller order.
pub fn decode_images(
    images: &[EncodedImage],
) -> std::result::Result<Vec<DecodedImage>, AttachmentError> {
    images
        .iter()
        .map(|image| {
            Ok(DecodedImage {
                data: decode_canonical_base64(
                    &image.data,
                    true,
                    AttachmentErrorCode::InvalidImageBase64,
                )?,
                media_type: image.media_type.clone(),
                name: image.name.clone(),
            })
        })
        .collect()
}

/// Validate one file upload (empty payload is a valid zero-byte file).
pub fn decode_file(file: &EncodedFile) -> std::result::Result<Vec<u8>, AttachmentError> {
    decode_canonical_base64(&file.data, false, AttachmentErrorCode::InvalidFileBase64)
}
