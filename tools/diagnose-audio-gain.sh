#!/usr/bin/env bash
# Passive, timestamped audio-gain graph observer.  This script deliberately
# contains no PipeWire, PulseAudio, HID, service, or SonicCore write path.
set -euo pipefail

export LC_ALL=C

readonly PROGRAM=${0##*/}

usage() {
	cat <<EOF
Usage: $PROGRAM [--duration SECONDS] [--interval SECONDS] [--output FILE] [--soniccore PATH]

Collect read-only, timestamped snapshots for the audio-gain evidence ledger.
The output is a text record containing the unmodified outputs of pactl,
pw-dump, optional pw-metadata, SonicCore JSON status, and HRIR file metadata.
It does not capture audio or change PipeWire, SonicCore, HID, or routing.

Required commands: pactl, pw-dump, soniccore (or --soniccore PATH).
Optional command: pw-metadata.  Missing optional data is recorded as unavailable.

The default output is $XDG_RUNTIME_DIR/jambalinux-soniccore/
diagnose-audio-gain-<time>.log. Existing files are never overwritten.
The record may contain application and media names; review it before sharing.
EOF
}

die() {
	printf '%s: %s\n' "$PROGRAM" "$1" >&2
	exit "${2:-1}"
}

positive_integer() {
	[[ $1 =~ ^[1-9][0-9]*$ ]]
}

positive_decimal() {
	[[ $1 =~ ^([1-9][0-9]*|0\.[0-9]*[1-9][0-9]*|\.[0-9]*[1-9][0-9]*)$ ]]
}

# /proc/uptime is a monotonic kernel clock. It is read, never changed.
monotonic_us() {
	local seconds whole fraction
	read -r seconds _ </proc/uptime || die 'cannot read /proc/uptime for a monotonic timestamp'
	whole=${seconds%%.*}
	fraction=${seconds#*.}
	[[ $fraction != "$seconds" ]] || fraction=0
	fraction=${fraction}000000
	fraction=${fraction:0:6}
	printf '%d%06d' "$whole" "$((10#$fraction))"
}

emit_command() {
	local label=$1; shift
	local output status
	printf '[%s]\n' "$label" >>"$LOG_FILE"
	if output=$("$@" 2>&1); then
		printf '%s\n' "$output" >>"$LOG_FILE"
	else
		status=$?
		printf 'unavailable: command exited %s\n%s\n' "$status" "$output" >>"$LOG_FILE"
	fi
	printf '[/ %s]\n' "$label" >>"$LOG_FILE"
}

emit_hrir_metadata() {
	local spatial_json path
	printf '[hrir-file-metadata]\n' >>"$LOG_FILE"
	spatial_json=$("$SONICCORE" spatial status --format json 2>/dev/null || true)
	# The status JSON is separately recorded below. This narrow extraction only
	# locates its advertised local HRIR path; it never follows a guessed path.
	path=$(printf '%s\n' "$spatial_json" | sed -n 's/.*"hrir_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
	if [[ -z $path ]]; then
		printf '%s\n' 'unavailable: spatial status did not expose hrir_path'
	elif [[ ! -f $path ]]; then
		printf 'unavailable: advertised hrir_path is not a regular file: %s\n' "$path"
	else
		printf 'path=%s\n' "$path"
		stat -c 'size_bytes=%s mtime_epoch=%Y mode=%a' -- "$path" 2>&1 || true
		if command -v sha256sum >/dev/null 2>&1; then
			sha256sum -- "$path" 2>&1 || true
		else
			printf '%s\n' 'unavailable: sha256sum not found'
		fi
	fi
	printf '[/ hrir-file-metadata]\n' >>"$LOG_FILE"
}

LOG_FILE=
SONICCORE=

sample() {
	local sequence=$1 monotonic wall
	monotonic=$(monotonic_us)
	wall=$(date --iso-8601=ns 2>/dev/null || date)
	{
		printf '\n=== sample %s ===\n' "$sequence"
		printf 'monotonic_us=%s\nwall_clock=%s\n' "$monotonic" "$wall"
	} >>"$LOG_FILE"
	emit_command 'pactl get-default-sink' pactl get-default-sink
	emit_command 'pactl list sinks (sink volume/mute/format/endpoint identity)' pactl list sinks
	emit_command 'pactl list sink-inputs (stream volume/mute/format/app names)' pactl list sink-inputs
	emit_command 'pw-dump (nodes/links/Props/channel maps/negotiated formats)' pw-dump
	if command -v pw-metadata >/dev/null 2>&1; then
		emit_command 'pw-metadata (available metadata)' pw-metadata
	else
		printf '[pw-metadata (available metadata)]\nunavailable: pw-metadata not found\n[/ pw-metadata (available metadata)]\n' >>"$LOG_FILE"
	fi
	emit_command 'soniccore status --format json' "$SONICCORE" status --format json
	emit_command 'soniccore equalizer status --format json' "$SONICCORE" equalizer status --format json
	emit_command 'soniccore spatial status --format json' "$SONICCORE" spatial status --format json
	emit_hrir_metadata
	printf '=== end sample %s ===\n' "$sequence" >>"$LOG_FILE"
}

main() {
	local duration=20 interval=1 output= sequence=0 start now deadline directory
	while (($# > 0)); do
		case $1 in
		--duration) (($# >= 2)) || die '--duration requires SECONDS'; positive_integer "$2" || die '--duration must be a positive integer'; duration=$2; shift 2 ;;
		--interval) (($# >= 2)) || die '--interval requires SECONDS'; positive_decimal "$2" || die '--interval must be a positive decimal'; interval=$2; shift 2 ;;
		--output) (($# >= 2)) || die '--output requires FILE'; output=$2; shift 2 ;;
		--soniccore) (($# >= 2)) || die '--soniccore requires PATH'; SONICCORE=$2; shift 2 ;;
		-h|--help|help) usage; exit 0 ;;
		*) die "unknown option: $1 (see --help)" 64 ;;
		esac
	done
	command -v pactl >/dev/null 2>&1 || die 'required command not found in PATH: pactl' 2
	command -v pw-dump >/dev/null 2>&1 || die 'required command not found in PATH: pw-dump' 2
	if [[ -z $SONICCORE ]]; then SONICCORE=$(command -v soniccore || true); fi
	[[ -n $SONICCORE && -x $SONICCORE ]] || die 'required SonicCore executable not found; pass --soniccore PATH' 2
	if [[ -z $output ]]; then
		[[ -n ${XDG_RUNTIME_DIR:-} && -d ${XDG_RUNTIME_DIR:-} ]] || die 'XDG_RUNTIME_DIR is unavailable; pass --output FILE'
		directory="$XDG_RUNTIME_DIR/jambalinux-soniccore"
		mkdir -p -- "$directory"
		output="$directory/diagnose-audio-gain-$(date +%Y%m%dT%H%M%S).log"
	fi
	[[ ! -e $output ]] || die "refusing to overwrite existing file: $output"
	directory=$(dirname -- "$output")
	[[ -d $directory ]] || die "output directory does not exist: $directory"
	LOG_FILE=$output
	(set -o noclobber; : >"$LOG_FILE") 2>/dev/null || die "could not create output file: $LOG_FILE"
	{
		printf '# JamBaLinux SonicCore passive audio-gain observation\n'
		printf '# No audio was captured and no PipeWire, SonicCore, HID, or routing state was changed.\n'
		printf '# Warning: pactl may expose potentially sensitive application and media names; review before sharing.\n'
		printf '# Raw command output is retained so missing fields are not invented.\n'
	} >>"$LOG_FILE"
	printf '%s: recording passive samples to %s\n' "$PROGRAM" "$LOG_FILE" >&2
	start=$(monotonic_us)
	deadline=$((start + duration * 1000000))
	while :; do
		sequence=$((sequence + 1))
		sample "$sequence"
		now=$(monotonic_us)
		((now >= deadline)) && break
		sleep "$interval"
	done
	printf '%s: wrote %s sample(s); review application/media names before sharing\n' "$PROGRAM" "$sequence" >&2
}

main "$@"
