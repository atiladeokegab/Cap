#!/usr/bin/env bash
# Probe only: sample top and memory once a second with wall-clock stamps.
out=$1
top -l 0 -s 1 -n 12 -o cpu -stats pid,command,cpu,th,state > "$out/top.txt" 2>&1 &
for _ in $(seq 3600); do
  python3 -c 'import time; print(f"{time.time():.3f}")'
  vm_stat | grep -E "Pages free|Swapouts|compressor"
  sleep 1
done > "$out/vm.txt" 2>&1
