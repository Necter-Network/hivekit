# Security policy

## Reporting a vulnerability

Please report security issues privately by email to **security@necter.network**. Do not open a
public issue or pull request for a vulnerability.

Include what you can of the following:

- the affected package (`hivekit-rs`, `hivekit-go`, `@necter/hivekit`, `@necter/hivejs`,
  `necter-hivekit`, `hivekit-core`), release binary or `install.sh`, and its version;
- a description of the issue and its impact;
- steps or a minimal module that reproduces it.

We aim to acknowledge reports within three working days and to agree on a disclosure date with
you once a fix is ready.

## Scope

- The SDKs, the `hivec` CLIs and the `.hbc` packaging in this repository.
- The release artifacts and `install.sh` (including checksum verification).
- Differences between what an SDK produces and [`docs/HBC_SPEC.md`](docs/HBC_SPEC.md) that could
  make a module behave differently from what its author intended.

Issues in the network itself (validators, SecureWeave consensus, rewards) are also welcome at the
same address.

## Supported versions

Security fixes are made for the latest release.
