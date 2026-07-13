//! Length-prefixed channel mux inside a Noise transport session.

use std::io;

use bytes::{Buf, BufMut, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MUX_HEADER_LEN: usize = 8;
pub const CHANNEL_MSG: u32 = 0;
pub const CHANNEL_CALL_AUDIO: u32 = 1;
pub const CHANNEL_CALL_VIDEO: u32 = 2;
pub const CHANNEL_KEEPALIVE: u32 = 0xFFFF_FFFF;
pub const MAX_MUX_PAYLOAD: usize = 1024 * 1024;

pub struct ChannelMuxWriter<W> {
    inner: W,
}

impl<W: AsyncWrite + Unpin> ChannelMuxWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }

    pub async fn write_frame(
        &mut self,
        channel: u32,
        payload: &[u8],
    ) -> io::Result<()> {
        if payload.len() > MAX_MUX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mux payload too large",
            ));
        }
        let mut hdr = BytesMut::with_capacity(MUX_HEADER_LEN);
        hdr.put_u32(channel);
        hdr.put_u32(payload.len() as u32);
        self.inner.write_all(&hdr).await?;
        if !payload.is_empty() {
            self.inner.write_all(payload).await?;
        }
        self.inner.flush().await
    }

    pub async fn write_keepalive_ping(&mut self) -> io::Result<()> {
        self.write_frame(CHANNEL_KEEPALIVE, &[]).await
    }
}

pub struct ChannelMuxReader<R> {
    inner: R,
    buf: BytesMut,
}

impl<R: AsyncRead + Unpin> ChannelMuxReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buf: BytesMut::with_capacity(4096),
        }
    }

    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Returns `(channel, payload)`. Keepalive pong is `channel=KEEPALIVE, payload=[0x01]`.
    pub async fn read_frame(&mut self) -> io::Result<(u32, Vec<u8>)> {
        loop {
            while self.buf.len() < MUX_HEADER_LEN {
                let n = self.inner.read_buf(&mut self.buf).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "mux stream closed",
                    ));
                }
            }
            let channel = u32::from_be_bytes(self.buf[0..4].try_into().unwrap());
            let len = u32::from_be_bytes(self.buf[4..8].try_into().unwrap()) as usize;
            if len > MAX_MUX_PAYLOAD {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mux length exceeds max",
                ));
            }
            if self.buf.len() < MUX_HEADER_LEN + len {
                let n = self.inner.read_buf(&mut self.buf).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "mux stream closed mid-frame",
                    ));
                }
                continue;
            }
            self.buf.advance(MUX_HEADER_LEN);
            let payload = self.buf.split_to(len).to_vec();
            return Ok((channel, payload));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn roundtrip_frame() {
        let (a, b) = duplex(64 * 1024);
        let (mut ar, mut aw) = (ChannelMuxReader::new(a), ChannelMuxWriter::new(b));
        aw.write_frame(CHANNEL_MSG, b"hello").await.unwrap();
        let (ch, payload) = ar.read_frame().await.unwrap();
        assert_eq!(ch, CHANNEL_MSG);
        assert_eq!(payload, b"hello");
    }
}
