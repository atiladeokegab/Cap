#!/usr/bin/env bash
# Build avfread, make test clips, then read them idle and with every core
# saturated by 2x as many busy processes. Usage: run.sh <work dir>
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:-$here/work}
mkdir -p "$work"
swiftc -O -o "$work/avfread" "$here/avfread.swift" || exit 1
for spec in "1920x1080:60" "3024x1964:60" "640x360:60"; do
  res=${spec%%:*}
  [[ -f $work/clip-$res.mp4 ]] ||
    ffmpeg -loglevel error -f lavfi -i "testsrc2=size=$res:rate=60:duration=10" \
      -c:v libx264 -pix_fmt yuv420p -g 120 -movflags +faststart "$work/clip-$res.mp4"
done
cores=$(sysctl -n hw.ncpu)
for res in 640x360 1920x1080 3024x1964; do
  "$work/avfread" "$work/clip-$res.mp4" "idle"
done
burn=()
for _ in $(seq $((cores * 2))); do yes >/dev/null & burn+=($!); done
sleep 1
for res in 640x360 1920x1080 3024x1964; do
  "$work/avfread" "$work/clip-$res.mp4" "cpu-burn"
done
kill "${burn[@]}" 2>/dev/null
wait 2>/dev/null
echo "cores=$cores $(sysctl -n machdep.cpu.brand_string 2>/dev/null) $(sysctl -n hw.model 2>/dev/null)"
