#!/usr/bin/env bash
set -eo pipefail

cargo +nightly export ./target/benchmarks -- bench --bench='search-*'

for time in {1..100}; do
    printf ' time %3d ms : ' $time
    ./target/benchmarks/search_ord compare ./target/benchmarks/search_ord -t 1 --sampler=flat $@ \
        -f 'search/u32/1024/nodup' --time-slice=$time
done
