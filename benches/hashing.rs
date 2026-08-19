//! SHA-256d hashing benchmarks (setup excluded from measured loop).

use std::hint::black_box;

use bitrst_core::BlockHeader;
use bitrst_crypto::sha256d::sha256d;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};

fn genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_blockhash: [0u8; 32],
        merkle_root: [
            0x3b, 0xa3, 0xed, 0xfd, 0x7a, 0x7b, 0x12, 0xb2, 0x7a, 0xc7, 0x2c, 0x3e, 0x67, 0x76,
            0x8f, 0x61, 0x7f, 0xc8, 0x1b, 0xc3, 0x88, 0x8a, 0x51, 0x32, 0x3a, 0x9f, 0xb8, 0xaa,
            0x4b, 0x1e, 0x5e, 0x4a,
        ],
        time: 1_231_006_505,
        bits: 0x1d00_ffff,
        nonce: 2_083_236_893,
    }
}

fn bench_sha256d_payload(c: &mut Criterion) {
    let payload = genesis_header().serialize();
    let mut group = c.benchmark_group("sha256d_payload");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("80_byte_header_bytes", |bencher| {
        bencher.iter(|| sha256d(black_box(&payload)));
    });
    group.finish();
}

fn bench_sha256d_header_hash(c: &mut Criterion) {
    let header = genesis_header();
    let mut group = c.benchmark_group("sha256d_header");
    group.throughput(Throughput::Bytes(80));
    group.bench_function("serialize_and_hash", |bencher| {
        bencher.iter_batched(
            || header.clone(),
            |header| black_box(header.hash()),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_sha256d_payload, bench_sha256d_header_hash);
criterion_main!(benches);
