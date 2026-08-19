//! Bounded proof-of-work nonce search benchmarks (setup excluded from measured loop).

use std::hint::black_box;

use bitrst_core::pow::Target;
use bitrst_core::BlockHeader;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

const NONCE_SEARCH_ATTEMPTS: u32 = 1_000;

fn search_template_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_blockhash: [0x11; 32],
        merkle_root: [0x22; 32],
        time: 1_700_000_000,
        bits: 0x1f00_ffff,
        nonce: 0,
    }
}

fn bounded_nonce_search(header: &mut BlockHeader, target: Target, attempts: u32) -> u32 {
    for step in 0..attempts {
        if target.meets(&header.hash()) {
            return step;
        }
        header.nonce = header.nonce.wrapping_add(1);
    }
    attempts
}

fn bench_bounded_nonce_search(c: &mut Criterion) {
    let target = Target::from_bits(search_template_header().bits).expect("valid test bits");
    let mut group = c.benchmark_group("bounded_nonce_search");
    group.bench_function("1000_header_hashes", |bencher| {
        bencher.iter_batched(
            || (search_template_header(), target),
            |(mut header, target)| {
                black_box(bounded_nonce_search(
                    &mut header,
                    target,
                    NONCE_SEARCH_ATTEMPTS,
                ))
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

fn bench_easy_target_nonce_search(c: &mut Criterion) {
    let target = Target::easy();
    let mut group = c.benchmark_group("bounded_nonce_search");
    group.bench_function("easy_target_until_solution", |bencher| {
        bencher.iter_batched(
            || search_template_header(),
            |mut header| {
                let mut attempts = 0u32;
                loop {
                    if target.meets(&header.hash()) {
                        return black_box(attempts);
                    }
                    header.nonce = header.nonce.wrapping_add(1);
                    attempts = attempts.wrapping_add(1);
                    if attempts >= NONCE_SEARCH_ATTEMPTS {
                        return black_box(attempts);
                    }
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_bounded_nonce_search,
    bench_easy_target_nonce_search
);
criterion_main!(benches);
