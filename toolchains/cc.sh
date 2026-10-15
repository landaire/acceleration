#!/bin/sh
# Runs the cc on PATH. Build scripts exec their compiler by path, so the
# toolchain names this script instead of a bare command.
exec cc "$@"
