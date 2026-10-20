#!/bin/sh
# usage: appshot.sh "<command line>" out.png [cols rows]
cd $(dirname $0)/..
pkill -f MacOS/orch-terminal-spike; sleep 1
${S:-/tmp/orch-term-spike}/run.sh >/dev/null; sleep 4
./ctl.sh '{"cmd":"new","n":1}'; sleep 4
./ctl.sh "{\"cmd\":\"eval\",\"js\":\"const p=[...spike.panes.values()][0]; p.fitNow=()=>{}; p.term.resize(${3:-100},${4:-30}); return 1\"}"; sleep 0.5
./ctl.sh "{\"cmd\":\"type\",\"text\":\"$1\\r\"}"
sleep ${WAIT:-3}
W=$(${S:-/tmp/orch-term-spike}/tools/winid OrchTerminal | awk '{print $1}' | head -1)
screencapture -x -o -l $W $2
