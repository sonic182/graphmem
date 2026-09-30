#!/usr/bin/env bash
set -euo pipefail

build() {
  mix release
}

function deploy {
  build
}

deploy
