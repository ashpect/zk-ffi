# zk-ffi

C-compatible FFI backends for the [Verity SDK](https://github.com/ashpect/verity). Each backend lives in its own crate under `backends/` and implements the Verity FFI contract.

## Backends

| Backend | Crate | Prefix | Status |
|---------|-------|--------|--------|
| [Barretenberg](backends/barretenberg/) | `barretenberg-ffi` | `bb_*` | Shipping |

## Structure

```
zk-ffi/
├── Cargo.toml              Workspace root
├── CONTRIBUTING.md          How to add a new backend
└── backends/
    ├── barretenberg/        Barretenberg UltraHonk (bb_*)
    │   ├── Cargo.toml
    │   └── src/lib.rs
    └── your-backend/       
```

## Building

```bash
# Build all backends
cargo build --release

# Build a specific backend
cargo build --release -p barretenberg-ffi

# Build for iOS
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cargo build --release --target aarch64-apple-ios -p barretenberg-ffi
cargo build --release --target aarch64-apple-ios-sim -p barretenberg-ffi
```

## Adding a Backend

See [CONTRIBUTING.md](CONTRIBUTING.md) for a step-by-step guide.
