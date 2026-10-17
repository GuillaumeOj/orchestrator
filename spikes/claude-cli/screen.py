#!/usr/bin/env python3
"""Render a pty.raw capture to the final screen text (needs pyte). usage: screen.py pty.raw [cols rows]"""
import sys, pyte
cols, rows = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (160, 50)
sc = pyte.Screen(cols, rows); st = pyte.ByteStream(sc); st.feed(open(sys.argv[1], "rb").read())
print("\n".join(l.rstrip() for l in sc.display if l.strip()))
