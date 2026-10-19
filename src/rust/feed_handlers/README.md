# FeedHandlers

FeedHandlers owns OKX and synthetic connectivity, SBE encoding, the instrument API on port 3001, and the RSocket stream on port 10001. Run it with `cargo run --manifest-path src/rust/feed_handlers/Cargo.toml` or through `./run-backend-services.sh`.

SBE schema and generated Rust codecs live under `src/sbe`. Benchmarks can be run from this crate with `cargo bench --bench sbe_encoding` and `cargo bench --bench order_book`.
