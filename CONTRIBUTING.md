# Contributing to HiveKit

Thanks for helping improve HiveKit. Bug reports, documentation fixes and pull requests are welcome.

## Before you start

- For anything larger than a small fix, open an issue first so the approach can be agreed.
- Security issues: do not open an issue; see [SECURITY.md](SECURITY.md).

## Ground rules

- **The spec is normative.** Every SDK must produce artifacts that conform to
  [`docs/HBC_SPEC.md`](docs/HBC_SPEC.md) and reproduce every value in
  [`docs/test-vectors.json`](docs/test-vectors.json). The copies of that file under each SDK's
  test directory must stay byte-identical to it.
- **Determinism.** Manifests must not contain timestamps or extra keys, `functions` are sorted
  and unique, and builds should be reproducible. The prebuilt runtimes in `hivekit-js/runtime/`
  and `hivekit/hivekit/runtime/` are rebuilt only with their `build.sh` scripts, and their
  SHA-256 is recorded in the package README.
- **Keep the SDKs in step.** A change to the ABI, manifest or host API should land in all
  SDKs together.
- Add tests with every change. End-to-end tests run modules under `ndsr`; install it with
  `curl -fsSL https://necter.network/install.sh | sh -s -- rust` (or set `NDSR_BIN`).

## Checks

| Package | Commands |
|---|---|
| `hivekit-core` | `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` |
| `hivekit-rs` | `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` |
| `hivekit-go` | `gofmt -l . && go vet ./... && go test ./...` |
| `hivekit-js` | `npm ci && npm run typecheck && npm run lint && npm run build && npm test` |
| `hivejs` | `npm ci && npm run typecheck && npm run lint && npm run build && npm test` |
| `hivekit` | `pip install -e '.[test]' && pytest` |
| `install.sh` | `shellcheck install.sh` |

## Pull requests

- Keep each pull request focused on one change and describe what it changes and why.
- Use plain, descriptive commit messages.
- By submitting a contribution you agree that it is licensed under the Apache License 2.0
  (see [LICENSE](LICENSE)).
