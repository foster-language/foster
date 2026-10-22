# Foster

Foster is an experimental statically typed language with structural contracts,
single ownership, group-based references, inferred effects, and remote objects.
The compiler and runtimes are written in Rust; the standard library is written
in Foster.

## Try it

Build with `cargo build --release --bin foster`. The executable is
`target/release/foster` (`foster.exe` on Windows). Use release builds for normal
execution and performance measurements.

```powershell
cargo run --release --bin foster -- run examples/live_inventory_pipeline.fos
cargo run --release --bin foster -- check examples/json_parser
cargo run --release --bin foster -- test library
```

With the executable on `PATH`:

```powershell
foster init hello-foster
cd hello-foster
foster run
foster check
foster fmt --check
```

Read [Writing Foster](documentation/writing-foster.md) for complete, tested
programs, or browse the [examples](examples/README.md). Coding agents must also
read [AGENTS.md](AGENTS.md).

## Projects and dependencies

`foster init` creates `foster.toml` and `src/main.fos` without overwriting
existing files. A minimal manifest is:

```toml
[package]
name = "hello-foster"
source = "src"
entry = "main.fos"
```

Only `package.name` is required. `source` defaults to `src` and must stay within
the project. `entry` defaults to `main.fos`; source libraries select `lib.fos`.
An explicit entry must exist and be a filename without directories. It supplies
the package root; other source files define submodules. A different entry cannot
coexist with another `main.fos` in that source root.

Add source projects or compiled libraries as dependencies:

```toml
[dependencies]
collections = { path = "../foster-collections" }
math = { path = "vendor/math.flib" }
```

Dependency keys define import namespaces. A dependency's entry is mounted as
`collections`, and its `map.fos` as `collections.map`. Internal imports are rebased
to that namespace; `core`, `std`, and declared dependency names retain their
meaning. Paths resolve transitively. Dependency names must be portable module
identifiers other than `core` or `std`; cycles and conflicting names are errors.
See [modules](documentation/language-design.md#source-files-and-modules) and
[compiled libraries](documentation/compiled-libraries.md).

Commands accept a source file, project directory, or manifest. With no path they
search for the nearest `foster.toml`; `fmt` and `documentation` use the current
directory if none exists. A directory with `main.fos` also works without a
manifest. `run` calls `main`, which accepts no arguments or one
`std.process.Arguments` value. Pass application arguments after `--`.

## Commands

| Task | Command |
| --- | --- |
| Run with application arguments | `foster run app.fos -- input.txt` |
| Run without Foster optimization | `foster run app.fos --no-optimize` |
| Check or format source | `foster check path`, `foster fmt path`, `foster fmt path --check` |
| Run source tests | `foster test path` |
| Build and run bytecode | `foster build app.fos -o app.fbc`, `foster run app.fbc` |
| Build a native executable | `foster build app.fos --native -o app.exe` |
| Inspect native IR | `foster build app.fos --native --emit native-ir` |
| Build a reusable library | `foster build path/to/library --library -o math.flib` |
| Package an application | `foster pack path/to/project -o app.fpk` |
| Run a package | `foster run app.fpk` |
| Generate and view API documentation | `foster documentation library --serve` |
| Start the language server | `foster lsp` |

Use `<command> --help` for options. The [documentation index](documentation/README.md)
links the language, ownership, library, backend, C integration, and package guides.
The [VS Code extension](editors/vscode/README.md) provides editor integration.

API documentation comes from source comments and resolved signatures. Its default
destination is the selected package's `docs` directory. Use `--output PATH` for
another destination, `--include-private` for internal APIs, and `--serve` to open
a local server. `foster serve-documentation PATH` serves an existing site. Both
serving commands support `--port NUMBER` and `--no-open`.

## Working in the repository

Follow the [development policy](documentation/development-policy.md) and use the
[workspace guide](documentation/workspace.md) for package boundaries and builds.

| Area | Guide |
| --- | --- |
| Language checking and code generation | [Compiler](compiler/README.md) |
| Executable formats | [Bytecode](bytecode/README.md) |
| Execution and platform services | [VM](vm/README.md), [native runtime](runtime/README.md), [host](host/README.md) |
| CLI, language server, debugger, documentation | [Driver](src/README.md), [tools](tools/README.md) |
| Foster library source | [Standard library](library/README.md), [optional packages](packages/README.md) |
| Correctness and CI | [Testing](documentation/testing.md), [test suites](tests/README.md), [CI selection](tools/ci/README.md) |
| Performance | [Benchmarks](benchmarks/README.md), [measurement guide](documentation/benchmarking.md), [research](research/README.md) |

Run `cargo test` for the full Rust workspace checks. For focused integration
suites use `python tools/integration_tests.py --suite tooling --jobs 2` (Python
3.11 or newer); omit `--suite` to run all suites. Portable Foster tests run with
`foster test tests/foster` and `foster test library`.
