#!/bin/sh
# Starts a bazel-remote action cache on localhost and points Buck2 at it.
#
# Usage: ci/start-action-cache.sh CACHE_DIR [MAX_SIZE_GB]
#
# Buck2 keeps no action cache between daemon runs on its own, so a fresh CI
# runner would rebuild everything. CACHE_DIR holds the cache's blobs; CI
# restores and saves it with actions/cache. The server's process ID and log
# go to CACHE_DIR.pid and CACHE_DIR.log. Writes .buckconfig.local, which
# switches the build to the execution platform that consults the cache.
set -eu

cache_dir=$1
max_size_gb=${2:-4}
grpc_port=9092
http_port=9093
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)

mkdir -p "$cache_dir"
bazel-remote \
    --dir "$cache_dir" \
    --max_size "$max_size_gb" \
    --grpc_address "127.0.0.1:$grpc_port" \
    --http_address "127.0.0.1:$http_port" \
    </dev/null >"$cache_dir.log" 2>&1 &
echo $! >"$cache_dir.pid"

attempt=0
until curl --silent --fail "http://127.0.0.1:$http_port/status" >/dev/null; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 50 ]; then
        echo "error: bazel-remote did not start; see $cache_dir.log" >&2
        exit 1
    fi
    sleep 0.2
done

cat >"$root/.buckconfig.local" <<CONFIG
[build]
  execution_platforms = root//platforms:cached

[buck2]
  # Rule implementations upload only the actions that request it otherwise.
  default_allow_cache_upload = true

[buck2_re_client]
  action_cache_address = grpc://127.0.0.1:$grpc_port
  cas_address = grpc://127.0.0.1:$grpc_port
  # Buck2 requires an engine address; no action executes remotely.
  engine_address = grpc://127.0.0.1:$grpc_port
  tls = false
CONFIG
