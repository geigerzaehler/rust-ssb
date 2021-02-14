#!/usr/bin/env bash

set -euo pipefail

RUSTDOCFLAGS="-D broken_intra_doc_links -D warnings --cfg docsrs" \
  cargo doc --workspace --all-features --no-deps --document-private-items
