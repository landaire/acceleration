#!/bin/sh
# Runs the ar on PATH. Build scripts exec their tools by path, so the
# toolchain names this script instead of a bare command.
exec ar "$@"
