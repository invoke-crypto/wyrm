#!/usr/bin/env bash
#   ./build.sh                 every contract, local toolchain
#   ./build.sh <crate>...      only the named contracts
#   ./build.sh --reproducible  the four deployed contracts inside the pinned docker image; needs committed code
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"

for c in "$@"; do
  if [ "$c" != --reproducible ] && [ ! -f "$c/Cargo.toml" ]; then
    echo "no crate named $c" >&2
    exit 1
  fi
done

long=$(find . -name target -prune -o -name '*.rs' -print0 | xargs -0 -r awk 'FNR == 1001 { sub("^\\./", "", FILENAME); print FILENAME }')
if [ -n "$long" ]; then
  echo "over 1000 lines:" $long >&2
  exit 1
fi

log=$(mktemp)
if cargo test -p wyrm-types --lib >"$log" 2>&1; then
  grep '^test result' "$log"
else
  cat "$log"; exit 1
fi

deployed="tithe scale-vault scales dragon"
if [ "${1:-}" = "--reproducible" ]; then
  if [ -n "$(git status --porcelain -- .)" ]; then
    echo "uncommitted changes: a reproducible build is of the committed code" >&2
    exit 1
  fi
  hashes=target/near/HASHES.txt
  mkdir -p target/near && : >"$hashes"
  for c in $deployed; do
    (cd "$c" && cargo near build reproducible-wasm)
    crate=${c//-/_}
    (cd "target/near/$crate" && sha256sum "$crate.wasm") >>"$hashes"
  done
  cat "$hashes"
  exit 0
fi

wanted=" ${*:-$deployed mock-dcl mock-ft mock-nearly state-cleanup} "
for c in $deployed mock-dcl mock-ft mock-nearly state-cleanup; do
  [[ $wanted == *" $c "* ]] || continue
  # The vault has no #[near] macros, so it exports no ABI to extract. Its reproducible command in Cargo.toml says the same.
  # state-cleanup has no near-sdk, whose metadata is where cargo-near reads the protocol version a contract targets.
  case $c in
    scale-vault) flags=--no-abi ;;
    state-cleanup) flags="--no-abi --skip-rust-version-check" ;;
    *) flags= ;;
  esac
  if (cd "$c" && cargo near build non-reproducible-wasm $flags >"$log" 2>&1); then
    echo "built $c"
  else
    cat "$log"; exit 1
  fi
done
