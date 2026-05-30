#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixStream;

/// Write a length-prefixed message: 4-byte big-endian length + payload.
#[cfg(unix)]
pub async fn write_message(stream: &mut UnixStream, msg: &[u8]) -> std::io::Result<()> {
    let len = msg.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(msg).await?;
    stream.flush().await?;
    Ok(())
}

/// Read a length-prefixed message: 4-byte big-endian length, then that many bytes.
/// Returns an empty Vec on EOF (peer closed).
#[cfg(unix)]
pub async fn read_message(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(Vec::new()),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;

    // Sanity check: reject absurdly large messages (>16 MB)
    if len > 16 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("message too large: {} bytes", len),
        ));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}
