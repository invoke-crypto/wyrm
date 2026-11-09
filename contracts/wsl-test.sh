#!/usr/bin/env bash
# Run it from Windows as `npm run test:wsl -- <args>`.
#   wsl-test.sh [--src <checkout>] [--dst <copy>] [--no-sync] [--no-unit] [--crates <a,b>] [<sandbox test>...]
#   --src      the checkout to mirror, default the current directory (the repo root under npm)
#   --dst      the WSL copy, a folder inside your WSL home, default ~/wyrm
#   --no-sync  build and test the copy as it is
#   --crates   build and unit-test only these crates, comma-separated
#   Flags come before the sandbox test names. TEST_FILTER=<name> runs only the matching sandbox tests.
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

src=$PWD dst=$HOME/wyrm sync=1 unit=1 crates=""
while [ $# -gt 0 ]; do
  case "$1" in
    --src) src=$2; shift 2 ;;
    --dst) dst=$2; shift 2 ;;
    --no-sync) sync=0; shift ;;
    --no-unit) unit=0; shift ;;
    --crates) crates=${2//,/ }; shift 2 ;;
    -*) echo "unknown flag $1" >&2; exit 2 ;;
    *) break ;;
  esac
done
for t in "$@"; do
  if [[ $t == -* ]]; then echo "flag $t comes after a sandbox test name; put flags first" >&2; exit 2; fi
done

src=${src/#\~/$HOME} dst=${dst/#\~/$HOME}
[[ $src =~ ^[A-Za-z]:[\\/] ]] && src=$(wslpath -u "$src")
# PowerShell expands a leading ~ to the Windows profile before WSL sees it. The copy belongs in the WSL home.
[[ $dst =~ ^[A-Za-z]:[\\/]Users[\\/][^\\/]+(.*)$ ]] && dst=$HOME${BASH_REMATCH[1]//\\//}
home=$(realpath -m -- "$HOME") src=$(realpath -m -- "$src") dst=$(realpath -m -- "$dst")
if [[ $dst != "$home"/?* || $dst == /mnt/* ]]; then
  echo "--dst must be a folder inside $home, got $dst" >&2; exit 1
fi
if [ $sync = 1 ]; then
  if [[ $dst == "$src" || $dst == "$src"/* || $src == "$dst"/* ]]; then
    echo "--src $src and --dst $dst overlap, and the sync would delete the source" >&2; exit 1
  fi
  if [ ! -f "$src/contracts/build.sh" ]; then echo "no checkout at $src" >&2; exit 1; fi
  mkdir -p "$dst/contracts"
  # tar keeps mtimes, so cargo reuses target/ after the mirror.
  find "$dst/contracts" -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} +
  rm -rf "$dst/docs" "$dst/gym/out"
  (cd "$src" && tar --exclude=contracts/target -cf - contracts docs gym/out) | tar -xf - -C "$dst"
  find "$dst/contracts" -name target -prune -o -name '*.sh' -exec sed -i 's/\r$//' {} +
fi

cd "$dst/contracts" || exit 1
status=0
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-6}
if ! bash ./build.sh $crates; then echo "BUILD FAIL"; exit 1; fi
for c in ${crates:-tithe scale-vault scales dragon mock-dcl mock-ft mock-nearly}; do
  wasm=target/near/${c//-/_}/${c//-/_}.wasm
  [ -f "$wasm" ] && echo "wasm ${c//-/_} $(stat -c %s "$wasm")"
done

if [ $unit = 1 ]; then
  echo "== unit"
  units=${crates:-tithe scale-vault scales dragon wyrm-types}
  cargo test $(printf -- '-p %s ' $units) 2>&1 | grep -E '^test result|FAILED|panicked|^error|^ +--> '
  [ "${PIPESTATUS[0]}" = 0 ] || status=1
fi

echo "== sandbox"
for t in "$@"; do
  log=/tmp/sandbox-$(tr / _ <<<"${dst#/}")-$(printf %s "$dst" | sha256sum | cut -c1-8)-$t.log
  cargo test -p tests --test "$t" -- ${TEST_FILTER:-} --nocapture --test-threads=2 >"$log" 2>&1
  code=$?
  echo "-- $t exit=$code log=$log"
  [ $code = 0 ] || status=1
  grep -E '^test |test result|GAS|panicked at|^error' "$log" | head -80
done
exit $status
