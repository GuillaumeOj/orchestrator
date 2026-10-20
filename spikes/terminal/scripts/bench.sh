#!/bin/sh
# bench.sh: 1 pane then 6 panes; throughput + frame times. Results in spike.log
cd $(dirname $0)/..
C=./ctl.sh
D=${S:-/tmp/orch-term-spike}/demo
w() { # wait for a report line count to grow
  n=$(grep -c '"ev":"run"' ${S:-/tmp/orch-term-spike}/spike.log); t=0; while [ $(grep -c '"ev":"run"' ${S:-/tmp/orch-term-spike}/spike.log) -le $n ] && [ $t -lt 150 ]; do sleep 1; t=$((t+1)); done; [ $t -ge 150 ] && echo "TIMEOUT" >> ${S:-/tmp/orch-term-spike}/spike.log; }
[ -n "$PRE" ] && $C "$PRE"; sleep 0.5
$C '{"cmd":"new","n":1}'; sleep 4
$C "{\"cmd\":\"type\",\"text\":\"cd $D; clear\r\"}"; sleep 1
$C '{"cmd":"frames","ms":3000,"tag":"idle-1"}'; sleep 6; ${S:-/tmp/orch-term-spike}/mem.sh > ${S:-/tmp/orch-term-spike}/mem-1pane$TAG.txt
$C '{"cmd":"run","tag":"yes-1","line":"yes | head -n 2000000"}'; w
$C '{"cmd":"run","tag":"cat-1","line":"cat big.txt"}'; w
$C '{"cmd":"new","n":5}'; sleep 6
for i in 2 3 4 5 6; do $C "{\"cmd\":\"type\",\"pane\":$i,\"text\":\"cd $D; clear\r\"}"; done; sleep 2
$C '{"cmd":"frames","ms":3000,"tag":"idle-6"}'; sleep 6; ${S:-/tmp/orch-term-spike}/mem.sh > ${S:-/tmp/orch-term-spike}/mem-6pane$TAG.txt
$C '{"cmd":"run","tag":"yes-6","panes":[1,2,3,4,5,6],"line":"yes | head -n 2000000"}'; w
$C '{"cmd":"run","tag":"cat-6","panes":[1,2,3,4,5,6],"line":"cat big.txt"}'; w
$C '{"cmd":"state"}'; sleep 1; ${S:-/tmp/orch-term-spike}/mem.sh > ${S:-/tmp/orch-term-spike}/mem-6pane-after$TAG.txt
