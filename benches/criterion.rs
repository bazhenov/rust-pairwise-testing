mod test_funcs;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use rust_pairwise_testing::Generator;
use test_funcs::{factorial, std_count, std_count_rev, std_take, sum, RandomStringGenerator};

fn sum_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("arithmetic");

    group.bench_function("sum_50000", |b| {
        b.iter(|| sum(50000));
    });

    group.bench_function("sum_49500", |b| {
        b.iter(|| sum(49500));
    });

    group.bench_function("factorial_500", |b| {
        b.iter(|| factorial(500));
    });

    group.bench_function("factorial_495", |b| {
        b.iter(|| factorial(495));
    });

    group.finish()
}

fn utf8_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8");

    group.bench_function("std_length_4950", |b| {
        let mut generator = RandomStringGenerator::new().unwrap();
        b.iter_batched(
            || generator.next_payload(),
            |s| std_take::<4950>(&s),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("std_length_5000", |b| {
        let mut generator = RandomStringGenerator::new().unwrap();
        b.iter_batched(
            || generator.next_payload(),
            |s| std_take::<5000>(&s),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("std_count", |b| {
        let mut generator = RandomStringGenerator::new().unwrap();
        b.iter_batched(
            || generator.next_payload(),
            |s| std_count(&s),
            BatchSize::SmallInput,
        );
    });

    group.bench_function("std_count_rev", |b| {
        let mut generator = RandomStringGenerator::new().unwrap();
        b.iter_batched(
            || generator.next_payload(),
            |s| std_count_rev(&s),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(benches, utf8_benchmark, sum_benchmark);
criterion_main!(benches);