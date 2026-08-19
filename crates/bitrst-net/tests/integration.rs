//! Integration tests for multi-peer block relay.

use std::time::Duration;

use bitrst_core::{Block, BlockHeader, ChainHandle, MempoolHandle, Target};
use bitrst_net::codec::{decode_inv, encode_getdata, encode_inv};
use bitrst_net::constants::{Network, MAX_PEER_EVENTS};
use bitrst_net::envelope::MessageHeader;
use bitrst_net::framing::{read_message, write_message};
use bitrst_net::handshake::{ConnectionDirection, HandshakeConfig};
use bitrst_net::message::{InvType, InventoryVector, Message, MessagePayload};
use bitrst_net::peer::{spawn_peer, PeerCommand, PeerContext};
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

async fn client_handshake(stream: &mut tokio::net::TcpStream, nonce: u64) {
    write_message(
        stream,
        Network::Testnet,
        &Message::version(bitrst_net::codec::default_version_message(nonce, 1, 0)),
    )
    .await
    .expect("write version");
    match read_message(stream, Network::Testnet)
        .await
        .expect("peer version")
        .payload
    {
        MessagePayload::Version(_) => {}
        other => panic!("expected version, got {other:?}"),
    }
    assert_eq!(
        read_message(stream, Network::Testnet)
            .await
            .expect("verack")
            .command,
        "verack"
    );
    write_message(stream, Network::Testnet, &Message::verack())
        .await
        .expect("write verack");
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

    let (leader_event_tx, mut leader_events) = mpsc::channel(MAX_PEER_EVENTS);
    let server = tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.expect("accept");
        let (cmd_tx, handle) = spawn_peer(
            stream,
            PeerContext::new(
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                leader_chain,
                MempoolHandle::new(),
                HandshakeConfig {
                    local_nonce: 100,
                    timeout: Duration::from_secs(5),
                },
                0,
                leader_event_tx,
            ),
        );
        (cmd_tx, handle)
    });

    sleep(Duration::from_millis(50)).await;
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let (follower_event_tx, mut follower_events) = mpsc::channel(MAX_PEER_EVENTS);
    let (follower_cmd, follower_handle) = spawn_peer(
        stream,
        PeerContext::new(
            addr,
            ConnectionDirection::Outbound,
            Network::Testnet,
            follower_chain.clone(),
            MempoolHandle::new(),
            HandshakeConfig {
                local_nonce: 200,
                timeout: Duration::from_secs(5),
            },
            0,
            follower_event_tx,
        ),
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

#[tokio::test]
async fn inv_getdata_block_wire_flow_updates_server_chain() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let genesis = genesis_block();
    let child = child_block(&genesis);
    let child_hash = child.hash();

    let server_chain = ChainHandle::new_genesis(genesis, NETWORK_TIME).expect("genesis");
    let server_chain_check = server_chain.clone();

    let (server_event_tx, mut server_events) = mpsc::channel(MAX_PEER_EVENTS);
    let server = tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.expect("accept");
        let (cmd_tx, handle) = spawn_peer(
            stream,
            PeerContext::new(
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                server_chain,
                MempoolHandle::new(),
                HandshakeConfig {
                    local_nonce: 301,
                    timeout: Duration::from_secs(5),
                },
                0,
                server_event_tx,
            ),
        );
        (cmd_tx, handle)
    });

    sleep(Duration::from_millis(50)).await;
    let mut client = tokio::net::TcpStream::connect(addr).await.expect("connect");
    client_handshake(&mut client, 401).await;

    let _ = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server handshake timeout");

    let inv_items = vec![InventoryVector {
        inv_type: InvType::Block,
        hash: child_hash,
    }];
    let expected_inv_payload = encode_inv(&inv_items).expect("encode inv");
    write_message(
        &mut client,
        Network::Testnet,
        &Message::inv(inv_items.clone()),
    )
    .await
    .expect("write inv");

    let getdata_message = tokio::time::timeout(
        Duration::from_secs(5),
        read_message(&mut client, Network::Testnet),
    )
    .await
    .expect("getdata timeout")
    .expect("read getdata");
    assert_eq!(getdata_message.command, "getdata");
    match getdata_message.payload {
        MessagePayload::GetData(items) => {
            assert_eq!(items, inv_items);
            assert_eq!(
                encode_getdata(&items).expect("encode getdata"),
                encode_getdata(&inv_items).expect("encode expected getdata")
            );
        }
        other => panic!("expected getdata payload, got {other:?}"),
    }

    write_message(
        &mut client,
        Network::Testnet,
        &Message::block(child.clone()),
    )
    .await
    .expect("write block");

    let (server_cmd, server_handle) = server.await.expect("server");

    for _ in 0..40 {
        if server_chain_check.height().expect("height") == 1 {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(server_chain_check.height().expect("height"), 1);
    assert_eq!(server_chain_check.tip_hash().expect("tip"), child_hash);
    assert_eq!(
        decode_inv(&expected_inv_payload).expect("decode inv"),
        inv_items
    );

    let _ = server_cmd.send(PeerCommand::Shutdown).await;
    let _ = client.shutdown().await;
    server_handle.abort();
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

#[tokio::test]
async fn inv_wire_payload_matches_codec_bytes() {
    let items = vec![InventoryVector {
        inv_type: InvType::Block,
        hash: [0x55; 32],
    }];
    let payload = encode_inv(&items).expect("encode");
    let (mut client, mut server) = tokio::io::duplex(4096);
    write_message(&mut client, Network::Testnet, &Message::inv(items.clone()))
        .await
        .expect("write inv");
    let message = read_message(&mut server, Network::Testnet)
        .await
        .expect("read inv");
    assert_eq!(message.command, "inv");
    match message.payload {
        MessagePayload::Inv(decoded) => {
            assert_eq!(decoded, items);
            assert_eq!(decode_inv(&payload).expect("decode"), items);
        }
        other => panic!("expected inv payload, got {other:?}"),
    }
}

#[tokio::test]
async fn tx_inv_getdata_wire_flow_admits_to_follower_mempool() {
    use bitrst_core::MempoolHandle;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let genesis = genesis_block();
    let server_chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
    let server_mempool = MempoolHandle::new();
    let server_mempool_check = server_mempool.clone();
    let (spend, txid) = funded_p2pkh_spend_on(&server_chain, &genesis);

    let (server_event_tx, mut server_events) = mpsc::channel(MAX_PEER_EVENTS);
    let server = tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.expect("accept");
        let (cmd_tx, handle) = spawn_peer(
            stream,
            PeerContext::new(
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                server_chain,
                server_mempool,
                HandshakeConfig {
                    local_nonce: 501,
                    timeout: Duration::from_secs(5),
                },
                1,
                server_event_tx,
            ),
        );
        (cmd_tx, handle)
    });

    sleep(Duration::from_millis(50)).await;
    let mut client = tokio::net::TcpStream::connect(addr).await.expect("connect");
    client_handshake(&mut client, 601).await;

    let _ = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server handshake timeout");

    let (server_cmd, server_handle) = server.await.expect("server");

    write_message(
        &mut client,
        Network::Testnet,
        &Message::inv(vec![InventoryVector {
            inv_type: InvType::Transaction,
            hash: txid,
        }]),
    )
    .await
    .expect("write inv");

    let getdata_message = tokio::time::timeout(
        Duration::from_secs(5),
        read_message(&mut client, Network::Testnet),
    )
    .await
    .expect("getdata timeout")
    .expect("read getdata");
    assert_eq!(getdata_message.command, "getdata");

    write_message(&mut client, Network::Testnet, &Message::tx(spend))
        .await
        .expect("write tx");

    for _ in 0..40 {
        if server_mempool_check.contains(&txid).unwrap_or(false) {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }

    assert!(server_mempool_check.contains(&txid).expect("contains"));
    let _ = server_cmd.send(PeerCommand::Shutdown).await;
    server_handle.abort();
}

#[tokio::test]
async fn conflicting_tx_inv_is_not_admitted_to_mempool() {
    use bitrst_core::MempoolHandle;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let genesis = genesis_block();
    let server_chain = ChainHandle::new_genesis(genesis.clone(), NETWORK_TIME).expect("genesis");
    let server_mempool = MempoolHandle::new();
    let server_mempool_check = server_mempool.clone();
    let (mut spend, txid) = funded_p2pkh_spend_on(&server_chain, &genesis);
    server_chain
        .with_chain(|chain| server_mempool.accept_tx(spend.clone(), chain.utxo()))
        .expect("chain")
        .expect("resident");
    assert!(server_mempool_check.contains(&txid).expect("seeded"));

    let (server_event_tx, mut server_events) = mpsc::channel(MAX_PEER_EVENTS);
    let server = tokio::spawn(async move {
        let (stream, peer_addr) = listener.accept().await.expect("accept");
        let (cmd_tx, handle) = spawn_peer(
            stream,
            PeerContext::new(
                peer_addr,
                ConnectionDirection::Inbound,
                Network::Testnet,
                server_chain,
                server_mempool,
                HandshakeConfig {
                    local_nonce: 701,
                    timeout: Duration::from_secs(5),
                },
                1,
                server_event_tx,
            ),
        );
        (cmd_tx, handle)
    });

    sleep(Duration::from_millis(50)).await;
    let mut client = tokio::net::TcpStream::connect(addr).await.expect("connect");
    client_handshake(&mut client, 801).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), server_events.recv())
        .await
        .expect("server handshake timeout");

    let (server_cmd, server_handle) = server.await.expect("server");

    spend.inputs[0].script_sig = vec![0x01];
    write_message(
        &mut client,
        Network::Testnet,
        &Message::inv(vec![InventoryVector {
            inv_type: InvType::Transaction,
            hash: spend.txid(),
        }]),
    )
    .await
    .expect("write inv");

    let getdata_message = tokio::time::timeout(
        Duration::from_secs(5),
        read_message(&mut client, Network::Testnet),
    )
    .await
    .expect("getdata timeout")
    .expect("read getdata");
    assert_eq!(getdata_message.command, "getdata");

    write_message(&mut client, Network::Testnet, &Message::tx(spend))
        .await
        .expect("write tx");

    sleep(Duration::from_millis(300)).await;
    assert_eq!(server_mempool_check.len().expect("len"), 1);
    assert!(server_mempool_check.contains(&txid).expect("original"));
    let _ = server_cmd.send(PeerCommand::Shutdown).await;
    server_handle.abort();
}

fn funded_p2pkh_spend(chain: &ChainHandle) -> (bitrst_core::Transaction, [u8; 32]) {
    funded_p2pkh_spend_on(chain, &genesis_block())
}

fn funded_p2pkh_spend_on(
    chain: &ChainHandle,
    genesis: &Block,
) -> (bitrst_core::Transaction, [u8; 32]) {
    use bitrst_core::{Transaction, TxInput, TxOutput};
    use bitrst_crypto::hash160::hash160;
    use bitrst_script::{p2pkh_script_pubkey, p2pkh_script_sig};
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[0x44; 32]).expect("secret");
    let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);
    let pubkey_bytes = pk.serialize();
    let lock_script = p2pkh_script_pubkey(&hash160(&pubkey_bytes));

    let mut fund_block = child_block(genesis);
    fund_block.transactions[0].outputs[0].script_pubkey = lock_script.clone();
    fund_block.header.merkle_root = fund_block.merkle_root().expect("merkle");
    let target = Target::from_bits(TEST_BITS).expect("bits");
    while !target.meets(&fund_block.header.hash()) {
        fund_block.header.nonce = fund_block.header.nonce.wrapping_add(1);
    }
    chain.connect_block(fund_block).expect("fund");
    let funding_txid = chain
        .get_block(&chain.tip_hash().expect("tip"))
        .expect("get")
        .expect("block")
        .transactions[0]
        .txid();

    let mut spend = Transaction {
        version: 1,
        inputs: vec![TxInput {
            previous_output: funding_txid,
            index: 0,
            script_sig: Vec::new(),
            sequence: u32::MAX,
        }],
        outputs: vec![TxOutput {
            value: 49_0000_0000,
            script_pubkey: Vec::new(),
        }],
        lock_time: 0,
    };
    let prev_scripts = vec![lock_script];
    let sighash = bitrst_core::sighash_all(&spend, 0, &prev_scripts).expect("sighash");
    let sig = secp.sign_ecdsa(&Message::from_digest(sighash), &sk);
    let mut sig_bytes = sig.serialize_der().to_vec();
    sig_bytes.push(0x01);
    spend.inputs[0].script_sig = p2pkh_script_sig(&sig_bytes, &pubkey_bytes);

    let spend_txid = spend.txid();
    (spend, spend_txid)
}
