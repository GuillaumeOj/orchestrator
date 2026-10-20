#!/bin/sh
S=${S:-/tmp/orch-term-spike}
cd $(dirname $0)/..
C=./ctl.sh
pkill -f MacOS/orch-terminal-spike; sleep 2; $S/run.sh >/dev/null; sleep 5
# 0. DOM renderer memory (no WebGL)
$C '{"cmd":"webgl","on":false}'; $C '{"cmd":"new","n":1}'; sleep 8; $S/mem.sh > $S/mem-1pane-dom.txt
$C '{"cmd":"new","n":5}'; sleep 10; $S/mem.sh > $S/mem-6pane-dom.txt
APP=$(pgrep -f MacOS/orch-terminal-spike); for k in $(pgrep -P $APP); do footprint -p $k --noCategories 2>/dev/null | grep -E "^\S.*Footprint"; done > $S/mem-shells.txt
pkill -f MacOS/orch-terminal-spike; sleep 2; $S/run.sh >/dev/null; sleep 5
# 1. raw WebKit limit: 24 visible WebGL terminals
$C '{"cmd":"webgl_stress","n":24}'; sleep 6
$C '{"cmd":"state"}'; sleep 1
cp $S/spike.log $S/spike-webgl-stress.log
pkill -f MacOS/orch-terminal-spike; sleep 2; $S/run.sh >/dev/null; sleep 5
# 2. FR-1005 on: 4 tabs x 6 panes
$C '{"cmd":"dropHidden","on":true}'
for t in 1 2 3 4; do $C '{"cmd":"new","n":6,"tab":true}'; sleep 6; done
$C '{"cmd":"state"}'; sleep 1
for t in 0 1 2 3 0 2; do $C "{\"cmd\":\"tab\",\"i\":$t}"; sleep 1.5; done
$C '{"cmd":"state"}'; sleep 2
$S/mem.sh > $S/mem-24term-drop.txt
cp $S/spike.log $S/spike-webgl-drop.log
pkill -f MacOS/orch-terminal-spike; sleep 2; $S/run.sh >/dev/null; sleep 5
# 3. FR-1005 off
$C '{"cmd":"dropHidden","on":false}'
for t in 1 2 3 4; do $C '{"cmd":"new","n":6,"tab":true}'; sleep 6; done
for t in 0 1 2 3 0; do $C "{\"cmd\":\"tab\",\"i\":$t}"; sleep 1.5; done
$C '{"cmd":"state"}'; sleep 2
$S/mem.sh > $S/mem-24term-nodrop.txt
cp $S/spike.log $S/spike-webgl-nodrop.log
pkill -f MacOS/orch-terminal-spike
