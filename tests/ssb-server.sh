#!/usr/bin/env bash

set -euo pipefail

declare detach=""
# shellcheck disable=SC2153
if [[ "${DETACH:-}" =~ ^(1|true)$ ]]; then
    detach="--detach"
fi

ssb_data_path="/tmp/rust-ssb-test"

mkdir -p "${ssb_data_path}"
docker run \
    --name rust-ssb-test-server \
    ${detach} \
    --rm \
    --volume "${ssb_data_path}:/data" \
    --env SSB_path=/data \
    --user "$(id -u)" \
    --publish 8008:8008 \
    thoschol/ssb-server \
    start --logging.level info --verbose
