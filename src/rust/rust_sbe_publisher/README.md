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
