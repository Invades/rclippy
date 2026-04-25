use std::io::{Error, ErrorKind};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::PROTOCOL_VERSION;

pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Hello {
        protocol_version: u16,
        device_id: String,
    },
    ClipboardText {
        seq: u64,
        sha256: String,
        text: String,
    },
    Ping,
    Pong,
    Error {
        message: String,
    },
}

impl Frame {
    pub fn hello(device_id: impl Into<String>) -> Self {
        Self::Hello {
            protocol_version: PROTOCOL_VERSION,
            device_id: device_id.into(),
        }
    }

    pub fn clipboard_text(seq: u64, text: String) -> Self {
        Self::ClipboardText {
            seq,
            sha256: sha256_text(&text),
            text,
        }
    }
}

pub fn sha256_text(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

pub fn validate_clipboard_frame(frame: &Frame, max_text_bytes: usize) -> Result<(), String> {
    match frame {
        Frame::ClipboardText { sha256, text, .. } => {
            if text.len() > max_text_bytes {
                return Err("clipboard text exceeds configured limit".to_owned());
            }
            let actual = sha256_text(text);
            if &actual != sha256 {
                return Err("clipboard text hash mismatch".to_owned());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub async fn write_frame<W>(writer: &mut W, frame: &Frame) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(frame).map_err(|err| Error::new(ErrorKind::InvalidData, err))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "frame too large"));
    }
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

pub async fn read_frame<R>(reader: &mut R) -> std::io::Result<Frame>
where
    R: AsyncRead + Unpin,
{
    let len = reader.read_u32().await? as usize;
    if len > MAX_FRAME_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "frame too large"));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|err| Error::new(ErrorKind::InvalidData, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frame_round_trip() {
        let frame = Frame::clipboard_text(7, "hello".to_owned());
        let (mut client, mut server) = tokio::io::duplex(4096);

        write_frame(&mut client, &frame).await.unwrap();
        let decoded = read_frame(&mut server).await.unwrap();

        assert_eq!(decoded, frame);
    }

    #[test]
    fn detects_bad_clipboard_hash() {
        let frame = Frame::ClipboardText {
            seq: 1,
            sha256: "bad".to_owned(),
            text: "hello".to_owned(),
        };

        assert!(validate_clipboard_frame(&frame, 1024).is_err());
    }

    #[test]
    fn rejects_oversize_clipboard_text() {
        let frame = Frame::clipboard_text(1, "hello".to_owned());

        assert!(validate_clipboard_frame(&frame, 3).is_err());
    }
}
