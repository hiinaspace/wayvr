#!/usr/bin/env bash
set -euo pipefail

iterations="${1:-100}"
delay="${2:-0.10}"
wayvrctl="${WAYVRCTL:-/home/s/code/wayvr/target/debug/wayvrctl}"
monitor_a="${MONITOR_A:-DP-5}"
monitor_b="${MONITOR_B:-DP-6}"

if [[ ! "$iterations" =~ ^[0-9]+$ ]] || ((iterations < 1)); then
	echo "usage: $0 [iterations] [delay-seconds]" >&2
	exit 2
fi

echo "Cycling $monitor_a and $monitor_b $iterations times with ${delay}s pacing" >&2

{
	for ((i = 0; i < iterations; i++)); do
		printf 'overlay-set-visible %q 0\n' "$monitor_a"
		printf 'overlay-set-visible %q 0\n' "$monitor_b"
		sleep "$delay"

		printf 'overlay-set-visible %q 1\n' "$monitor_a"
		printf 'overlay-set-visible %q 1\n' "$monitor_b"
		sleep "$delay"

		printf 'overlay-adjust %q --x 0.01 --scale 1.01\n' "$monitor_a"
		printf 'overlay-adjust %q --x=-0.01 --scale 0.99\n' "$monitor_b"
		sleep "$delay"

		# Return both overlays to their starting pose before the next cycle.
		printf 'overlay-adjust %q --x=-0.01 --scale 0.990099\n' "$monitor_a"
		printf 'overlay-adjust %q --x 0.01 --scale 1.010101\n' "$monitor_b"
		sleep "$delay"
	done

	# Leave the monitor set visible, matching normal interactive use.
	printf 'overlay-set-visible %q 1\n' "$monitor_a"
	printf 'overlay-set-visible %q 1\n' "$monitor_b"
} | "$wayvrctl" batch --fail-fast
