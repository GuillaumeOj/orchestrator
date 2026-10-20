#!/bin/sh
printf 'Nerd:        \U000f0219 \n'
printf 'Box : ┌─┬─┐ │ ├─┼─┤ └─┴─┘ ░▒▓█ ⠿ ✓ ✗ →\n'
printf 'CJK : 漢字 かな 한글  Emoji: 🙂 🚀 ✅\n'
printf 'Liga: => -> != === <= >= :: || www &&\n'
printf '\033[1mBold\033[0m \033[3mItalic\033[0m \033[4mUnder\033[0m \033[9mStrike\033[0m \033[2mDim\033[0m \033[7mRev\033[0m\n'
for i in 0 1 2 3 4 5 6 7; do printf "\033[4${i}m  \033[0m"; done; printf '\n'
for i in 0 1 2 3 4 5 6 7; do printf "\033[10${i}m  \033[0m"; done; printf '\n'
awk 'BEGIN{for(c=0;c<78;c++){r=255-c*255/77;g=c*510/77;b=c*255/77;if(g>255)g=510-g;printf "\033[48;2;%d;%d;%dm \033[0m",r,g,b}; printf "\n"}'
