// @feature runtime
// @spec docs/features/runtime.md
// @boundary bounded-runtime-json-input
use std::io::{Error, ErrorKind, Read};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

pub(crate) const MAX_JSON_FRAME_BYTES: usize = 1024 * 1024;

pub(crate) fn read_json_document(reader: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_JSON_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JSON_FRAME_BYTES {
        return Err(frame_too_large());
    }
    Ok(bytes)
}

/// On overflow, the reader remains within the oversized frame, before its newline.
/// Callers can close the transport or discard the remainder before reading another frame.
pub(crate) async fn read_json_line<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!bytes.is_empty()).then_some(bytes));
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if bytes.len().saturating_add(newline) > MAX_JSON_FRAME_BYTES {
                return Err(frame_too_large());
            }
            bytes.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Ok(Some(bytes));
        }

        let remaining_with_sentinel = (MAX_JSON_FRAME_BYTES + 1).saturating_sub(bytes.len());
        let consumed = available.len().min(remaining_with_sentinel);
        bytes.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if bytes.len() > MAX_JSON_FRAME_BYTES {
            return Err(frame_too_large());
        }
    }
}

pub(crate) async fn discard_json_line<R>(reader: &mut R) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(newline + 1);
            return Ok(());
        }
        let consumed = available.len();
        reader.consume(consumed);
    }
}

fn frame_too_large() -> Error {
    Error::new(
        ErrorKind::InvalidData,
        format!("JSON frame exceeds {MAX_JSON_FRAME_BYTES} bytes"),
    )
}
