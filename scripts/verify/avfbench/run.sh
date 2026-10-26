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
arm() { # <label> <count> <prefix...>
  local label=$1 count=$2; shift 2
  local burn=()
  for _ in $(seq "$count"); do "$@" yes >/dev/null & burn+=($!); done
  sleep 1
  "$work/avfread" "$work/clip-1920x1080.mp4" "$label"
  kill "${burn[@]}" 2>/dev/null
  wait 2>/dev/null
}
arm "burn-1x-cores" "$cores" env
arm "burn-2x-cores" $((cores * 2)) env
arm "burn-2x-nice20" $((cores * 2)) nice -n 20
arm "burn-2x-taskpolicy-b" $((cores * 2)) taskpolicy -b
arm "burn-half-cores" $((cores / 2)) env
echo "cores=$cores $(sysctl -n machdep.cpu.brand_string 2>/dev/null) $(sysctl -n hw.model 2>/dev/null)"
