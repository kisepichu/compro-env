#!/usr/bin/env bash
# Drains the adapter request from stdin then emits a minimal valid response.
set -eu
cat >/dev/null
cat <<'JSON'
{
  "schema_version": 1,
  "adapter": {
    "name": "test-fixture",
    "version": "0.1.0",
    "toolchains": []
  },
  "libraries": [],
  "solutions": []
}
JSON
