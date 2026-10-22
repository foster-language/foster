# Foster documentation

Start with [Writing Foster](writing-foster.md) for complete, compiler-checked
examples and the [library guide](../library/README.md) to choose APIs.

## Language and library

| Need | Reference |
| --- | --- |
| Setup, projects, and commands | [Repository README](../README.md) |
| Syntax | [Language reference](language-design.md) |
| Behavioral rules and known gaps | [Semantic specification](semantics.md) |
| Borrowing, moves, and mutation | [Ownership](ownership.md), [closures](closures.md), [effects](effect-derivation.md) |
| Remote objects and failures | [Remote semantics](remote-semantics.md) |
| Library operations | [Library guide](../library/README.md); generate exact API signatures with `foster documentation library --serve` |
| Specialized APIs | [Time](time.md), [randomness](random.md), [Unicode](unicode.md), [JSON](json.md), [hash collections](hash-collections.md), [processes](processes.md) |
| Build and distribute | [Native compilation](native.md), [packages](package-format.md), [compiled libraries](compiled-libraries.md) |
| Native services | [C integration](c-integration.md), [host providers](host-providers.md) |

Source comments define individual library operations. Guides explain usage;
the semantic specification defines the language contract. Proposed work belongs
in the [roadmap](roadmap.md).

## Compiler contributors

Follow [AGENTS.md](../AGENTS.md) and the [development policy](development-policy.md).

| Area | Reference |
| --- | --- |
| Architecture | [Workspace](workspace.md), [VM](vm.md), [native backend](native.md) |
| Executable artifacts | [Bytecode format](binary-format.md), [symbolic modules](symbolic-modules.md) |
| Analysis and diagnostics | [Diagnostics](diagnostics.md), [interactive checking](incremental-checking.md), [analysis precision](analysis-precision.md) |
| Verification | [Testing](testing.md), [coverage](test-coverage.md), [ownership verification](ownership-verification.md) |
| Library contracts | [Documentation standard](../library/DOCUMENTATION.md), [contract inventory](library-contract-audit.md), [intrinsic boundary](core-library.md) |
| Performance | [Benchmarking](benchmarking.md), [recorded research](../research/README.md) |
