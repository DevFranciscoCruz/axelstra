#!/bin/sh
# Unix twin of prepare-sidecar.ps1: builds the daemon and names it the way Tauri's externalBin expects
# (`axelstra-<target triple>`); the bundle ships it beside the app binary as `axelstra`.
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo build --release -p axelstra
triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p app/src-tauri/binaries
cp target/release/axelstra "app/src-tauri/binaries/axelstra-$triple"
