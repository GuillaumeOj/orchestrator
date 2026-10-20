#!/bin/sh
# Send one JSON command to the running spike: ./ctl.sh '{"cmd":"new","n":6}'
exec python3 -c 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall(sys.argv[2].encode()+b"\n"); s.close()' "${SPIKE_CTL:-/tmp/orch-term-spike.sock}" "$1"
