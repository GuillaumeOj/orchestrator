#!/bin/sh
# phys_footprint of the spike app + its WebKit helpers (+ PTY children separately)
APP=$(pgrep -f 'MacOS/orch-terminal-spike' | head -1)
T0=$(ps -o lstart= -p $APP)
HELPERS=$(ps -axo pid=,comm= | grep -E 'com.apple.WebKit\.(WebContent|GPU|Networking)$' | awk '{print $1}')
ARGS="-p $APP"; for h in $HELPERS; do ARGS="$ARGS -p $h"; done
footprint $ARGS --noCategories 2>/dev/null | grep -E "Footprint|phys_footprint|^[a-zA-Z.]+ \[|TOTAL|WebKit|orch" 
echo "--- system"; memory_pressure | grep -E "free percentage"; sysctl -n vm.swapusage
