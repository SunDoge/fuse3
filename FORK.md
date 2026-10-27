# MoriFS correctness branch

This branch carries a focused patch set for MoriFS against upstream
`5f9720db4efe9a84b30633a8eacab7dd9cc2b3ff`. It is not a claim that MoriFS already
implements a complete filesystem. Keep the upstream package name/version and pin
consumers to a reviewed Git revision.

## Imported upstream work

[PR #139](https://github.com/Sherlock-Holo/fuse3/pull/139), through
`fd12cfc`, was cherry-picked with original authors and source hashes preserved:

- retain inode numbers until lookup references are forgotten;
- preserve identities on rename/exchange and never recycle the root;
- pass `(inode, nlookup)` through raw `batch_forget` (a breaking API change);
- count only readdirplus entries that are actually packed and refund unconsumed
  or cancelled entries;
- include directory-entry alignment padding in reply-size checks.

The baseline already includes upstream [#131](https://github.com/Sherlock-Holo/fuse3/pull/131),
which fixes the positive errno in xattr size replies, and [#136](https://github.com/Sherlock-Holo/fuse3/pull/136),
which makes background request limits configurable. Neither was reapplied.

## Raw notification access

`raw::Filesystem::ready(Notify)` is an additive callback with a default no-op.
It runs once after a successful kernel INIT reply and before normal request
dispatch. A raw filesystem can retain the notifier and invalidate pages or
entries after an out-of-band change, without requiring a `poll` request.

Store the notifier and return promptly. Do not perform I/O against the mount in
this callback. Send invalidations later from a separate task, outside filesystem
locks and handlers that might be required by kernel writeback. The path API is
unchanged; raw notification access does not solve the path bridge's remaining
hard-link or reply-delivery limitations documented in README.md.

## Validation

Locally verified on Linux x86_64, Rust 1.99.1, on 2026-10-27:

- 12 library tests passed with Tokio, and 12 with async-io;
- both feature configurations passed all-target clippy with warnings denied;
- documentation and the upstream examples compiled (examples retain existing
  dead-code/unused-mut warnings);
- a real unprivileged kernel mount passed: warm a page cache, change the backend,
  verify the old cached bytes, invalidate the inode, read updated bytes through
  the same descriptor; query an xattr's size and data; list 400 directory entries;
  verify the mount remains usable and unmount explicitly.

```sh
cargo test --lib --features tokio-runtime,file-lock,unprivileged
cargo test --lib --features async-io-runtime,file-lock,unprivileged
cargo clippy --all-targets --features tokio-runtime,file-lock,unprivileged -- -D warnings
cargo clippy --all-targets --features async-io-runtime,file-lock,unprivileged -- -D warnings
cargo doc --no-deps --features tokio-runtime,file-lock,unprivileged
cargo check -p examples
cargo fmt --all --check
# Requires Linux /dev/fuse and fusermount3. Ignored in an ordinary test run;
# explicitly requesting it fails if mounting is unavailable, rather than skipping.
cargo test --test mount_regression --features tokio-runtime,file-lock,unprivileged -- --ignored --nocapture
```

Linux CI runs both library suites. The real-mount regression is opt-in because
normal CI runners may not expose /dev/fuse. macOS/FreeBSD execution and power-loss
durability were not tested here. The probe is not a writable filesystem test and
does not establish fsync, lock, crash-recovery or application-build correctness;
MoriFS must implement and validate those semantics separately.
