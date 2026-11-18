#!/bin/sh
# Writes .buckconfig.local for a GitHub-hosted runner. The build then runs on
# the execution platform that consults the action cache (platforms/defs.bzl).
# landaire/build-cache's buck-cache action adds the cache's address when the
# job can reach the cache. On Windows it appends to this file, so run it after.
#
# The C compiler, linker and MSVC come from the runner image, and action keys
# do not cover them. host_tools names the image, so each image version keeps
# its own cache entries.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

cat >"$root/.buckconfig.local" <<CONFIG
[build]
  execution_platforms = root//platforms:cached

[action_cache]
  host_tools = $ImageOS-$ImageVersion-$RUNNER_ARCH
CONFIG
