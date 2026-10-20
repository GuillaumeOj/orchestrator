#!/bin/sh
# start app detached; logs to $S/spike.log
cd $(dirname $0)/..
rm -f ${S:-/tmp/orch-term-spike}/spike.log
SPIKE_CTL=/tmp/orch-term-spike.sock SPIKE_LOG=${S:-/tmp/orch-term-spike}/spike.log nohup ./src-tauri/target/release/bundle/macos/OrchTerminalSpike.app/Contents/MacOS/orch-terminal-spike > ${S:-/tmp/orch-term-spike}/stderr.log 2>&1 &
echo $!
