//! Backup container benchmarks: key derivation and streaming encryption.
//!
//!     cargo bench -p mt-backup
//!
//! Host numbers are **relative only** (see docs/PERFORMANCE.md). On a low-end phone the
//! Argon2id step, which is memory-bound, is the number to measure on the device itself.

#![allow(
    clippy::unwrap_used,
    reason = "a benchmark that fails to set up has failed"
)]

use std::{
    hint::black_box,
    io::{Cursor, Read, Write},
    time::Duration,
};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use mt_backup::{BackupReader, BackupWriter, KdfParams};

const PASS: &str = "kopi susu gula aren enak";
/// About the size of a 100 000-transaction logical export.
const PAYLOAD: usize = 8 * 1024 * 1024;

fn payload() -> Vec<u8> {
    // Not compressible-looking: the container does not compress, but keep it honest.
    (0..PAYLOAD)
        .map(|i| u8::try_from((i.wrapping_mul(2_654_435_761)) >> 13 & 0xff).unwrap())
        .collect()
}

fn seal(data: &[u8], params: KdfParams) -> Vec<u8> {
    let mut writer = BackupWriter::new(Vec::new(), PASS, params).unwrap();
    writer.write_all(data).unwrap();
    writer.finish().unwrap()
}

fn kdf(c: &mut Criterion) {
    let mut group =
        c.benchmark_group("argon2id key derivation (the cost of opening or making a backup)");
    group
        .sample_size(10)
        .measurement_time(Duration::from_secs(10));
    let tiny = seal(b"x", KdfParams::DEFAULT);
    group.bench_function(
        "production cost (64 MiB, t=3, p=1): open a tiny backup",
        |b| {
            b.iter(|| {
                let mut reader = BackupReader::open(Cursor::new(&tiny), PASS).unwrap();
                let mut out = Vec::new();
                reader.read_to_end(&mut out).unwrap();
                black_box(out)
            });
        },
    );
    group.finish();
}

fn stream(c: &mut Criterion) {
    let data = payload();
    let sealed = seal(&data, KdfParams::for_tests());
    let mut group =
        c.benchmark_group("streaming encryption, 8 MiB payload (cheap KDF, so this is the cipher)");
    group
        .throughput(Throughput::Bytes(PAYLOAD as u64))
        .sample_size(20);
    group.bench_function("encrypt", |b| {
        b.iter(|| black_box(seal(&data, KdfParams::for_tests())))
    });
    group.bench_function("decrypt and authenticate", |b| {
        b.iter(|| {
            let mut reader = BackupReader::open(Cursor::new(&sealed), PASS).unwrap();
            let mut out = Vec::with_capacity(PAYLOAD);
            reader.read_to_end(&mut out).unwrap();
            black_box(out)
        });
    });
    group.finish();
}

criterion_group!(benches, kdf, stream);
criterion_main!(benches);
