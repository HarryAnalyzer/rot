#!/usr/bin/env bash
# Reproduce this project the way the hackathon's first screen does: clone the
# public repository into an empty directory and run it, with nothing from the
# author's machine and no toolchain override. If this script is green, a judge
# with Docker can get the same result in one command:
#
#   docker run --rm -v "$PWD:/w" -w /w rust:1.99 bash scripts/verify.sh
#
# It deliberately ignores the working tree it is launched from: the point is to
# test what was actually pushed.
set -euo pipefail

REPO="${1:-https://github.com/HarryAnalyzer/rot}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

missing=()
command -v git >/dev/null 2>&1 || missing+=(git)
command -v python3 >/dev/null 2>&1 || missing+=(python3)
if [ ${#missing[@]} -gt 0 ]; then
  echo "=== installing ${missing[*]}"
  apt-get update -qq >/dev/null 2>&1 || true
  apt-get install -y -qq "${missing[@]}" >/dev/null 2>&1 || true
else
  echo "=== git and python3 already present"
fi

echo "=== cloning $REPO"
git clone -q "$REPO" "$WORK/repo"
cd "$WORK/repo"
echo "    HEAD: $(git log -1 --format=%h\ %s)"

echo "=== toolchain this repo pins"
head -3 rust-toolchain.toml

echo "=== cargo test"
# No RUSTUP_TOOLCHAIN override: rustup honours rust-toolchain.toml and fetches
# the pinned compiler, which is exactly what a judge's machine will do.
cargo test 2>&1 | tail -8

echo "=== python3 sim/simulate.py"
python3 sim/simulate.py 2>&1 | tail -8

echo "=== the simulation must not have rewritten its own committed output"
if git diff --quiet; then
  echo "    clean: re-running the simulation reproduces the committed numbers"
else
  echo "    DIRTY - the committed results do not match a fresh run:"
  git diff --stat
  exit 1
fi

echo
echo "=== all green"
