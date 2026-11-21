#!/bin/bash
# Print the device state (read-only); add --json for JSON
exec "$(dirname "$0")/g6.sh" --status "$@"
