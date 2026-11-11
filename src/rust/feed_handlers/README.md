# FeedHandlers

FeedHandlers owns OKX and synthetic connectivity, raw market-data SBE encoding, and the RSocket API on port 10001. The RSocket API serves raw market-data streams and instrument-list request-response queries. Price messages contain symbol, price, and exchange timestamp; rolling volatility is derived by Insights. HTTP on port 3001 is reserved for health, metrics, and internal operational endpoints. Run it with `cargo run --manifest-path src/rust/feed_handlers/Cargo.toml` or through `./run-backend-services.sh`.

SBE schema and generated Rust codecs live under `src/sbe`. Benchmarks can be run from this crate with `cargo bench --bench sbe_encoding` and `cargo bench --bench order_book`.
