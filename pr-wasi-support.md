## Summary

Move `assert_fs` from `[dependencies]` to `[dev-dependencies]` in `Cargo.toml`.

## Problem

`assert_fs` is a filesystem testing library that is only used in `#[cfg(test)]` blocks (specifically in `src/sed/in_place.rs` and `tests/by-util/test_sed.rs`). However, it is listed as a regular dependency, which means it is compiled for all targets including `wasm32-wasi`, where it fails because it depends on `std::os::unix`.

## Fix

Move `assert_fs = { workspace = true }` from `[dependencies]` to `[dev-dependencies]`. This is a one-line change that has no effect on the built binary — `assert_fs` was never used in non-test code.
