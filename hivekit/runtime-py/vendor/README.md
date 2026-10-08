# Vendored crates

`rustpython-derive-impl` is version 0.6.0 from crates.io (MIT) with one change:
the code it generates no longer depends on `HashMap` iteration order, which is
random per compiler process. Property (getset) registrations and frozen
modules are emitted sorted by name. Without this, two builds of the runtime
from the same source differ byte-for-byte. It is wired in with
`[patch.crates-io]` in `../Cargo.toml`.
