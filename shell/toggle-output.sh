#!/bin/bash
# Toggle speakers/headphones
exec "$(dirname "$0")/g6.sh" --toggle-output
