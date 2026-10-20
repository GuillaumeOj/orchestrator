#!/bin/sh
S=${S:-/tmp/orch-term-spike}
for v in channel coalesce pull; do
  pkill -f MacOS/orch-terminal-spike; sleep 2
  case $v in
    channel) PRE=""; export SPIKE_COALESCE_MS=0;;
    coalesce) PRE=""; export SPIKE_COALESCE_MS=8;;
    pull) PRE='{"cmd":"eval","js":"window.spikePull=true; return 1"}'; export SPIKE_COALESCE_MS=0;;
  esac
  $S/run.sh > /dev/null; sleep 5
  $S/mem.sh > $S/mem-0pane-$v.txt
  PRE="$PRE" TAG="-$v" $S/bench.sh
  cp $S/spike.log $S/spike-$v.log
done
pkill -f MacOS/orch-terminal-spike
echo done
