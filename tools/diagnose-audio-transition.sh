#!/usr/bin/env bash
# Read-only diagnostics for two audio symptoms of JamBaLinux SonicCore:
#
#   spatial  Records timestamped PipeWire/PulseAudio events, the default sink,
#            sinks, sink inputs (with their sink and corked flag), PipeWire
#            audio node states and MPRIS PlaybackStatus while the user toggles
#            spatial audio by hand. Used to tell which hypothesis in
#            docs/protocol/audio-processing.md ("Diagnóstico de transições")
#            explains a pause in playback.
#   timing   Measures the wall time of the read-only status commands the
#            widget runs and of their building blocks.
#
# The script only observes. It never moves streams, changes the default sink,
# sets node parameters, links ports, starts or stops units, or mutates the
# SonicCore configuration.
set -euo pipefail

# Stable, parseable output from pactl and a '.' radix in EPOCHREALTIME.
export LC_ALL=C

readonly PROGRAM=${0##*/}

usage() {
	cat <<EOF
Usage:
  $PROGRAM spatial [--duration SECONDS] [--interval SECONDS] [--output FILE]
  $PROGRAM timing [--runs N] [--soniccore PATH]
  $PROGRAM --help

Read-only diagnostics for spatial-audio playback pauses and equalizer latency.
Nothing is changed in PipeWire, PulseAudio, systemd or the SonicCore state.

spatial   Observe the audio graph for SECONDS (default 20) while you toggle
          spatial audio manually (widget or CLI) in another window. Snapshots
          are taken every --interval seconds (default 0.25) and written only
          when they change, next to every 'pactl subscribe' event. The log goes
          to FILE, or by default to
          \$XDG_RUNTIME_DIR/jambalinux-soniccore/diagnose-audio-transition-<time>.log.
          An existing FILE is never overwritten.

timing    Run each read-only status command N times (default 3) and report
          min/median/max wall time in milliseconds:
            soniccore equalizer status --format json
            soniccore spatial status --format json
          plus 'pw-dump', 'pactl list sinks' and '/bin/sh -lc true' (the
          login-shell wrapper the widget uses) for the latency breakdown.
          The soniccore binary is taken from --soniccore, PATH, or
          \$HOME/.cargo/bin/soniccore.

Requires pactl and pw-dump. python3 (node states) and busctl or playerctl
(MPRIS) are optional; missing ones are reported in the log.
EOF
}

die() {
	printf '%s: %s\n' "$PROGRAM" "$1" >&2
	exit "${2:-1}"
}

require_tools() {
	local missing=() tool
	for tool in pactl pw-dump; do
		command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
	done
	if ((${#missing[@]} > 0)); then
		die "required command(s) not found in PATH: ${missing[*]}. Install the PipeWire/PulseAudio client tools (pipewire, pipewire-pulse / libpulse) and retry." 2
	fi
}

positive_integer() {
	[[ $1 =~ ^[1-9][0-9]*$ ]]
}

positive_decimal() {
	[[ $1 =~ ^([0-9]+(\.[0-9]+)?|\.[0-9]+)$ ]] && [[ ! $1 =~ ^[0.]+$ ]]
}

# Microseconds since the epoch.
now_us() {
	if [[ -n ${EPOCHREALTIME:-} ]]; then
		printf '%s' "${EPOCHREALTIME/[.,]/}"
	else
		date +%s%6N
	fi
}

format_ms() {
	local us=$1
	printf '%d.%03d' $((us / 1000)) $((us % 1000))
}

# ---------------------------------------------------------------------------
# spatial

START_US=0
LOG_FILE=
WORK_DIR=
SUBSCRIBE_PID=
READER_PID=

log_line() {
	local category=$1 message=$2 elapsed
	elapsed=$(($(now_us) - START_US))
	printf '+%d.%03d %-9s %s\n' $((elapsed / 1000000)) $((elapsed / 1000 % 1000)) \
		"$category" "$message" >>"$LOG_FILE"
}

log_block() {
	local category=$1 block=$2 line
	if [[ -z $block ]]; then
		log_line "$category" "(empty)"
		return
	fi
	while IFS= read -r line; do
		log_line "$category" "$line"
	done <<<"$block"
}

snapshot_default_sink() {
	pactl get-default-sink 2>&1 || true
}

snapshot_sinks() {
	# index, name, state
	pactl list short sinks 2>&1 | awk -F'\t' '{ printf "sink=%s name=%s state=%s\n", $1, $2, $NF }' || true
}

snapshot_sink_inputs() {
	pactl list sink-inputs 2>&1 | awk '
		function flush() {
			if (index_ != "")
				printf "input=%s sink=%s corked=%s app=%s media=%s\n", index_, sink, corked, app, media
			index_ = ""; sink = "?"; corked = "?"; app = "?"; media = "?"
		}
		function value(line) {
			sub(/^[^=]*= "/, "", line); sub(/"$/, "", line); return line
		}
		/^Sink Input #/ { flush(); index_ = substr($3, 2); next }
		/^\tSink: / { sink = $2; next }
		/^\tCorked: / { corked = $2; next }
		/^\t\tapplication\.name = / { app = value($0); next }
		/^\t\tmedia\.name = / { media = value($0); next }
		END { flush() }
	' || true
}

snapshot_nodes() {
	if ! command -v python3 >/dev/null 2>&1; then
		printf 'unavailable: python3 not found\n'
		return
	fi
	pw-dump 2>/dev/null | python3 -c '
import json, sys
try:
    objects = json.load(sys.stdin)
except Exception as error:
    print(f"unavailable: could not parse pw-dump output ({error})")
    sys.exit(0)
rows = []
for obj in objects:
    if obj.get("type") != "PipeWire:Interface:Node":
        continue
    info = obj.get("info") or {}
    props = info.get("props") or {}
    media_class = props.get("media.class", "")
    if "Audio" not in media_class:
        continue
    rows.append("id=%s serial=%s class=%s state=%s name=%s" % (
        obj.get("id"), props.get("object.serial", "?"), media_class,
        info.get("state", "?"), props.get("node.name", "?")))
print("\n".join(sorted(rows)))
' || printf 'unavailable: pw-dump failed\n'
}

MPRIS_TOOL=
detect_mpris_tool() {
	if command -v busctl >/dev/null 2>&1; then
		MPRIS_TOOL=busctl
	elif command -v playerctl >/dev/null 2>&1; then
		MPRIS_TOOL=playerctl
	fi
}

snapshot_mpris() {
	local name status
	case $MPRIS_TOOL in
	busctl)
		while IFS= read -r name; do
			status=$(busctl --user --timeout=1 get-property "$name" /org/mpris/MediaPlayer2 \
				org.mpris.MediaPlayer2.Player PlaybackStatus 2>/dev/null || printf 'unknown')
			status=${status#s }
			printf '%s %s\n' "${name#org.mpris.MediaPlayer2.}" "${status//\"/}"
		done < <(busctl --user --no-legend list 2>/dev/null |
			awk '$1 ~ /^org\.mpris\.MediaPlayer2\./ { print $1 }' | sort)
		;;
	playerctl)
		playerctl --all-players --format '{{playerName}} {{status}}' status 2>/dev/null || true
		;;
	*)
		printf 'unavailable: neither busctl nor playerctl found\n'
		;;
	esac
}

spatial_config_file() {
	printf '%s/jambalinux-soniccore/spatial.json' "${XDG_CONFIG_HOME:-$HOME/.config}"
}

snapshot_spatial_config() {
	local file
	file=$(spatial_config_file)
	if [[ ! -e $file ]]; then
		printf 'absent: %s\n' "$file"
		return
	fi
	# Report the stored gate as one compact line; the file is only read.
	tr -d '\n' <"$file" | tr -s ' \t' ' ' | head -c 400 || true
	printf '\n'
}

cleanup_spatial() {
	if [[ -n $SUBSCRIBE_PID ]]; then
		kill "$SUBSCRIBE_PID" 2>/dev/null || true
		wait "$SUBSCRIBE_PID" 2>/dev/null || true
	fi
	if [[ -n $READER_PID ]]; then
		wait "$READER_PID" 2>/dev/null || true
	fi
	if [[ -n $WORK_DIR ]]; then
		rm -rf -- "$WORK_DIR"
	fi
}

run_spatial() {
	local duration=20 interval=0.25 output=
	while (($# > 0)); do
		case $1 in
		--duration)
			(($# >= 2)) || die "--duration requires a value"
			positive_integer "$2" || die "--duration must be a positive integer number of seconds"
			duration=$2
			shift 2
			;;
		--interval)
			(($# >= 2)) || die "--interval requires a value"
			positive_decimal "$2" || die "--interval must be a positive number of seconds"
			interval=$2
			shift 2
			;;
		--output)
			(($# >= 2)) || die "--output requires a file path"
			output=$2
			shift 2
			;;
		-h | --help)
			usage
			exit 0
			;;
		*) die "unknown spatial option: $1 (see --help)" ;;
		esac
	done

	require_tools

	if [[ -z $output ]]; then
		[[ -n ${XDG_RUNTIME_DIR:-} && -d ${XDG_RUNTIME_DIR} ]] ||
			die "XDG_RUNTIME_DIR is not set or missing; pass --output FILE"
		local directory="$XDG_RUNTIME_DIR/jambalinux-soniccore"
		mkdir -p -- "$directory"
		output="$directory/diagnose-audio-transition-$(date +%Y%m%dT%H%M%S).log"
	fi
	[[ -e $output ]] && die "refusing to overwrite existing file: $output"
	local output_directory
	output_directory=$(dirname -- "$output")
	[[ -d $output_directory ]] || die "output directory does not exist: $output_directory"

	LOG_FILE=$output
	(
		set -o noclobber
		: >"$LOG_FILE"
	) 2>/dev/null || die "could not create $LOG_FILE"
	WORK_DIR=$(mktemp -d "$output_directory/.diagnose-audio-transition.XXXXXX")
	trap cleanup_spatial EXIT
	trap 'exit 130' INT TERM

	detect_mpris_tool
	START_US=$(now_us)
	{
		printf '# JamBaLinux SonicCore audio transition diagnostics\n'
		printf '# started: %s\n' "$(date --iso-8601=ns 2>/dev/null || date)"
		printf '# duration: %ss, snapshot interval: %ss\n' "$duration" "$interval"
		printf '# mpris: %s\n' "${MPRIS_TOOL:-unavailable}"
		printf '# format: +seconds category message (snapshots are logged only on change)\n'
		pactl info 2>/dev/null | grep -E '^(Server Name|Server Version|Default Sink):' |
			sed 's/^/# /' || true
	} >>"$LOG_FILE"

	# pactl subscribe → FIFO → timestamping reader, so every event keeps the
	# moment it arrived rather than the moment of the next snapshot.
	local fifo="$WORK_DIR/subscribe"
	mkfifo "$fifo"
	(
		while IFS= read -r line; do
			log_line pulse "$line"
		done <"$fifo"
	) &
	READER_PID=$!
	pactl subscribe >"$fifo" 2>&1 &
	SUBSCRIBE_PID=$!

	printf '%s: recording for %ss to %s\n' "$PROGRAM" "$duration" "$LOG_FILE" >&2
	printf '%s: toggle spatial audio now while media is playing\n' "$PROGRAM" >&2

	local -A previous=()
	local categories=(config default sinks inputs nodes mpris)
	local deadline=$((START_US + duration * 1000000)) category snapshot
	while (($(now_us) < deadline)); do
		for category in "${categories[@]}"; do
			case $category in
			config) snapshot=$(snapshot_spatial_config) ;;
			default) snapshot=$(snapshot_default_sink) ;;
			sinks) snapshot=$(snapshot_sinks) ;;
			inputs) snapshot=$(snapshot_sink_inputs) ;;
			nodes) snapshot=$(snapshot_nodes) ;;
			mpris) snapshot=$(snapshot_mpris) ;;
			esac
			if [[ ${previous[$category]-__unset__} != "$snapshot" ]]; then
				log_block "$category" "$snapshot"
				previous[$category]=$snapshot
			fi
		done
		sleep "$interval"
	done
	log_line end "recording finished"

	cleanup_spatial
	SUBSCRIBE_PID=
	READER_PID=
	WORK_DIR=
	trap - EXIT

	local paused
	paused=$(grep -cE '^\+[0-9.]+ mpris +.* Paused$' "$LOG_FILE" || true)
	printf '%s: done; %s MPRIS "Paused" snapshot line(s). Log: %s\n' \
		"$PROGRAM" "${paused:-0}" "$LOG_FILE" >&2
}

# ---------------------------------------------------------------------------
# timing

TIMING_RESULTS=()

time_command() {
	local label=$1 runs=$2
	shift 2
	local samples=() failures=0 run start elapsed error_file first_error=
	error_file=$(mktemp)
	for ((run = 0; run < runs; run++)); do
		start=$(now_us)
		if ! "$@" >/dev/null 2>"$error_file"; then
			failures=$((failures + 1))
			[[ -n $first_error ]] || first_error=$(head -n 1 "$error_file")
		fi
		elapsed=$(($(now_us) - start))
		samples+=("$elapsed")
	done
	rm -f -- "$error_file"
	local sorted
	mapfile -t sorted < <(printf '%s\n' "${samples[@]}" | sort -n)
	local count=${#sorted[@]}
	TIMING_RESULTS+=("$(printf '%-44s %4d %10s %10s %10s %8d' "$label" "$count" \
		"$(format_ms "${sorted[0]}")" "$(format_ms "${sorted[count / 2]}")" \
		"$(format_ms "${sorted[count - 1]}")" "$failures")")
	if [[ -n $first_error ]]; then
		TIMING_RESULTS+=("    first error: $first_error")
	fi
}

run_timing() {
	local runs=3 soniccore=
	while (($# > 0)); do
		case $1 in
		--runs)
			(($# >= 2)) || die "--runs requires a value"
			positive_integer "$2" || die "--runs must be a positive integer"
			runs=$2
			shift 2
			;;
		--soniccore)
			(($# >= 2)) || die "--soniccore requires a path"
			soniccore=$2
			shift 2
			;;
		-h | --help)
			usage
			exit 0
			;;
		*) die "unknown timing option: $1 (see --help)" ;;
		esac
	done

	require_tools

	if [[ -z $soniccore ]]; then
		if command -v soniccore >/dev/null 2>&1; then
			soniccore=$(command -v soniccore)
		elif [[ -x ${HOME:-}/.cargo/bin/soniccore ]]; then
			soniccore=$HOME/.cargo/bin/soniccore
		fi
	fi
	[[ -n $soniccore && -x $soniccore ]] ||
		die "soniccore binary not found; install it or pass --soniccore PATH" 2

	time_command 'pw-dump' "$runs" pw-dump
	time_command 'pactl list sinks' "$runs" pactl list sinks
	if [[ -x /bin/sh ]]; then
		time_command '/bin/sh -lc true (widget wrapper)' "$runs" /bin/sh -lc true
	fi
	time_command 'soniccore equalizer status --format json' "$runs" \
		"$soniccore" equalizer status --format json
	time_command 'soniccore spatial status --format json' "$runs" \
		"$soniccore" spatial status --format json

	printf '# soniccore: %s\n' "$soniccore"
	printf '%-44s %4s %10s %10s %10s %8s\n' command runs min_ms median_ms max_ms failures
	printf '%s\n' "${TIMING_RESULTS[@]}"
}

# ---------------------------------------------------------------------------

main() {
	(($# > 0)) || {
		usage >&2
		exit 64
	}
	case $1 in
	-h | --help | help) usage ;;
	spatial)
		shift
		run_spatial "$@"
		;;
	timing)
		shift
		run_timing "$@"
		;;
	*)
		printf '%s: unknown mode: %s\n\n' "$PROGRAM" "$1" >&2
		usage >&2
		exit 64
		;;
	esac
}

main "$@"
