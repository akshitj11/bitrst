//! Async framed P2P reader and bounded writer.

use bytes::{Buf, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::codec::{decode_payload, encode_payload};
use crate::constants::{Network, MAX_PAYLOAD_SIZE, MESSAGE_HEADER_SIZE};
use crate::envelope::MessageHeader;
use crate::error::NetError;
use crate::message::Message;

/// Reads one complete P2P message from `reader`.
///
/// # Errors
///
/// Returns [`NetError`] on I/O failure, malformed headers, or checksum mismatch.
pub async fn read_message<R>(reader: &mut R, network: Network) -> Result<Message, NetError>
where
    R: AsyncRead + Unpin,
{
    let mut header_bytes = [0u8; MESSAGE_HEADER_SIZE];
    read_exact_or_closed(reader, &mut header_bytes).await?;
    let header = MessageHeader::decode(&header_bytes, network.magic(), MAX_PAYLOAD_SIZE)?;

    let mut payload = vec![0u8; header.payload_len as usize];
    if !payload.is_empty() {
        read_exact_or_closed(reader, &mut payload).await?;
    }
    header.verify_checksum(&payload)?;

    decode_payload(header.command.as_str(), &payload)
}

/// Writes one complete P2P message to `writer`.
///
/// # Errors
///
/// Returns [`NetError`] on encoding or I/O failure.
pub async fn write_message<W>(
    writer: &mut W,
    network: Network,
    message: &Message,
) -> Result<(), NetError>
where
    W: AsyncWrite + Unpin,
{
    let (command, payload) = encode_payload(message)?;
    let header = MessageHeader::encode(&command, &payload, network.magic(), MAX_PAYLOAD_SIZE)?;
    writer
        .write_all(&header)
        .await
        .map_err(|_| NetError::Io("write header"))?;
    if !payload.is_empty() {
        writer
            .write_all(&payload)
            .await
            .map_err(|_| NetError::Io("write payload"))?;
    }
    writer.flush().await.map_err(|_| NetError::Io("flush"))?;
    Ok(())
}

/// Bounded outbound queue feeding a background writer task.
pub struct MessageWriter {
    sender: tokio::sync::mpsc::Sender<Message>,
}

impl MessageWriter {
    /// Spawns a writer task that drains `receiver` to `writer`.
    pub fn spawn<W>(
        mut writer: W,
        network: Network,
        capacity: usize,
    ) -> (Self, tokio::task::JoinHandle<()>)
    where
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
        let handle = tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                if write_message(&mut writer, network, &message).await.is_err() {
                    break;
                }
            }
        });
        (Self { sender }, handle)
    }

    /// Enqueues a message for sending, awaiting capacity when the queue is full.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::OutboundQueueFull`] when the writer task has stopped.
    pub async fn send(&self, message: Message) -> Result<(), NetError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| NetError::OutboundQueueFull)
    }
}

/// Buffered reader helper for partial header accumulation.
pub struct FramedReader {
    buffer: BytesMut,
}

impl FramedReader {
    /// Creates an empty read buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(MESSAGE_HEADER_SIZE + 256),
        }
    }

    /// Reads the next message using an internal grow buffer.
    ///
    /// # Errors
    ///
    /// Returns [`NetError`] on I/O failure or malformed messages.
    pub async fn read_message<R>(
        &mut self,
        reader: &mut R,
        network: Network,
    ) -> Result<Message, NetError>
    where
        R: AsyncRead + Unpin,
    {
        while self.buffer.len() < MESSAGE_HEADER_SIZE {
            let read = reader
                .read_buf(&mut self.buffer)
                .await
                .map_err(|_| NetError::Io("read"))?;
            if read == 0 {
                return Err(NetError::ConnectionClosed);
            }
        }

        let mut header_bytes = [0u8; MESSAGE_HEADER_SIZE];
        header_bytes.copy_from_slice(&self.buffer[..MESSAGE_HEADER_SIZE]);
        let header = MessageHeader::decode(&header_bytes, network.magic(), MAX_PAYLOAD_SIZE)?;
        let total = MESSAGE_HEADER_SIZE + header.payload_len as usize;
        while self.buffer.len() < total {
            let read = reader
                .read_buf(&mut self.buffer)
                .await
                .map_err(|_| NetError::Io("read"))?;
            if read == 0 {
                return Err(NetError::ConnectionClosed);
            }
        }

        let payload = self.buffer[MESSAGE_HEADER_SIZE..total].to_vec();
        self.buffer.advance(total);
        header.verify_checksum(&payload)?;
        decode_payload(header.command.as_str(), &payload)
    }
}

impl Default for FramedReader {
    fn default() -> Self {
        Self::new()
    }
}

async fn read_exact_or_closed<R>(reader: &mut R, buf: &mut [u8]) -> Result<(), NetError>
where
    R: AsyncRead + Unpin,
{
    let mut offset = 0usize;
    while offset < buf.len() {
        let read = reader
            .read(&mut buf[offset..])
            .await
            .map_err(|_| NetError::Io("read exact"))?;
        if read == 0 {
            return Err(NetError::ConnectionClosed);
        }
        offset += read;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read_message, write_message, FramedReader, MessageWriter};
    use crate::constants::Network;
    use crate::message::Message;
    #[tokio::test]
    async fn duplex_roundtrip_verack() {
        let (mut left, mut right) = tokio::io::duplex(1024);
        let message = Message::verack();
        write_message(&mut left, Network::Mainnet, &message)
            .await
            .expect("write");
        let decoded = read_message(&mut right, Network::Mainnet)
            .await
            .expect("read");
        assert_eq!(decoded, message);
    }

    #[tokio::test]
    async fn framed_reader_handles_split_header() {
        let (mut left, right) = tokio::io::duplex(1024);
        let message = Message::verack();
        write_message(&mut left, Network::Testnet, &message)
            .await
            .expect("write");
        drop(left);
        let mut reader = FramedReader::new();
        let decoded = reader
            .read_message(&mut { right }, Network::Testnet)
            .await
            .expect("read");
        assert_eq!(decoded.command, "verack");
    }

    #[tokio::test]
    async fn message_writer_send_waits_for_bounded_capacity() {
        use std::pin::Pin;
        use std::sync::{Arc, Mutex};
        use std::task::{Context, Poll};
        use tokio::io::AsyncWrite;
        use tokio::sync::Notify;
        use tokio::time::{sleep, Duration};

        struct PausedWriter {
            gate: Arc<Notify>,
            released: Arc<Mutex<bool>>,
        }

        impl AsyncWrite for PausedWriter {
            fn poll_write(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                if *self.released.lock().expect("lock") {
                    return Poll::Ready(Ok(buf.len()));
                }
                self.gate.notify_waiters();
                cx.waker().wake_by_ref();
                Poll::Pending
            }

            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }

            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let gate = Arc::new(Notify::new());
        let released = Arc::new(Mutex::new(false));
        let writer = PausedWriter {
            gate: Arc::clone(&gate),
            released: Arc::clone(&released),
        };
        let (message_writer, handle) = MessageWriter::spawn(writer, Network::Testnet, 1);

        message_writer
            .send(Message::verack())
            .await
            .expect("first send");
        sleep(Duration::from_millis(20)).await;
        message_writer
            .send(Message::verack())
            .await
            .expect("second send fills queue");

        let third_send = tokio::spawn(async move { message_writer.send(Message::verack()).await });

        sleep(Duration::from_millis(50)).await;
        assert!(!third_send.is_finished());

        *released.lock().expect("lock") = true;
        gate.notify_waiters();

        tokio::time::timeout(Duration::from_secs(1), third_send)
            .await
            .expect("third send should complete after writer unblocks")
            .expect("join")
            .expect("third send");

        let _ = handle.await;
    }
}
