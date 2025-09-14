use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rand::RngCore;
use std::io::{Read, Write};
use std::path::PathBuf;

fn buf_reader_writer_write_only_throughput(c: &mut Criterion) {
    let mut rng = rand::rng();
    let mut group = c.benchmark_group("BufReadWriter::write::Throughput");
    let mut bytes = vec![0; 50];
    rng.fill_bytes(&mut bytes);

    // let total_num_bytes = 1_000_000_000;
    let total_num_bytes = 500_000_000;
    let num_writes = total_num_bytes / bytes.len();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode", |b| {
        b.iter(|| {
            let mut output = std::fs::File::create("tmp.bin")
                .map(bufrw::BufReaderWriter::new)
                .unwrap();
            for _ in 0..num_writes {
                output.write_all(&bytes).unwrap();
            }
            output.flush().unwrap();
        })
    });
    group.finish();
}

fn buf_writer_write_only_throughput(c: &mut Criterion) {
    let mut rng = rand::rng();
    let mut group = c.benchmark_group("BufWriter::write::Throughput");
    let mut bytes = vec![0; 50];
    rng.fill_bytes(&mut bytes);

    let total_num_bytes = 500_000_000;
    let num_writes = total_num_bytes / bytes.len();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode", |b| {
        b.iter(|| {
            let mut output = std::fs::File::create("tmp.bin")
                .map(std::io::BufWriter::new)
                .unwrap();
            for _ in 0..num_writes {
                output.write_all(&bytes).unwrap();
            }
            output.flush().unwrap();
        })
    });
    group.finish();
}

fn ensure_readable_file_exists() {
    if !PathBuf::new().join("tmp.bin").exists() {
        let mut rng = rand::rng();
        let mut bytes = vec![0; 50];
        rng.fill_bytes(&mut bytes);
        let mut output = std::fs::File::create("tmp.bin")
            .map(std::io::BufWriter::new)
            .unwrap();
        let total_num_bytes = 500_000_000;
        let num_writes = total_num_bytes / bytes.len();
        for _ in 0..num_writes {
            output.write_all(&bytes).unwrap();
        }
        output.flush().unwrap();
    }
}

fn buf_reader_writer_read_only_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("BufReadWriter::read::Throughput");
    let mut bytes = vec![0; 50];

    let total_num_bytes = 500_000_000;
    let num_writes = total_num_bytes / bytes.len();

    ensure_readable_file_exists();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode", |b| {
        b.iter(|| {
            let mut output = std::fs::File::open("tmp.bin")
                .map(bufrw::BufReaderWriter::new)
                .unwrap();
            for _ in 0..num_writes {
                output.read_exact(&mut bytes).unwrap();
            }
        })
    });
    group.finish();
}


fn buf_reader_read_only_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("BufReader::read::Throughput");
    let mut bytes = vec![0; 50];

    let total_num_bytes = 500_000_000;
    let num_writes = total_num_bytes / bytes.len();

    ensure_readable_file_exists();

    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("decode", |b| {
        b.iter(|| {
            let mut output = std::fs::File::open("tmp.bin")
                .map(std::io::BufReader::new)
                .unwrap();
            for _ in 0..num_writes {
                output.read_exact(&mut bytes).unwrap();
            }
        })
    });
    group.finish();
}


criterion_group!(
    benches,
    buf_reader_writer_write_only_throughput,
    buf_writer_write_only_throughput,
    buf_reader_writer_read_only_throughput,
    buf_reader_read_only_throughput
);
criterion_main!(benches);
