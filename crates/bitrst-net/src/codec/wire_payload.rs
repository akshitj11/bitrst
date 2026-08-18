//! Top-level payload encode/decode dispatch by command name.

use bitrst_core::{Block, Transaction};

use crate::error::NetError;
use crate::message::{Message, MessagePayload};

use super::inv::{decode_getdata, decode_inv, encode_getdata, encode_inv};
use super::version::{decode_version, encode_version};

/// Encodes a [`Message`] into `(command, payload bytes)`.
///
/// # Errors
///
/// Returns [`NetError`] when payload encoding fails.
pub fn encode_payload(message: &Message) -> Result<(String, Vec<u8>), NetError> {
    match &message.payload {
        MessagePayload::Verack => Ok((message.command.clone(), Vec::new())),
        MessagePayload::Version(version) => Ok((message.command.clone(), encode_version(version)?)),
        MessagePayload::Inv(items) => Ok((message.command.clone(), encode_inv(items)?)),
        MessagePayload::GetData(items) => Ok((message.command.clone(), encode_getdata(items)?)),
        MessagePayload::Block(block) => Ok((message.command.clone(), block.serialize())),
        MessagePayload::Tx(transaction) => Ok((message.command.clone(), transaction.serialize())),
    }
}

/// Decodes a command/payload pair into a [`Message`].
///
/// # Errors
///
/// Returns [`NetError`] when the command is unknown or payload decoding fails.
pub fn decode_payload(command: &str, payload: &[u8]) -> Result<Message, NetError> {
    let decoded = match command {
        "verack" if payload.is_empty() => MessagePayload::Verack,
        "version" => MessagePayload::Version(decode_version(payload)?),
        "inv" => MessagePayload::Inv(decode_inv(payload)?),
        "getdata" => MessagePayload::GetData(decode_getdata(payload)?),
        "block" => MessagePayload::Block(Block::deserialize(payload)?),
        "tx" => MessagePayload::Tx(Transaction::deserialize(payload)?),
        _ => return Err(NetError::HandshakeViolation("unsupported command")),
    };

    Ok(Message {
        command: command.to_owned(),
        payload: decoded,
    })
}

#[cfg(test)]
mod tests {
    use super::decode_payload;
    use crate::message::Message;
    use bitrst_core::{Block, BlockHeader, Transaction};

    #[test]
    fn tx_roundtrip_through_payload_codec() {
        let tx = Transaction::coinbase(1, 50_0000_0000);
        let message = Message::tx(tx.clone());
        let (command, payload) = super::encode_payload(&message).expect("encode");
        let decoded = decode_payload(&command, &payload).expect("decode");
        assert_eq!(decoded, message);
        assert_eq!(decoded.payload, crate::message::MessagePayload::Tx(tx));
    }

    #[test]
    fn block_roundtrip_through_payload_codec() {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0; 32],
            merkle_root: [0; 32],
            time: 1,
            bits: 0x1f00_ffff,
            nonce: 0,
        };
        let block = Block::coinbase(header, 1, 50_0000_0000);
        let message = Message::block(block.clone());
        let (command, payload) = super::encode_payload(&message).expect("encode");
        let decoded = decode_payload(&command, &payload).expect("decode");
        assert_eq!(
            decoded.payload,
            crate::message::MessagePayload::Block(block)
        );
    }
}
