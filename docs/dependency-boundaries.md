# Dependency Boundaries

The Rust SDK should keep dependencies flowing in one direction. Cycles between
crates are not allowed by Cargo, and cycles between modules should be treated as
a design smell even when the compiler can technically build them.

## Desired Crate Direction

The intended dependency graph is:

```text
yorkie-core
  -> standard library and small pure-Rust utilities only

yorkie-protocol
  -> yorkie-core
  -> generated protocol types and protocol-only helpers

yorkie-client
  -> yorkie-core
  -> yorkie-protocol

yorkie
  -> yorkie-core
  -> yorkie-protocol
  -> yorkie-client
```

The `yorkie` crate is a facade. It should re-export user-facing APIs and avoid
owning behavior.

## Crate Responsibilities

| Crate | Owns | Must not depend on |
| --- | --- | --- |
| `yorkie-core` | Document model, CRDTs, changes, operations, logical clocks, JSON-like values. | `yorkie-client`, network runtimes, generated transport clients. |
| `yorkie-protocol` | Generated protobuf types, wire-level constants, protocol-only helpers. | `yorkie-client`, facade APIs. |
| `yorkie-client` | Client lifecycle, attach/detach, sync, watch streams, channels, retries. | The `yorkie` facade crate. |
| `yorkie` | Public SDK entrypoint and re-exports. | Internal implementation details beyond re-export needs. |

## Rules

- Keep `yorkie-core` transport-free. It should not know about HTTP, Connect,
  gRPC, Tokio tasks, watch streams, or API keys.
- Keep `yorkie-client` as the integration layer. It may combine core document
  types with protocol wire types.
- Keep `yorkie` thin. If a function needs real behavior, it probably belongs in
  `yorkie-core` or `yorkie-client`.
- Prefer private modules by default. Use `pub(crate)` for cross-module internals
  and `pub` only for intentional public API.
- Do not make two crates depend on each other. If two crates need each other's
  types, move the shared concept downward or introduce a narrow adapter layer.
- Do not add a new crate just to break a small local cycle. First try to move
  the type to the crate that naturally owns the concept.

## Converter Placement

Converters are the most likely place to create dependency pressure because they
touch both SDK model types and wire types.

Preferred initial placement:

```text
yorkie-client
  -> core model types
  -> protocol wire types
  -> conversion code
```

This keeps `yorkie-core` independent from wire format and keeps
`yorkie-protocol` independent from high-level SDK behavior.

If conversion code grows large, consider a dedicated adapter crate with one-way
dependencies:

```text
yorkie-converter
  -> yorkie-core
  -> yorkie-protocol

yorkie-client
  -> yorkie-converter
```

Do not make `yorkie-core` depend on `yorkie-protocol` just to serialize itself.
Serialization is a boundary concern, not a core document concern.

## Module-Level Guidance

Inside a crate, modules should also flow from high-level orchestration to lower
level data structures:

```text
client orchestration -> attachment/sync/watch helpers -> protocol adapter
document API -> change/context -> operation -> crdt/time/json data structures
```

Current `yorkie-client` module layout follows the same split:

| Module | Role |
| --- | --- |
| `client.rs` | Client orchestration, lifecycle state, and public client methods. |
| `options.rs` | Client, attach, detach, channel, and sync mode option types. |
| `attachment.rs` | Per-resource attachment metadata and sync interval helpers. |
| `transport.rs` | Request/response DTOs and transport traits for RPC boundaries. |
| `protocol.rs` | Conversion between client transport DTOs and generated protobuf request/response types. |
| `error.rs` | Client-layer error and result types. |
| `lib.rs` | Crate entrypoint and public re-exports only. |

Avoid sibling modules that call into each other in both directions. If that
happens, extract the shared type or function into a lower-level module.

Good signs:

- Types can be explained without referencing their callers.
- Lower-level modules have fewer dependencies than higher-level modules.
- Tests can exercise lower-level modules without constructing clients or
  network state.

Warning signs:

- A core type imports a client type.
- A protocol type imports a facade type.
- A module exposes `pub` items only because another sibling module needs them.
- A converter is split across two crates and each side needs callbacks into the
  other.

## Review Checklist

Before adding a dependency:

- Is the dependency direction still one-way?
- Is this dependency needed for ownership of the concept, or only for
  convenience?
- Could the shared type live in a lower-level module or crate?
- Is this dependency visible in public API?
- Would this make future FFI or WASM bindings harder?
