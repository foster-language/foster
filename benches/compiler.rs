use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use foster::vm::CompileOptions;

const SOURCE: &str = include_str!("../benchmarks/fibonacci.fos");
#[path = "../benchmarks/support/native_dispatch.rs"]
mod native_sources;

fn compiler_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compiler");
    group
        .sample_size(40)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));

    group.bench_function("front_end_and_checked_hir", |bencher| {
        bencher.iter(|| foster::compile(black_box(SOURCE)).unwrap());
    });

    let compilation = foster::compile(SOURCE).unwrap();
    group.bench_function("bytecode_unoptimized", |bencher| {
        bencher.iter(|| {
            foster::vm::compile_with_options(
                black_box(&compilation),
                CompileOptions { optimize: false },
            )
            .unwrap()
        });
    });
    group.bench_function("bytecode_optimized", |bencher| {
        bencher.iter(|| {
            foster::vm::compile_with_options(
                black_box(&compilation),
                CompileOptions { optimize: true },
            )
            .unwrap()
        });
    });

    // Stress many distinct constants and values kept live across a loop.
    // Source generation and front-end checking stay outside the timed region.
    let mut constants = String::from("func main() -> Int { let value = 0\n");
    for value in 1..=256 {
        constants.push_str(&format!("value = value + {value}\n"));
    }
    constants.push_str("value }");
    let mut loop_source = String::from("func work(seed: Int) -> Int {\n");
    for index in 0..64 {
        loop_source.push_str(&format!("let value{index} = seed + {}\n", index + 1000));
    }
    loop_source.push_str("let total = seed\nloop { break if total >= 64\ntotal = total + 1 }\n");
    for index in 0..64 {
        loop_source.push_str(&format!("value{index} + "));
    }
    loop_source.push_str("total }\nfunc main() -> Int { work(1) }");
    for (name, source) in [
        ("bytecode_many_constants", constants),
        ("bytecode_loop_liveness", loop_source),
    ] {
        let compilation = foster::compile(&source).unwrap();
        group.bench_function(name, |bencher| {
            bencher.iter(|| {
                foster::vm::compile_with_options(
                    black_box(&compilation),
                    CompileOptions { optimize: true },
                )
                .unwrap()
            });
        });
    }
    group.finish();
}

fn native_dispatch_benchmarks(criterion: &mut Criterion) {
    for (name, sizes, source) in [
        (
            "native_candidates",
            &[4, 16, 32][..],
            native_sources::candidates as fn(usize) -> String,
        ),
        (
            "native_views",
            &[4, 16, 32][..],
            native_sources::views as fn(usize) -> String,
        ),
    ] {
        let mut group = criterion.benchmark_group(name);
        group
            .sample_size(10)
            .warm_up_time(Duration::from_millis(200))
            .measurement_time(Duration::from_secs(1));
        for &size in sizes {
            let compilation = foster::compile(&source(size)).unwrap();
            let options = foster::native::CompileOptions { optimize: false };
            // Warm shared SSA outside the timed region; emission reuses one plan.
            let prepared = foster::native::prepare_with_options(&compilation, options).unwrap();
            group.bench_with_input(BenchmarkId::new("prepare", size), &size, |bencher, _| {
                bencher.iter(|| {
                    foster::native::prepare_with_options(black_box(&compilation), options).unwrap()
                });
            });
            group.bench_with_input(BenchmarkId::new("emit", size), &size, |bencher, _| {
                bencher.iter(|| black_box(&prepared).compile_object(options).unwrap());
            });
        }
        group.finish();
    }
}

criterion_group!(benches, compiler_benchmarks, native_dispatch_benchmarks);
criterion_main!(benches);
