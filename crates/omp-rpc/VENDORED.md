# Vendored: omp-rpc

Official Rust client for the omp RPC protocol, copied unmodified (except for removing the
standalone `[workspace]` table from `Cargo.toml`) from
https://github.com/can1357/oh-my-pi/tree/v18.12.3/sdk/rust/omp-rpc (MIT). `LICENSE` is the
upstream repository license at that tag.

`src/wire.rs` is generated upstream from the RPC wire schema. To update, copy the directory from
the omp tag matching the minimum supported omp version and re-apply the Cargo.toml change.
