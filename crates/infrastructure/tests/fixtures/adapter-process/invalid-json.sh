#!/usr/bin/env bash
# Emits text that is not JSON so the runner must reject it.
set -eu
cat >/dev/null
printf 'not valid json\n'
