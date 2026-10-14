//! Native messaging framing. Incoming frames over our application cap are
//! drained without allocating their body, preserving the following frame.

use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    Payload(Vec<u8>),
    Skipped,
}

pub async fn read_message<R>(reader: &mut R) -> Result<Option<Message>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    if reader.read(&mut len_buf[..1]).await? == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut len_buf[1..])
        .await
        .context("reading message length")?;
    let len = u32::from_ne_bytes(len_buf);
    if len > MAX_MESSAGE_BYTES {
        let copied = tokio::io::copy(&mut reader.take(u64::from(len)), &mut tokio::io::sink())
            .await
            .context("discarding oversized message")?;
        if copied != u64::from(len) {
            return Err(anyhow!("EOF in oversized message body"));
        }
        return Ok(Some(Message::Skipped));
    }
    let mut payload = vec![0u8; len as usize];
    reader
        .read_exact(&mut payload)
        .await
        .context("reading message body")?;
    Ok(Some(Message::Payload(payload)))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = (payload.len() as u32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }
    #[tokio::test]
    async fn oversized_frame_does_not_hide_following_message() {
        let mut bytes = frame(&vec![b'x'; MAX_MESSAGE_BYTES as usize + 1]);
        bytes.extend(frame(b"{\"type\":\"ping\"}"));
        let mut reader = bytes.as_slice();
        assert_eq!(
            read_message(&mut reader).await.unwrap(),
            Some(Message::Skipped)
        );
        assert_eq!(
            read_message(&mut reader).await.unwrap(),
            Some(Message::Payload(b"{\"type\":\"ping\"}".to_vec()))
        );
        assert_eq!(read_message(&mut reader).await.unwrap(), None);
    }
    #[tokio::test]
    async fn truncated_header_and_body_are_errors() {
        assert!(read_message(&mut &[1u8, 0][..]).await.is_err());
        assert!(read_message(&mut &[2u8, 0, 0, 0, b'x'][..]).await.is_err());
        let bytes = (MAX_MESSAGE_BYTES + 1).to_ne_bytes();
        assert!(read_message(&mut bytes.as_slice()).await.is_err());
    }
    #[tokio::test]
    async fn framing_round_trip() {
        let mut writer = Vec::new();
        write_message(&mut writer, b"hello").await.unwrap();
        assert_eq!(
            read_message(&mut writer.as_slice()).await.unwrap(),
            Some(Message::Payload(b"hello".to_vec()))
        );
    }
}
