#!/usr/bin/env bash
# Probe only: loop the vfr-holds playback case idle, under pure CPU load and
# under VideoToolbox load (7 more copies of the case), recording every request
# of 100 ms or more. Usage: probe-arms.sh <out dir>
set -uo pipefail
out=$1
mkdir -p "$out"
eval "$(python3 scripts/verify/plan.py env --tier fast --platform macos --suites playback)"
export CAP_VERIFY_TIER=fast
bin=$(cargo test --profile ci-verify -p cap-verify --test playback --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "playback":
        print(m["executable"])')
echo "binary: $bin"
# The test binary loads FFmpeg from @executable_path/../Frameworks.
[[ -e target/ci-verify/Frameworks ]] || ln -s ../Frameworks target/ci-verify/Frameworks
export DYLD_FALLBACK_LIBRARY_PATH="$PWD/target/Frameworks/Spacedrive.framework/Libraries:$PWD/target/native-deps/lib"
[[ -x $bin ]] || exit 2
case_glob='playback/vfr-holds/full/seq60-from30/*'
one() { # <label> <iteration>
  CAP_VERIFY_CASE=$case_glob CAP_VERIFY_LATENCY_FILE="$out/$1.txt" CAP_VERIFY_LATENCY_MIN_MS=100 \
    "$bin" play_from_middle --exact --test-threads=1 >"$out/$1-$2.log" 2>&1
  echo "$1 $2 exit=$?" >>"$out/exits.txt"
}
export CAP_PROBE_AVF_TIMING=1
# Fixture generation, once.
one warmup 0
for i in $(seq 5); do one idle "$i"; done
burn=()
for _ in $(seq 12); do yes >/dev/null & burn+=($!); done
for i in $(seq 5); do one cpu "$i"; done
kill "${burn[@]}"
grep -h "avf-step" "$out"/idle-*.log | awk '{s[$2]+=$3; n[$2]++} END {for (k in s) printf "idle %s avg %.1f ms n=%d\n", k, s[k]/n[k], n[k]}'
grep -h "avf-step" "$out"/cpu-*.log | awk '{s[$2]+=$3; n[$2]++; if ($3>m[$2]) m[$2]=$3} END {for (k in s) printf "cpu %s avg %.1f ms max %.1f n=%d\n", k, s[k]/n[k], m[k], n[k]}'
grep -h "avf-step" "$out"/cpu-*.log | head -12
grep -c . "$out"/*.txt
grep -v "exit=0" "$out/exits.txt" | head -40
