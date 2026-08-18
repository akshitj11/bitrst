//! Integration tests for multi-peer block relay.

use std::time::Duration;

use bitrst_core::{Block, BlockHeader, ChainHandle, Target};
use bitrst_net::constants::Network;
use bitrst_net::envelope::MessageHeader;
use bitrst_net::framing::read_message;
use bitrst_net::handshake::{ConnectionDirection, HandshakeConfig};
use bitrst_net::message::Message;
use bitrst_net::peer::{spawn_peer, PeerCommand};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::sleep;

const NETWORK_TIME: u32 = 1_231_006_505;
const TEST_BITS: u32 = 0x1f00_ffff;

fn genesis_block() -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: [0u8; 32],
        merkle_root: [0u8; 32],
        time: NETWORK_TIME,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, 0, 50_0000_0000);
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&block.header.hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    block
}

fn child_block(parent: &Block) -> Block {
    let header = BlockHeader {
        version: 1,
        prev_blockhash: parent.hash(),
        merkle_root: [0u8; 32],
        time: NETWORK_TIME + 600,
        bits: TEST_BITS,
        nonce: 0,
    };
    let mut block = Block::coinbase(header, 1, 50_0000_0000);
    block.header.merkle_root = block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&block.header.hash()) {
        block.header.nonce = block.header.nonce.wrapping_add(1);
    }
    block
}

#[tokio::test]
async fn checksum_mismatch_is_rejected() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let payload = b"payload-bytes";
    let mut header = MessageHeader::encode(
        "verack",
        payload,
        Network::Testnet.magic(),
        bitrst_net::constants::MAX_PAYLOAD_SIZE,
    )
    .expect("header");
    header[20] ^= 0xff;
    client.write_all(&header).await.expect("header");
    client.write_all(payload).await.expect("payload");

    let result = read_message(&mut server, Network::Testnet).await;
    assert!(matches!(
        result,
        Err(bitrst_net::NetError::ChecksumMismatch { .. })
    ));
}

#[tokio::test]
async fn truncated_inv_payload_is_rejected() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let bad_payload = vec![0x01, 0x00, 0x00, 0x00, 0x02];
    let header = MessageHeader::encode(
        "inv",
        &bad_payload,
        Network::Testnet.magic(),
        bitrst_net::constants::MAX_PAYLOAD_SIZE,
    )
    .expect("header");
    client.write_all(&header).await.expect("header");
    client.write_all(&bad_payload).await.expect("payload");

    let result = read_message(&mut server, Network::Testnet).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn malformed_block_payload_is_rejected_without_panic() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let bad_payload = vec![0xff; 16];
    let header = MessageHeader::encode(
        "block",
        &bad_payload,
        Network::Testnet.magic(),
        bitrst_net::constants::MAX_PAYLOAD_SIZE,
    )
    .expect("header");
    client.write_all(&header).await.expect("header");
    client.write_all(&bad_payload).await.expect("payload");

    let result = read_message(&mut server, Network::Testnet).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn two_node_block_relay_updates_follower_chain() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let leader_chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
    let follower_chain = ChainHandle::new_genesis(genesis_block(), NETWORK_TIME).expect("genesis");
    let child = child_block(&genesis_block());

    let (leader_event_tx, mut leader_events) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.expect("accept");
        let (cmd_tx, handle) = spawn_peer(
            stream,
            peer_addr,
            ConnectionDirection::Inbound,
            Network::Testnet,
            leader_chain,
            HandshakeConfig {
                local_nonce: 100,
                timeout: Duration::from_secs(5),
            },
            0,
            leader_event_tx,
        );
        (cmd_tx, handle)
    });

    sleep(Duration::from_millis(50)).await;
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let (follower_event_tx, mut follower_events) = mpsc::unbounded_channel();
    let (follower_cmd, follower_handle) = spawn_peer(
        stream,
        addr,
        ConnectionDirection::Outbound,
        Network::Testnet,
        follower_chain.clone(),
        HandshakeConfig {
            local_nonce: 200,
            timeout: Duration::from_secs(5),
        },
        0,
        follower_event_tx,
    );

    let _ = tokio::time::timeout(Duration::from_secs(5), leader_events.recv())
        .await
        .expect("leader handshake timeout");
    let _ = tokio::time::timeout(Duration::from_secs(5), follower_events.recv())
        .await
        .expect("follower handshake timeout");

    let (leader_cmd, leader_handle) = server.await.expect("server");
    leader_cmd
        .send(PeerCommand::Send(Message::block(child.clone())))
        .await
        .expect("send block");

    for _ in 0..40 {
        if follower_chain.height().expect("height") == 1 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(follower_chain.height().expect("height"), 1);
    assert_eq!(follower_chain.tip_hash().expect("tip"), child.hash());

    drop(follower_cmd);
    follower_handle.abort();
    drop(leader_cmd);
    leader_handle.abort();
}

#[test]
fn version_header_vector_decodes_known_fields() {
    let message = bitrst_net::codec::default_version_message(0x553b_9a3b_3bd4_3308, 999, 725);
    let mut message = message;
    message.version = 70015;
    message.user_agent = "/satoshi:0.15.1/".to_owned();
    message.relay = false;
    let payload = bitrst_net::codec::encode_version(&message).expect("encode");
    let version = bitrst_net::codec::decode_version(&payload).expect("decode version");
    assert_eq!(version.version, 70015);
    assert_eq!(version.user_agent, "/satoshi:0.15.1/");
    assert_eq!(version.start_height, 725);
    assert!(!version.relay);
}
