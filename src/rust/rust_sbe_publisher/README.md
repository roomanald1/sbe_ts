java -Dsbe.target.language=rust \
     -Dsbe.output.dir=sbe_out \
     -jar sbe.jar \
     sbe_schema.xml

## Run from VS Code

Open the repository root (`sbe_ts`) in VS Code. The workspace links the Rust
crate for rust-analyzer, so the Run/Debug CodeLens appears above `main`.

- Use **Run** above `main` to start the publisher.
- Use **Debug** above `main`, or select **Debug Rust publisher** in Run and
  Debug, to launch it under CodeLLDB.

Install the recommended `rust-analyzer` and `CodeLLDB` extensions when prompted.
The REST API listens on port 3000 and the RSocket WebSocket server uses the
`PORT` environment variable (10000 by default).

You can also run it from the integrated terminal:

```sh
cargo run --manifest-path src/rust/rust_sbe_publisher/Cargo.toml
```

## Measure SBE encoding cost

Run the release-mode microbenchmark from the crate directory:

```sh
cargo bench --bench sbe_encoding
```

It reports encoding plus RSocket payload construction, and a scratch-buffer
reuse lower bound. The scratch-buffer case is only a comparison: reusing a
buffer is not safe for payloads that may still be in flight. This benchmark
also times synthetic symbol/price generation and compares per-client encoding
with encoding once and cloning immutable `Bytes` at fan-outs of 1, 4, and 16.
It also measures a Tokio broadcast channel delivering shared bytes and
constructing payloads for 200 and 400 subscribers. These cases are an
in-process CPU benchmark only; they do not measure RSocket/WebSocket framing,
socket scheduling, backpressure, or network capacity.

For profiling the complete service on Linux, build with symbols and run the
server under `perf` or `heaptrack` while a client consumes the feed:

```sh
cargo build --release
perf stat -d -- target/release/rust_sbe_publisher
heaptrack target/release/rust_sbe_publisher
```

Use the same feed, client count, and test duration for before/after
comparisons. `perf record -g -- target/release/rust_sbe_publisher` can capture
a CPU profile for later inspection with `perf report`.

## RSocket subscriber load test

With the publisher already running locally, run the headless OKX load test from
the TypeScript workspace. It ramps through 1, 50, 100, 200, and 400 subscribers,
measuring each level for 10 seconds:

```sh
cd src/typescript/workspace
node apps/react-app/scripts/rsocket-load-test.cjs --server-pid=<publisher-pid>
```

The test reports received messages, per-client distribution, payload bandwidth,
publisher CPU/RSS, and load-generator CPU. Override `--duration-ms=30000` for
longer phases, `--levels=1,50,100` to limit subscriber counts, or
`--url=ws://127.0.0.1:10000` to target another local instance. This test uses
the live OKX feed and the shared-payload production path; requests and network
conditions can vary between runs.

The OKX WebSocket subscription reconnects after transport errors or unexpected
stream termination. Retry delays use exponential backoff from 1 to 30 seconds
and reset after a connection has remained up for at least 60 seconds.
