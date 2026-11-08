#!/usr/bin/env bash
# cargo audit + cargo deny (outils installés une fois dans le volume arr-tools).
. "$(dirname "$0")/lib.sh"
cargo_do -v arr-tools:/tools -e CARGO_INSTALL_ROOT=/tools -- \
  "apk add -q --no-cache git openssl-dev openssl-libs-static pkgconf >/dev/null; \
   [ -x /tools/bin/cargo-audit ] || cargo install --locked cargo-audit; \
   [ -x /tools/bin/cargo-deny ] || cargo install --locked cargo-deny; \
   /tools/bin/cargo-audit audit && /tools/bin/cargo-deny check"
echo "audit: OK"
