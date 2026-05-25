#!/usr/bin/env bash
set -euo pipefail

mkdir -p /workspace/.server/cargo-home /workspace/.server/target

exec cargo watch \
  --poll \
  -w apps \
  -w packages \
  -w Cargo.toml \
  -w Cargo.lock \
  -x "run -p railyard-server"
