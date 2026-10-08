# HiveKit

HiveKit is the developer toolkit for the [Necter Network](https://necter.network). You write a
module in Rust, Go, TypeScript, JavaScript or Python; HiveKit compiles it to **Hive Bytecode**
(a `.hbc` file); and **NDSR**, the Necter Distributed State Runtime, executes it
deterministically on every node, metering each call with gas and signing a receipt. Validators
agree on results through **SecureWeave** consensus.

| Package | Language | What it is |
|---|---|---|
| [`hivekit-rs`](hivekit-rs) | Rust | `hivekit` crate, `#[hive_export]` macros, `hivec` CLI |
| [`hivekit-go`](hivekit-go) | Go (TinyGo) | `github.com/Necter-Network/hivekit/hivekit-go` and its `hivec` CLI |
| [`hivekit-js`](hivekit-js) | TypeScript / JavaScript | `@necter/hivekit`: AssemblyScript and JavaScript-engine targets, `hivec` CLI |
| [`hivekit`](hivekit) | Python | `necter-hivekit` (import `hivekit`), `hivec` CLI |
| [`hivejs`](hivejs) | TypeScript / JavaScript | `@necter/hivejs`: call deployed modules from Node and browsers |
| [`hivekit-core`](hivekit-core) | Rust | shared core: manifests, canonical JSON, Keccak-256 addresses, `.hbc` packaging |
| [`docs/HBC_SPEC.md`](docs/HBC_SPEC.md) | | the Hive Bytecode format and guest ABI (normative) |
| [`templates`](templates) | | minimal starter projects, one per language |

Full documentation: **https://necter.network/docs**

## Install

One command per language. It installs the `ndsr` runtime (and, for Rust and Go, a prebuilt
`hivec`) into `~/.necter/bin`, verifies every download against the release's SHA-256 checksums,
checks the language toolchain and prints the next steps. No `sudo`.

```sh
curl -fsSL https://necter.network/install.sh | sh -s -- rust
curl -fsSL https://necter.network/install.sh | sh -s -- go
curl -fsSL https://necter.network/install.sh | sh -s -- typescript
curl -fsSL https://necter.network/install.sh | sh -s -- javascript
curl -fsSL https://necter.network/install.sh | sh -s -- python
curl -fsSL https://necter.network/install.sh | sh -s -- miner     # necter-miner CLI (servers, Linux, macOS)
curl -fsSL https://necter.network/install.sh | sh -s -- desktop   # Necter Miner desktop app (macOS DMG)
curl -fsSL https://necter.network/install.sh | sh -s -- validator # ndsr + service templates for a node
curl -fsSL https://necter.network/install.sh | sh -s -- all       # every SDK and the miner CLI
```

Options: `--dir DIR` (install prefix, default `~/.necter`), `--version` (installer and tool
versions), `--uninstall`, `--help`. The same script is [`install.sh`](install.sh) in this repository.
Prebuilt binaries cover macOS (arm64, x86_64) and Linux (x86_64, aarch64; static).

The packages are distributed from this repository's
[GitHub releases](https://github.com/Necter-Network/hivekit/releases); publishing to crates.io, npm
and PyPI is planned. Packages called `hivekit` or `hivejs` on those registries today belong to
unrelated projects.

| Language | Add the SDK to a project |
|---|---|
| Rust | `hivekit = { git = "https://github.com/Necter-Network/hivekit", tag = "v1.0.0" }` |
| Go | `go get github.com/Necter-Network/hivekit/hivekit-go@v1.0.0` |
| TypeScript / JavaScript | `npm install --save-dev https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter-hivekit-1.0.0.tgz` |
| Frontend client | `npm install https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter-hivejs-1.0.0.tgz` |
| Python | `pip install https://github.com/Necter-Network/hivekit/releases/download/v1.0.0/necter_hivekit-1.0.0-py3-none-any.whl` |

## Quickstart

After installing, `necter-init <language> <dir>` creates a minimal project from
[`templates/`](templates). Every language ends the same way: a `.hbc` that `ndsr run` executes
exactly as a network node would.

**Rust**

```sh
necter-init rust my_module && cd my_module
hivec build                                          # -> dist/my_module.hbc
ndsr run dist/my_module.hbc addNumbers --input '{"a":2,"b":3}'
```

**Go** (needs TinyGo)

```sh
necter-init go greeter && cd greeter
go mod tidy
hivec build .                                        # -> dist/greeter.hbc
ndsr run dist/greeter.hbc addNumbers --input '{"a":2,"b":3}'
```

**TypeScript** (AssemblyScript target)

```sh
necter-init typescript counter_ts && cd counter_ts
npm install
npx hivec build counter.ts                           # -> dist/counter.hbc
ndsr run dist/counter.hbc increment --input 5
```

**JavaScript**

```sh
necter-init javascript counter_js && cd counter_js
npm install
npx hivec build counter.js                           # -> dist/counter.hbc
ndsr run dist/counter.hbc increment --input '{"by":2}' --gas 10000000
```

**Python** (3.10+)

```sh
necter-init python counter_py && cd counter_py
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt
hivec build counter.py                               # -> dist/counter.hbc
ndsr run dist/counter.hbc increment --input '{"by":2}' --gas 10000000
```

Modules built with the JavaScript or Python engines need more gas than `ndsr run`'s default of
1,000,000 per call.

Then deploy to the testnet: https://necter.network/docs/deploy/overview/

## The `hivec` commands

Each SDK ships a `hivec` with the same verbs: `build`, `inspect`, `functions` and `run`
(`run` executes on `ndsr`, found through `$NDSR_BIN` or `PATH`). The installer puts the Rust
and Go CLIs in `~/.necter/bin` as `hivec-rs` and `hivec-go`, and points `hivec` at the language
you installed. The TypeScript/JavaScript CLI comes with `@necter/hivekit` (`npx hivec`) and the
Python CLI with `necter-hivekit` (inside your virtual environment).

## Building from source

| Package | Build and test |
|---|---|
| `hivekit-rs` | `cargo test` (integration tests run when `ndsr` is on `PATH`) |
| `hivekit-core` | `cargo test` |
| `hivekit-go` | `go test ./...` (TinyGo end-to-end tests run when `tinygo` and `ndsr` are found) |
| `hivekit-js` | `npm ci && npm run build && npm test` |
| `hivejs` | `npm ci && npm run build && npm test` |
| `hivekit` | `pip install -e '.[test]' && pytest` |

The JavaScript and Python packages include prebuilt, byte-reproducible runtimes
(`hivekit-js/runtime/`, `hivekit/hivekit/runtime/`); `runtime-js/build.sh` and
`runtime-py/build.sh` rebuild them.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities privately as described in
[SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
