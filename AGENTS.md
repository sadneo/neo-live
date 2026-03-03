# AGENTS.md - neo-live Development Guide

## Build, Lint, and Test Commands

### Building
```bash
cargo build              # Debug build
cargo build --release     # Release build
```

### Running
```bash
# Run server on local interface
cargo run -- serve

# Run server on LAN interface
cargo run -- serve --host-mode lan

# Connect to server
cargo run -- connect --address 127.0.0.1
```

### Testing
```bash
cargo test               # Run all tests
cargo test <test_name>    # Run a single test (e.g., cargo test serve_basic)
cargo test --release     # Run tests in release mode
```

### Linting
```bash
cargo clippy             # Run clippy for linting suggestions
cargo clippy -- -D warnings  # Treat warnings as errors
cargo fmt                # Format code
cargo fmt -- --check     # Check formatting without modifying
```

---

## Code Style Guidelines

### Imports

**Order imports** (per Rust standard):
1. Standard library (`std::`, `core::`)
2. External crates (`tokio::`, `yrs::`, etc.)
3. Local crate (`crate::`, `super::`)

**Group with blank lines** between groups:
```rust
use std::net::SocketAddrV4;
use std::sync::Arc;

use log::{error, info, trace};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use yrs::updates::decoder::Decode;
use yrs::{Doc, Transact, Update};

use crate::protocol::{self, CrdtUpdate, FrameReader};
```

### Formatting

- Use `rustfmt` - run `cargo fmt` before committing
- For `tokio::select!` macros, rustfmt handles indentation correctly
- Max line length: 100 characters (default)
- Use 4 spaces for indentation

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Modules | `snake_case` | `client.rs`, `protocol.rs` |
| Structs | `PascalCase` | `ClientPool`, `IncomingMessage` |
| Functions | `snake_case` | `run_client`, `handle_initial_sync` |
| Variables | `snake_case` | `read_socket`, `stream_rx` |
| Constants | `SCREAMING_SNAKE_CASE` | `CHANNEL_SIZE` |
| Enums | `PascalCase` | `HostMode` (from clap) |

### Types

- Use explicit types for public function parameters and return values
- Prefer `&str` over `&String` where possible
- Use `Arc<T>` for shared ownership across async tasks
- Use `tokio::sync::mpsc` for channel-based communication

### Async Patterns

- Use `tokio::spawn` to spawn background tasks
- Use `tokio::select!` for concurrent operations
- Always handle channel `recv()` results - don't assume messages arrive
- Use `Arc<RwLock<T>>` for shared mutable state across tasks

### Error Handling

**Do not use `unwrap()` or `expect()` in production code.**
```rust
// BAD
let framed = protocol::encode_frame(&msg).unwrap();

// GOOD - use match or if let
let Some(framed) = protocol::encode_frame(&msg) else {
    error!("Failed to encode message");
    continue;
};

// GOOD - for expected errors
match rmp_serde::from_slice::<CrdtUpdate>(&msg_bytes) {
    Ok(update) => { /* handle */ }
    Err(e) => {
        error!("Failed to deserialize CrdtUpdate: {}", e);
        continue;
    }
}
```

- Log errors at appropriate levels: `error!` for failures, `trace!` for debug info
- Use `?` operator for propagating errors in fallible functions
- Handle all `Result` returns from `apply_update`, `encode_diff_v1`, etc.

### yrs/CRDT Specific

- Use `doc.transact()` for read operations
- Use `doc.transact_mut()` for write operations
- Remember: read transactions cannot be used after a write transaction
- Always encode diffs against the correct state vector, not empty/default

```rust
// GOOD - encode diff against state vector
let sv = txn.state_vector().encode_v1();
let update = txn.encode_diff_v1(&StateVector::decode_v1(&sv)?);

// BAD - this sends entire document every time
let update = txn.encode_diff_v1(&yrs::StateVector::default());
```

### Project Structure

```
src/
├── lib.rs       # Public exports
├── main.rs      # CLI entry point
├── client.rs    # Client connection logic
├── server.rs    # Server relay logic
└── protocol.rs  # Serialization, frames, types
```

### Logging

- Use `log` crate with `env_logger`
- Levels: `error!` > `warn!` > `info!` > `debug!` > `trace!`
- Include context in log messages:
```rust
error!("Failed to deserialize CrdtUpdate: {}", e);
info!("Received initial sync request for buffer: {}", sync.buffer);
```

### Known Issues / TODOs

- Tests in `tests/` need updating for CRDT protocol
- Server's `Doc` cannot be spawned in `tokio::task::spawn` due to internal observer callbacks (yrs limitation)
- Cursor position is currently hardcoded to (0, 0) - needs fixing
