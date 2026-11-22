//! Reading and writing protocol frames on async streams.

use std::time::Duration;

use dexdrop_core::protocol::{Message, ProtocolError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Errors of a session's I/O.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("connection: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("the other device took too long to answer")]
    Timeout,
    #[error("the connection closed")]
    Closed,
}

/// Writes one message (not flushed).
pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<(), WireError> {
    let frame = msg.encode_frame()?;
    w.write_all(&frame).await?;
    Ok(())
}

/// Writes one message and flushes.
pub async fn send<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<(), WireError> {
    write_msg(w, msg).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one message; a clean end of stream is [`WireError::Closed`].
pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message, WireError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(WireError::Closed);
        }
        Err(err) => return Err(err.into()),
    }
    let len = Message::check_frame_len(u32::from_be_bytes(len))?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Message::decode(&body)?)
}

/// Reads one message within `limit`.
pub async fn read_msg_within<R: AsyncRead + Unpin>(
    r: &mut R,
    limit: Duration,
) -> Result<Message, WireError> {
    tokio::time::timeout(limit, read_msg(r))
        .await
        .map_err(|_| WireError::Timeout)?
}
