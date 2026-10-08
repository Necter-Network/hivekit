#!/bin/sh
# Necter developer toolchain installer.
#
#   curl -fsSL https://necter.network/install.sh | sh -s -- <language>
#
# Installs the NDSR runtime (`ndsr`), the HiveKit `hivec` CLIs, the `necter-init` project
# generator and, on request, the `necter-miner` CLI into ~/.necter/bin. Every download is
# verified against the release's SHA256SUMS. Never needs sudo. Safe to run again.
#
# Source: https://github.com/Necter-Network/hivekit/blob/main/install.sh

set -eu

INSTALLER_VERSION="1.0.0"
HIVEKIT_VERSION="1.0.0"
MINER_VERSION="1.0.0"
MINER_TAG="v1.0.0-testnet"

HIVEKIT_BASE="${NECTER_HIVEKIT_BASE:-https://github.com/Necter-Network/hivekit/releases/download/v${HIVEKIT_VERSION}}"
MINER_BASE="${NECTER_MINER_BASE:-https://github.com/Necter-Network/necter-releases/releases/download/${MINER_TAG}}"
REPO_URL="https://github.com/Necter-Network/hivekit"
DOCS_URL="https://necter.network/docs"

NECTER_DIR="${NECTER_DIR:-$HOME/.necter}"
LANGS=""
ACTION="install"
TMP=""

# ---------------------------------------------------------------- output helpers

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  B="$(printf '\033[1m')"; G="$(printf '\033[32m')"; Y="$(printf '\033[33m')"
  R="$(printf '\033[31m')"; D="$(printf '\033[2m')"; N="$(printf '\033[0m')"
else
  B=""; G=""; Y=""; R=""; D=""; N=""
fi

say()  { printf '%s\n' "$*"; }
info() { printf '%s==>%s %s\n' "$B" "$N" "$*"; }
ok()   { printf '  %sok%s    %s\n' "$G" "$N" "$*"; }
warn() { printf '  %swarn%s  %s\n' "$Y" "$N" "$*"; }
die()  { printf '%serror:%s %s\n' "$R" "$N" "$*" >&2; exit 1; }

usage() {
  cat <<EOF
Necter developer toolchain installer ${INSTALLER_VERSION}

Usage:
  curl -fsSL https://necter.network/install.sh | sh -s -- [options] <component>...
  sh install.sh [options] <component>...

Components:
  rust          ndsr + hivec for Rust (needs rustup and the wasm32-unknown-unknown target)
  go            ndsr + hivec for Go (needs Go 1.21+ and TinyGo)
  typescript    ndsr; the TypeScript SDK is an npm package added per project (needs Node 18+)
  javascript    same as typescript
  python        ndsr; the Python SDK is a wheel added per project (needs Python 3.10+)
  miner         necter-miner, the command-line miner (servers, Linux, macOS)
  desktop       the Necter Miner desktop app (macOS): downloads, verifies and opens the .dmg
  validator     ndsr plus a service template (systemd / launchd) and an example config for
                running a node; nothing is started
  all           rust, go, typescript, javascript, python and miner

Options:
  --dir DIR     install under DIR instead of ~/.necter (binaries go to DIR/bin)
  --uninstall   remove everything this script installed under the install dir
  --version     print the installer and release versions, and what is installed
  -h, --help    show this help

Every SDK component also installs 'necter-init', which creates a starter project:
  necter-init <rust|go|typescript|javascript|python> <directory>

Docs: ${DOCS_URL}/build/install/
EOF
}

# ---------------------------------------------------------------- arguments

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --version|-V) ACTION="version" ;;
    --uninstall) ACTION="uninstall" ;;
    --dir) [ $# -ge 2 ] || die "--dir needs a directory"; NECTER_DIR="$2"; shift ;;
    --dir=*) NECTER_DIR="${1#--dir=}" ;;
    rust|go|python|miner|desktop|validator) LANGS="$LANGS $1" ;;
    typescript|ts) LANGS="$LANGS typescript" ;;
    javascript|js) LANGS="$LANGS javascript" ;;
    all) LANGS="$LANGS rust go typescript javascript python miner" ;;
    -*) die "unknown option: $1 (see --help)" ;;
    *) die "unknown component: $1 (see --help)" ;;
  esac
  shift
done

case "$NECTER_DIR" in
  /*) ;;
  "") die "--dir is empty" ;;
  *) NECTER_DIR="$(pwd)/$NECTER_DIR" ;;
esac
BIN="$NECTER_DIR/bin"
SHARE="$NECTER_DIR/share/hivekit"
RECEIPTS="$NECTER_DIR/share/receipts"

has() { command -v "$1" >/dev/null 2>&1; }
want() { case " $LANGS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

# ---------------------------------------------------------------- version / uninstall

tool_version() {
  # $1 = binary path, rest = args
  _b="$1"; shift
  if [ -x "$_b" ]; then "$_b" "$@" 2>/dev/null | head -n 1; else say "not installed"; fi
}

if [ "$ACTION" = "version" ]; then
  say "necter install.sh ${INSTALLER_VERSION}"
  say "hivekit release   v${HIVEKIT_VERSION}"
  say "miner release     ${MINER_TAG}"
  say "install dir       ${NECTER_DIR}"
  say "ndsr              $(tool_version "$BIN/ndsr" --version)"
  say "hivec-rs          $(tool_version "$BIN/hivec-rs" --version)"
  say "hivec-go          $(tool_version "$BIN/hivec-go" version)"
  say "necter-miner      $(tool_version "$BIN/necter-miner" --version)"
  exit 0
fi

if [ "$ACTION" = "uninstall" ]; then
  info "Removing the Necter toolchain from $NECTER_DIR"
  for f in ndsr hivec hivec-rs hivec-go necter-init necter-miner; do
    if [ -e "$BIN/$f" ] || [ -L "$BIN/$f" ]; then rm -f "$BIN/$f"; ok "removed $BIN/$f"; fi
  done
  [ -d "$SHARE" ] && rm -rf "$SHARE" && ok "removed $SHARE"
  [ -d "$NECTER_DIR/share/validator" ] && rm -rf "$NECTER_DIR/share/validator" && ok "removed $NECTER_DIR/share/validator"
  [ -d "$RECEIPTS" ] && rm -rf "$RECEIPTS"
  rmdir "$NECTER_DIR/share" "$BIN" "$NECTER_DIR" 2>/dev/null || true
  say ""
  say "Done. Remove the PATH line for $BIN from your shell profile if you added one."
  say "Miner data (keys, wallet, config) is kept; 'necter-miner' stores it outside $NECTER_DIR."
  say "Node data directories you created for ndsr (node.key, state) are not touched."
  exit 0
fi

[ -n "$LANGS" ] || { usage; say ""; die "choose at least one component, e.g.: sh -s -- rust"; }

# ---------------------------------------------------------------- platform

detect_target() {
  _os="$(uname -s)"; _arch="$(uname -m)"
  case "$_os" in
    Darwin)
      # A shell running under Rosetta reports x86_64 on Apple silicon: prefer native arm64.
      if [ "$_arch" = "x86_64" ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = "1" ]; then
        _arch="arm64"
      fi
      case "$_arch" in
        arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
        x86_64) TARGET="x86_64-apple-darwin" ;;
        *) die "unsupported macOS architecture: $_arch" ;;
      esac ;;
    Linux)
      case "$_arch" in
        x86_64|amd64) TARGET="x86_64-unknown-linux-musl" ;;
        aarch64|arm64) TARGET="aarch64-unknown-linux-musl" ;;
        *) die "unsupported Linux architecture: $_arch (prebuilt binaries: x86_64, aarch64)" ;;
      esac ;;
    *) die "unsupported OS: $_os (prebuilt binaries: macOS and Linux; on Windows use WSL)" ;;
  esac
}

# ---------------------------------------------------------------- downloads

download() {
  # $1 = url, $2 = destination file
  if has curl; then
    curl --proto '=https,file' --tlsv1.2 -fsSL --retry 3 -o "$2" "$1" || return 1
  elif has wget; then
    wget -q -O "$2" "$1" || return 1
  else
    die "curl or wget is required"
  fi
}

sha256_of() {
  if has sha256sum; then sha256sum "$1" | awk '{print $1}'
  elif has shasum; then shasum -a 256 "$1" | awk '{print $1}'
  elif has openssl; then openssl dgst -sha256 "$1" | awk '{print $NF}'
  else die "no SHA-256 tool found (need sha256sum, shasum or openssl)"
  fi
}

# fetch_verified BASE ASSET -> $TMP/ASSET, checked against BASE/SHA256SUMS
fetch_verified() {
  _base="$1"; _asset="$2"
  _sums="$TMP/SHA256SUMS.$(printf '%s' "$_base" | cksum | awk '{print $1}')"
  if [ ! -f "$_sums" ]; then
    download "$_base/SHA256SUMS" "$_sums" || die "could not download $_base/SHA256SUMS"
  fi
  _want="$(awk -v f="$_asset" '$2 == f || $2 == "*" f {print $1}' "$_sums" | head -n 1)"
  [ -n "$_want" ] || die "$_asset is not listed in $_base/SHA256SUMS"
  download "$_base/$_asset" "$TMP/$_asset" || die "could not download $_base/$_asset"
  _got="$(sha256_of "$TMP/$_asset")"
  [ "$_got" = "$_want" ] || die "checksum mismatch for $_asset (expected $_want, got $_got)"
  ok "downloaded $_asset (sha256 verified)"
}

installed() { [ -f "$RECEIPTS/$1" ] && [ "$(cat "$RECEIPTS/$1")" = "$2" ]; }
receipt() { mkdir -p "$RECEIPTS"; printf '%s\n' "$2" > "$RECEIPTS/$1"; }

# install_binary NAME BASE VERSION: <name>-<version>-<target>.tar.gz holding <name>
install_binary() {
  _name="$1"; _base="$2"; _ver="$3"
  if installed "$_name" "$_ver-$TARGET" && [ -x "$BIN/$_name" ]; then
    ok "$_name $_ver already installed"
    return 0
  fi
  _asset="$_name-$_ver-$TARGET.tar.gz"
  fetch_verified "$_base" "$_asset"
  mkdir -p "$TMP/x-$_name" "$BIN"
  tar -xzf "$TMP/$_asset" -C "$TMP/x-$_name"
  _src="$(find "$TMP/x-$_name" -type f -name "$_name" | head -n 1)"
  [ -n "$_src" ] || die "$_asset does not contain $_name"
  chmod 0755 "$_src"
  mv -f "$_src" "$BIN/$_name.new" && mv -f "$BIN/$_name.new" "$BIN/$_name"
  if [ "$(uname -s)" = "Darwin" ]; then xattr -d com.apple.quarantine "$BIN/$_name" 2>/dev/null || true; fi
  receipt "$_name" "$_ver-$TARGET"
  ok "installed $BIN/$_name"
}

install_templates() {
  if installed templates "$HIVEKIT_VERSION" && [ -d "$SHARE/templates" ] && [ -x "$BIN/necter-init" ]; then
    ok "project templates $HIVEKIT_VERSION already installed"
    return 0
  fi
  _asset="hivekit-templates-$HIVEKIT_VERSION.tar.gz"
  fetch_verified "$HIVEKIT_BASE" "$_asset"
  rm -rf "$SHARE/templates.new"
  mkdir -p "$SHARE/templates.new" "$BIN"
  tar -xzf "$TMP/$_asset" -C "$SHARE/templates.new"
  [ -d "$SHARE/templates.new/templates" ] || die "$_asset has an unexpected layout"
  rm -rf "$SHARE/templates"
  mv "$SHARE/templates.new/templates" "$SHARE/templates"
  rm -rf "$SHARE/templates.new"
  write_necter_init
  receipt templates "$HIVEKIT_VERSION"
  ok "installed $BIN/necter-init"
}

write_necter_init() {
  cat > "$BIN/necter-init.new" <<EOF
#!/bin/sh
# necter-init: create a HiveKit starter project. Installed by install.sh.
set -eu
TEMPLATES="$SHARE/templates"
EOF
  cat >> "$BIN/necter-init.new" <<'EOF'
usage() {
  echo "usage: necter-init <rust|go|typescript|javascript|python> <directory>"
  echo "Creates a starter HiveKit project in <directory> (which must not exist or be empty)."
}
[ $# -eq 2 ] || { usage >&2; exit 2; }
case "$1" in
  -h|--help) usage; exit 0 ;;
  rust|go|typescript|javascript|python) lang="$1" ;;
  ts) lang=typescript ;;
  js) lang=javascript ;;
  *) usage >&2; exit 2 ;;
esac
dir="$2"
name="$(basename "$dir")"
case "$name" in
  [A-Za-z]*) ;;
  *) echo "necter-init: project name '$name' must start with a letter" >&2; exit 2 ;;
esac
case "$name" in
  *[!A-Za-z0-9_-]*) echo "necter-init: project name '$name' may only use letters, digits, '_' and '-'" >&2; exit 2 ;;
esac
if [ -e "$dir" ] && [ -n "$(ls -A "$dir" 2>/dev/null)" ]; then
  echo "necter-init: $dir already exists and is not empty" >&2; exit 1
fi
[ -d "$TEMPLATES/$lang" ] || { echo "necter-init: template '$lang' is missing; re-run install.sh" >&2; exit 1; }
mkdir -p "$dir"
cp -R "$TEMPLATES/$lang/." "$dir/"
grep -rl "__NAME__" "$dir" 2>/dev/null | while IFS= read -r f; do
  sed "s/__NAME__/$name/g" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
done
echo "Created $lang project in $dir"
EOF
  chmod 0755 "$BIN/necter-init.new"
  mv -f "$BIN/necter-init.new" "$BIN/necter-init"
}

# `hivec` picks the Rust or Go CLI from the project it is pointed at.
write_hivec_dispatcher() {
  cat > "$BIN/hivec.new" <<'EOF'
#!/bin/sh
# hivec: runs hivec-rs or hivec-go depending on the project. Installed by install.sh.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
detect() {
  # $1 = path argument (file or directory); look for go.mod / Cargo.toml upwards.
  p="${1:-.}"
  case "$p" in
    *.go) echo go; return ;;
    *.rs|*Cargo.toml|*.wasm) echo rs; return ;;
  esac
  [ -d "$p" ] || p="$(dirname "$p")"
  d="$(cd "$p" 2>/dev/null && pwd || echo "")"
  while [ -n "$d" ]; do
    [ -f "$d/go.mod" ] && { echo go; return; }
    [ -f "$d/Cargo.toml" ] && { echo rs; return; }
    [ "$d" = "/" ] && break
    d="$(dirname "$d")"
  done
  echo ""
}
first_path() {
  # first non-flag argument after the subcommand
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      -o|--out|-name|--name|-version|--module-version|-description|--description|-tinygo|--example|--functions|--language|--target-dir) shift ;;
      -*) ;;
      *) echo "$1"; return ;;
    esac
    shift
  done
}
cmd="${1:-}"
case "$cmd" in
  --version|-V|version)
    [ -x "$here/hivec-rs" ] && "$here/hivec-rs" --version
    [ -x "$here/hivec-go" ] && "$here/hivec-go" version
    exit 0 ;;
  build|functions|package)
    lang="$(detect "$(first_path "$@")")" ;;
  *) lang="" ;;
esac
if [ -z "$lang" ]; then
  if [ -x "$here/hivec-rs" ]; then lang=rs; else lang=go; fi
fi
if [ -x "$here/hivec-$lang" ]; then exec "$here/hivec-$lang" "$@"; fi
for other in rs go; do
  if [ -x "$here/hivec-$other" ]; then exec "$here/hivec-$other" "$@"; fi
done
echo "hivec: neither hivec-rs nor hivec-go is installed; run install.sh rust or install.sh go" >&2
exit 1
EOF
  chmod 0755 "$BIN/hivec.new"
  mv -f "$BIN/hivec.new" "$BIN/hivec"
}

DMG_ASSET="Necter-Miner-macOS-universal.dmg"
install_desktop() {
  if [ "$(uname -s)" != "Darwin" ]; then
    warn "the desktop app is for macOS; on Linux use the 'miner' component,"
    say "          on Android: $MINER_BASE/Necter-Miner-android.apk"
    return 0
  fi
  fetch_verified "$MINER_BASE" "$DMG_ASSET"
  _dest="$HOME/Downloads"
  [ -d "$_dest" ] || _dest="$NECTER_DIR"
  mv -f "$TMP/$DMG_ASSET" "$_dest/$DMG_ASSET"
  ok "saved $_dest/$DMG_ASSET"
  if [ -z "${NECTER_NO_OPEN:-}" ]; then
    open "$_dest/$DMG_ASSET" && ok "opened the disk image"
  fi
}

VAL="$NECTER_DIR/share/validator"
install_validator_files() {
  mkdir -p "$VAL"
  cat > "$VAL/ndsr.env.example" <<EOF
# ndsr node configuration (environment file). Copy to ndsr.env and edit.
# Every variable matches an 'ndsr serve' flag; see: ndsr serve --help
NDSR_DATA_DIR=/var/lib/ndsr
NDSR_BIND=127.0.0.1:7070
# Required when NDSR_BIND is not a loopback address:
# NDSR_API_TOKEN=<long random string>
# EVM address credited with this node's compute units:
# NDSR_PAYOUT_ADDRESS=0x<your payout address>
# Committee mode (settlement chain = Ethereum Sepolia on the testnet):
# NDSR_CHAIN_RPC=https://<your Sepolia JSON-RPC endpoint>
# NDSR_CHAIN_ID=11155111
# NDSR_HUB_URL=https://testnet-rpc.necter.network
# NDSR_NETWORK=necter-testnet
# NDSR_STATE_SYNC=true
# Public URL other nodes use to reach this one (behind your TLS reverse proxy):
# NDSR_NODE_URL=https://node.example.com
EOF
  cat > "$VAL/ndsr.service" <<EOF
# systemd unit for ndsr. Install (as root):
#   install -m 0755 $BIN/ndsr /usr/local/bin/ndsr
#   useradd --system --home-dir /var/lib/ndsr --create-home --shell /usr/sbin/nologin ndsr
#   install -m 0640 -o root -g ndsr ndsr.env /etc/ndsr.env
#   install -m 0644 ndsr.service /etc/systemd/system/ndsr.service
#   systemctl daemon-reload && systemctl enable --now ndsr
[Unit]
Description=Necter Distributed State Runtime (ndsr) node
After=network-online.target
Wants=network-online.target

[Service]
User=ndsr
Group=ndsr
EnvironmentFile=/etc/ndsr.env
ExecStart=/usr/local/bin/ndsr serve
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadWritePaths=/var/lib/ndsr
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
EOF
  cat > "$VAL/network.necter.ndsr.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- launchd agent for ndsr. Install:
       cp network.necter.ndsr.plist ~/Library/LaunchAgents/
       launchctl load ~/Library/LaunchAgents/network.necter.ndsr.plist
     Edit ProgramArguments to add the flags you need (see: ndsr serve help). -->
<plist version="1.0">
<dict>
  <key>Label</key><string>network.necter.ndsr</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN/ndsr</string>
    <string>serve</string>
    <string>--data-dir</string><string>$HOME/Library/Application Support/network.necter.ndsr</string>
    <string>--bind</string><string>127.0.0.1:7070</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/ndsr.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/ndsr.log</string>
</dict>
</plist>
EOF
  ok "wrote $VAL/ndsr.env.example"
  ok "wrote $VAL/ndsr.service (systemd) and $VAL/network.necter.ndsr.plist (launchd)"
  say "  ${D}Nothing was started.${N}"
}

# ---------------------------------------------------------------- prerequisites

version_ge() {
  # version_ge A B: true if dotted version A >= B
  _a="$1"; _b="$2"
  while [ -n "$_b" ]; do
    _x="${_a%%.*}"; _y="${_b%%.*}"
    _x="${_x:-0}"; _y="${_y:-0}"
    [ "$_x" -gt "$_y" ] 2>/dev/null && return 0
    [ "$_x" -lt "$_y" ] 2>/dev/null && return 1
    case "$_a" in *.*) _a="${_a#*.}" ;; *) _a="0" ;; esac
    case "$_b" in *.*) _b="${_b#*.}" ;; *) _b="" ;; esac
  done
  return 0
}

MISSING=0
check_rust() {
  if has cargo; then ok "cargo $(cargo --version 2>/dev/null | awk '{print $2}')"
  else
    warn "Rust not found. Install it with rustup:"
    say "          curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    MISSING=1
  fi
  if has rustup; then
    if rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
      ok "rust target wasm32-unknown-unknown"
    else
      warn "missing Rust target: rustup target add wasm32-unknown-unknown"
      MISSING=1
    fi
  elif has cargo; then
    warn "rustup not found; make sure your Rust install has the wasm32-unknown-unknown target"
  fi
}

check_go() {
  if has go; then
    _v="$(go env GOVERSION 2>/dev/null | sed 's/^go//')"
    if version_ge "${_v:-0}" 1.21; then ok "go $_v"; else warn "go $_v is older than 1.21: https://go.dev/dl/"; MISSING=1; fi
  else
    warn "Go not found (1.21+): https://go.dev/dl/"
    MISSING=1
  fi
  if [ -n "${TINYGO:-}" ] && [ -x "${TINYGO}" ]; then ok "tinygo $("$TINYGO" version 2>/dev/null | awk '{print $3}') (\$TINYGO)"
  elif has tinygo; then ok "tinygo $(tinygo version 2>/dev/null | awk '{print $3}')"
  else
    warn "TinyGo not found. Official install guide: https://tinygo.org/getting-started/install/"
    if [ "$(uname -s)" = "Darwin" ]; then
      say "          brew tap tinygo-org/tools && brew install tinygo"
    else
      say "          (Debian/Ubuntu: download the tinygo_<version>_amd64.deb / arm64.deb from"
      say "           https://github.com/tinygo-org/tinygo/releases and install it with dpkg -i)"
    fi
    MISSING=1
  fi
}

check_node() {
  if has node; then
    _v="$(node --version 2>/dev/null | sed 's/^v//')"
    if version_ge "${_v:-0}" 18; then ok "node $_v"; else warn "node $_v is older than 18: https://nodejs.org/"; MISSING=1; fi
  else
    warn "Node.js 18+ not found: https://nodejs.org/"
    MISSING=1
  fi
  has npm || { warn "npm not found (it ships with Node.js)"; MISSING=1; }
}

PYTHON=""
check_python() {
  for p in python3.14 python3.13 python3.12 python3.11 python3.10 python3 python; do
    if has "$p"; then
      _v="$("$p" -c 'import sys; print("%d.%d" % sys.version_info[:2])' 2>/dev/null || echo 0)"
      if version_ge "$_v" 3.10; then PYTHON="$p"; ok "$p ($_v)"; return; fi
    fi
  done
  warn "Python 3.10+ not found: https://www.python.org/downloads/"
  MISSING=1
  PYTHON="python3"
}

# ---------------------------------------------------------------- next steps

next_steps() {
  say ""
  say "${B}Next steps${N}"
  if want rust; then
    cat <<EOF

  ${B}Rust${N}
    necter-init rust my_module && cd my_module
    hivec build                                   # -> dist/my_module.hbc
    ndsr run dist/my_module.hbc addNumbers --input '{"a":2,"b":3}'
EOF
  fi
  if want go; then
    cat <<EOF

  ${B}Go${N}
    necter-init go greeter && cd greeter
    go mod tidy
    hivec build .                                 # -> dist/greeter.hbc
    ndsr run dist/greeter.hbc addNumbers --input '{"a":2,"b":3}'
EOF
  fi
  if want typescript; then
    cat <<EOF

  ${B}TypeScript${N}
    necter-init typescript counter_ts && cd counter_ts
    npm install
    npx hivec build counter.ts                    # -> dist/counter.hbc
    ndsr run dist/counter.hbc increment --input 5
EOF
  fi
  if want javascript; then
    cat <<EOF

  ${B}JavaScript${N}
    necter-init javascript counter_js && cd counter_js
    npm install
    npx hivec build counter.js                    # -> dist/counter.hbc
    ndsr run dist/counter.hbc increment --input '{"by":2}' --gas 10000000
EOF
  fi
  if want python; then
    cat <<EOF

  ${B}Python${N}
    necter-init python counter_py && cd counter_py
    ${PYTHON:-python3} -m venv .venv && . .venv/bin/activate
    pip install -r requirements.txt
    hivec build counter.py                        # -> dist/counter.hbc
    ndsr run dist/counter.hbc increment --input '{"by":2}' --gas 10000000
EOF
  fi
  if want miner; then
    if [ "$(uname -s)" = "Linux" ]; then
      cat <<EOF

  ${B}Miner (server)${N}
    necter-miner init
    necter-miner join --project 0x<project_id> --payout 0x<your payout address>
    necter-miner status
    # run it as a systemd service (copies the binary to /usr/local/bin first):
    sudo install -m 0755 "$BIN/necter-miner" /usr/local/bin/necter-miner
    sudo /usr/local/bin/necter-miner service install
EOF
    else
      cat <<EOF

  ${B}Miner${N}
    necter-miner init
    necter-miner join --project 0x<project_id> --payout 0x<your payout address>
    necter-miner ui                               # local dashboard
    necter-miner service install                  # start at login (launchd)
EOF
    fi
  fi
  if want validator; then
    cat <<EOF

  ${B}Node${N}
    ndsr keygen --data-dir ./ndsr-data            # creates node.key, prints the node identity
    ndsr serve --data-dir ./ndsr-data             # local node on http://127.0.0.1:7070
    curl -s http://127.0.0.1:7070/status          # in another terminal
    Service templates and an example config: $VAL
    Validator applications open at mainnet. During the testnet the validator set is
    operated by the Necter team and a node you run does not join it.
    Guide: ${DOCS_URL}/validators/
EOF
  fi
  if want desktop && [ "$(uname -s)" = "Darwin" ]; then
    cat <<EOF

  ${B}Desktop app${N}
    Drag Necter Miner from the disk image to Applications and open it from there
    (first launch: Control-click the app, choose Open, then confirm).
EOF
  fi
  say ""
  say "  Docs: ${DOCS_URL}   Source: ${REPO_URL}"
}

path_hint() {
  case ":${PATH}:" in
    *":$BIN:"*) return 0 ;;
  esac
  say ""
  say "${B}Add $BIN to your PATH${N}"
  _sh="$(basename "${SHELL:-sh}")"
  case "$BIN" in
    "$HOME"/*) _shown="\$HOME${BIN#"$HOME"}" ;;
    *) _shown="$BIN" ;;
  esac
  _line="export PATH=\"$_shown:\$PATH\""
  case "$_sh" in
    zsh)  say "  echo '$_line' >> ~/.zshrc && $_line" ;;
    bash) say "  echo '$_line' >> ~/.bashrc && $_line" ;;
    fish) say "  fish_add_path $_shown" ;;
    *)    say "  $_line   # and add it to your shell profile" ;;
  esac
  say "  ${D}bash: ~/.bashrc   zsh: ~/.zshrc   fish: fish_add_path $_shown${N}"
  return 0
}

# ---------------------------------------------------------------- main

detect_target
TMP="$(mktemp -d 2>/dev/null || mktemp -d -t necter)"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "${B}Necter toolchain${N} ${D}(hivekit v${HIVEKIT_VERSION}, ${TARGET}, into ${NECTER_DIR})${N}"
mkdir -p "$BIN"

SDK=0
for l in rust go typescript javascript python; do want "$l" && SDK=1; done

if [ "$SDK" = 1 ] || want validator; then
  info "NDSR runtime"
  install_binary ndsr "$HIVEKIT_BASE" "$HIVEKIT_VERSION"
fi
if [ "$SDK" = 1 ]; then
  info "Project templates"
  install_templates
fi
if want rust; then
  info "Rust"
  install_binary hivec-rs "$HIVEKIT_BASE" "$HIVEKIT_VERSION"
  check_rust
fi
if want go; then
  info "Go"
  install_binary hivec-go "$HIVEKIT_BASE" "$HIVEKIT_VERSION"
  check_go
fi
if want rust || want go; then write_hivec_dispatcher; fi
if want typescript || want javascript; then
  info "TypeScript / JavaScript"
  check_node
  say "  ${D}SDK: @necter/hivekit (added by each project's package.json)${N}"
fi
if want python; then
  info "Python"
  check_python
  say "  ${D}SDK: necter-hivekit (installed into each project's virtual environment)${N}"
fi
if want miner; then
  info "Miner"
  install_binary necter-miner "$MINER_BASE" "$MINER_VERSION"
fi
if want desktop; then
  info "Necter Miner desktop app"
  install_desktop
fi
if want validator; then
  info "Node service templates"
  install_validator_files
fi

say ""
if [ "$MISSING" = 1 ]; then
  say "${Y}Installed, but some prerequisites are missing (see 'warn' above).${N}"
else
  say "${G}Installed.${N}"
fi
path_hint
next_steps
