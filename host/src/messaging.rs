//! Native messaging stdio framing.
//!
//! Each message is a 4-byte little-endian (native order on the platforms we
//! care about; Firefox uses native order, and Linux is little-endian) length
//! prefix followed by UTF-8 JSON. Max payload 1 MiB per the spec.

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;

/// Read one length-prefixed message from `reader`.
///
/// Returns `Ok(None)` if EOF is encountered cleanly between messages,
/// `Err` if EOF happens mid-message or the length is invalid.
pub async fn read_message<R>(reader: &mut R) -> Result<Option<Vec<u8>>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e).context("reading message length"),
    }

    let len = u32::from_ne_bytes(len_buf);
    if len == 0 {
        return Ok(Some(Vec::new()));
    }
    if len > MAX_MESSAGE_BYTES {
        return Err(anyhow!(
            "message length {len} exceeds maximum {MAX_MESSAGE_BYTES}"
        ));
    }

    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .await
        .context("reading message body")?;
    Ok(Some(payload))
}

/// Write a length-prefixed message to `writer`.
pub async fn write_message<W>(writer: &mut W, payload: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_MESSAGE_BYTES as usize {
        return Err(anyhow!(
            "outbound message {} bytes exceeds {MAX_MESSAGE_BYTES}",
            payload.len()
        ));
    }
    let len = (payload.len() as u32).to_ne_bytes();
    writer.write_all(&len).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}
