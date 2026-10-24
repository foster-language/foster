# Criterion entry points

`compiler.rs` measures frontend and lowering cost. `runtime.rs` measures VM
execution of precompiled workloads. Cargo registers both as Criterion benches
with `harness = false`.

```text
cargo bench --bench compiler
cargo bench --bench runtime
cargo bench --bench compiler -- native_
```

Run from the repository root. Workload sources and other harnesses are indexed in
[benchmarks](../benchmarks/README.md). Criterion output stays in `target/criterion/`.

The `native_candidates` and `native_views` groups measure preparation and object
emission separately as dispatch implementors and returned-view chains grow.
Source generation, checking, and initial shared SSA construction are outside the
timed regions. The source generators are also exercised by compiler regressions.
