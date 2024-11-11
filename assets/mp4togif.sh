#!/bin/bash

ffmpeg -i "$1" -filter_complex "fps=10,scale=1120:-1[s]; [s]split[a][b]; [a]palettegen[palette]; [b][palette]paletteuse" "$2"
