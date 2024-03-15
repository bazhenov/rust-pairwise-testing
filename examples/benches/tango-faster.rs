#![cfg_attr(feature = "align", feature(fn_align))]

use crate::test_funcs::{factorial, sum};
use std::rc::Rc;
use tango_bench::{
    benchmark_fn, generators::RandomVec, tango_benchmarks, tango_main, IntoBenchmarks,
};
use test_funcs::{
    create_str_benchmark, sort_unstable, str_count, str_take, IndexedString, INPUT_TEXT,
};

mod test_funcs;

fn num_benchmarks() -> impl IntoBenchmarks {
    [
        benchmark_fn("sum", |b| b.iter(|| sum(4950))),
        benchmark_fn("factorial", |b| b.iter(|| factorial(495))),
    ]
}

/// TODO remove duplication
fn vec_benchmarks() -> impl IntoBenchmarks {
    let mut benches = vec![];
    for size in [100, 1_000, 10_000, 100_000] {
        let mut random = RandomVec::<u64>::new(size);
        benches.push(benchmark_fn(format!("sort/{}", size), move |b| {
            random.sync(b.seed);
            let v = random.next_haystack();
            b.iter(move || sort_unstable(&v, &()))
        }))
    }
    benches
}

fn str_benchmarks() -> impl IntoBenchmarks {
    let input = Rc::new(IndexedString::from(INPUT_TEXT));
    [
        create_str_benchmark("str_length/random", &input, str_count),
        create_str_benchmark("str_length/random_limited", &input, |s| str_take(4950, s)),
    ]
}

tango_benchmarks!(str_benchmarks(), num_benchmarks(), vec_benchmarks());
tango_main!();
