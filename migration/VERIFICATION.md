# Local verification

Upstream revision: `e09052d94d64cdcbc082357cd5c2c860e4a19b40`.

Verified on October 12, 2026, in Ubuntu-24.04 WSL with Rust 1.98.1:

- `cargo build --locked -p example-mud --bin example-mud`: passed.
- `cargo fmt --all`: completed.
- `cargo clippy --locked -p example-mud --bin example-mud -- -D warnings`: passed.
- `cargo test --locked -p example-mud --lib --test savsoul --test scenarios`:
  46 passed, zero failed (5 seed tests, 1 Savsoul scenario, 40 upstream scenarios).
- `python migration/validate_world.py`: passed; all 20 Savsoul rooms reachable
  from Haven and returnable; all exits resolve, no duplicate IDs, valid directions,
  reciprocal closed door definitions.
- Windows TCP connection to `127.0.0.1:4000`: received Grim's account login prompt.

The Savsoul gameplay scenario checks blocked passage before opening, arrival in
Royal Oak, medical sign text, medical table inspection and pickup rejection,
the entire faded trail, bar second-floor routes, return to Haven, and door closure
and reopening from Royal Oak. Login testing created no real persisted accounts.
