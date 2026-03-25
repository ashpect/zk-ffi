# Adding a New Backend

This guide walks through adding a new ZK proving backend to the Verity SDK.

## Overview

Each backend is a Rust `staticlib` crate that exports C-compatible functions with a unique prefix (e.g., `h2_` for Halo2). The Verity SDK's vtable dispatcher routes unified `verity_*` calls to the correct backend at runtime.

**Your PR touches two repos:**
1. **This repo (zk-ffi):** Add your backend crate under `backends/`
2. **SDK repo (verity):** Add a vtable registration file + enum value

## Step 1: Create Your Backend Crate

```bash
mkdir -p backends/halo2/src
```

### `backends/halo2/Cargo.toml`

```toml
[package]
name = "halo2-ffi"
version = "0.1.0"
edition = "2021"
description = "C-compatible FFI bindings for the Halo2 backend"

[lib]
crate-type = ["staticlib"]

[dependencies]
# Your proving library here
anyhow = "1"
serde_json = "1"
toml = "0.8"
```

### `backends/halo2/src/lib.rs`

Implement these 16 functions with your prefix (`h2_`):

```rust
// Required — all backends must implement these:

#[no_mangle] pub unsafe extern "C" fn h2_prepare(circuit_path, **out_prover, **out_verifier) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_load_prover(path, **out) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_load_verifier(path, **out) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_load_prover_bytes(ptr, len, **out) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_load_verifier_bytes(ptr, len, **out) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_save_prover(prover, path) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_save_verifier(verifier, path) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_serialize_prover(prover, *out_buf) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_serialize_verifier(verifier, *out_buf) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_prove_toml(prover, toml_path, *out_proof) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_prove_json(prover, inputs_json, *out_proof) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_verify(verifier, proof_ptr, proof_len) -> c_int;
#[no_mangle] pub unsafe extern "C" fn h2_free_prover(prover);
#[no_mangle] pub unsafe extern "C" fn h2_free_verifier(verifier);
#[no_mangle] pub unsafe extern "C" fn h2_free_buf(buf);
```

Use `backends/barretenberg/src/lib.rs` as a reference implementation.

### Key rules

- **Buffer type:** Use a `#[repr(C)]` struct with `{ ptr: *mut u8, len: usize, cap: usize }`. Must be layout-compatible with `VerityBuf`.
- **Error codes:** Return standard Verity error codes (0=success, 1=invalid input, 2=scheme read error, 4=proof error, etc.).
- **Panic safety:** Wrap every FFI function in `catch_panic()` to prevent Rust panics from unwinding across the FFI boundary.
- **Handle ownership:** `prepare` / `load_*` create handles via `Box::into_raw`. `free_*` reclaim via `Box::from_raw`. Handles must be freed exactly once.
- **Prove clones:** If your prover consumes `self` on prove, clone it inside the FFI function so the handle stays reusable.

## Step 2: Register in the Workspace

Add your crate to the root `Cargo.toml`:

```toml
[workspace]
members = [
    "backends/barretenberg",
    "backends/halo2",          # ← add this
]
```

## Step 3: Build and Verify

```bash
cargo build --release -p halo2-ffi
cargo build --release --target aarch64-apple-ios -p halo2-ffi
cargo build --release --target aarch64-apple-ios-sim -p halo2-ffi
```

## Step 4: SDK Integration (separate PR to verity repo)

### Add vtable registration file

Create `Sources/VerityDispatch/h2_backend.c` in the SDK repo. Copy from `pk_backend.c` or `bb_backend.c` and replace the prefix:

```c
#include "verity_backend.h"

// Extern declarations for your h2_* symbols
extern int h2_prepare(const char *, void **, void **);
extern int h2_prove_toml(const void *, const char *, RawBuf *);
// ... all 16 functions

// Thin wrappers (cast typed pointers to void*)
static int w_h2_prepare(const char *path, void **p, void **v) {
    return h2_prepare(path, p, v);
}
// ... wrappers for all functions

// Vtable
static const VerityVtable h2_vtable = {
    .init     = h2_init,
    .prepare  = w_h2_prepare,
    // ... all 16 slots
};

// Auto-register at library load time
__attribute__((constructor))
static void h2_register(void) {
    verity_register_backend(VERITY_BACKEND_HALO2, &h2_vtable);
}
```

### Add enum value

In `Sources/VerityDispatch/include/verity_ffi.h`:

```c
typedef enum {
    VERITY_BACKEND_PROVEKIT       = 0,
    VERITY_BACKEND_BARRETENBERG   = 1,
    VERITY_BACKEND_HALO2          = 2,   // ← add this
} VerityBackend;
```

### Update build script

Add your static lib to the xcframework merge step in `scripts/build-xcframework.sh`.

### What you DON'T need to change

- **Swift SDK:** Zero files. No switch cases, no new code.
- **Kotlin SDK:** Zero files.
- **Other backends:** Zero files.
- **Dispatcher core (verity_dispatch.c):** Zero files.

## Checklist

- [ ] Backend crate builds for host, iOS device, and iOS simulator
- [ ] All 16 functions implemented with correct error codes
- [ ] All FFI functions wrapped in `catch_panic()`
- [ ] Handles freed correctly (no leaks, no double-free)
- [ ] `cargo clippy` clean
- [ ] Vtable registration file created in SDK repo
- [ ] Enum value added to `verity_ffi.h`
- [ ] Build script updated
- [ ] Tests pass on iOS Simulator
