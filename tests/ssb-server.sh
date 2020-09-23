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
    --tty \
    --volume "${ssb_data_path}:/data" \
    --env SSB_path=/data \
    --env "DEBUG=*" \
    --user "$(id -u)" \
    --publish 8008:8008 \
    --hostname "ssb-test-server.local" \
    thoschol/ssb-server:v1 \
    start \
    --logging.level info \
    --verbose \
    --host "ssb-test-server.local" \
    --replicate.legacy false \
    --ebt.logging true
