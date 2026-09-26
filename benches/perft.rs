use chess::core::{Position, perft};
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn perft_benches(c: &mut Criterion) {
    let start = Position::startpos();
    let kiwipete =
        Position::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
            .unwrap();
    let mut group = c.benchmark_group("perft");
    group.sample_size(10);
    group.bench_function("startpos depth 5", |b| {
        b.iter(|| perft(black_box(&start), 5))
    });
    group.bench_function("kiwipete depth 4", |b| {
        b.iter(|| perft(black_box(&kiwipete), 4))
    });
    group.finish();
}

criterion_group!(benches, perft_benches);
criterion_main!(benches);
