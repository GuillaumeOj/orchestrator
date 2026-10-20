#!/bin/sh
# compose.sh kitty.png app.png out.png  (content area 700x390, labels on top)
S=$(dirname $0)
magick "$1" -crop 700x480+0+33 +repage -background '#111' -fill '#ddd' -font /System/Library/Fonts/Menlo.ttc -pointsize 13 -gravity north -splice 0x20 -annotate +0+3 'kitty 0.49.2' "$S/_k.png"
magick "$2" -crop 700x480+0+51 +repage -background '#111' -fill '#ddd' -font /System/Library/Fonts/Menlo.ttc -pointsize 13 -gravity north -splice 0x20 -annotate +0+3 'xterm.js 6.0 WebGL in WKWebView (spike)' "$S/_a.png"
magick "$S/_k.png" "$S/_a.png" -bordercolor "#111" -border 2 -background '#111' +append "$3"
