#!/usr/bin/env bash
set -euo pipefail

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $# == 11 && $(id -u) == 0 && $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs ]] ||
    fail 'Use the append benchmark in a private guest mount namespace.'
case_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$case_dir/../mount-state.sh"
require_private_mount_namespace
filesystem=$1 engine=$2 group=$3 output=$4
adapter=$5 socket=$6 server_pid=$7 identity=$8 device_id=$9
[[ "$device_id" =~ ^[0-9]+$ ]] || fail 'Invalid lab device ID.'
"$adapter" check-owned "$device_id" "$server_pid" "$identity" "$socket"
mount_dir=${10} mount_name=${11}
[[ "$mount_dir" == "${socket%/*}/mount" && -d "$mount_dir" &&
   "$mount_name" == "$(basename -- "${socket%/*}")" ]] ||
    fail 'Use the mount path and identity assigned by the device owner.'
require_private_mount_parent "$mount_dir"
mount_absent "$mount_dir" || fail 'Use an inspectable, unmounted run path.'
backend="/dev/ublkb$device_id"
! findmnt -rn -S "$backend" >/dev/null || fail 'The owned device is already mounted.'
[[ -d "$output" && $(findmnt -n -o FSTYPE -T "$output") == tmpfs ]] ||
    fail 'Keep the measurements on guest tmpfs.'
append=$(realpath -- "${IO_JOURNEY_APPEND_BIN:?}")
[[ -x "$append" && $(findmnt -n -o FSTYPE -T "$append") == tmpfs ]] ||
    fail 'Use an append binary from guest tmpfs.'

run_dir=$(mktemp -d /dev/shm/io-journey-append-case.XXXXXX)
fuse_pid=

start_mount() {
    mount -t ext4 "$backend" "$mount_dir"
}

owned_ext4_mount() {
    [[ $(findmnt -nr -o SOURCE,FSTYPE -M "$mount_dir") == "$backend ext4" ]]
}

stop_mount() {
    unmount_owned_path "$mount_dir" owned_ext4_mount
}

cleanup_mount() {
    stop_mount
}

if [[ "$filesystem" == fuse ]]; then
    binary=$(realpath -- "${IO_JOURNEY_FUSE_BIN:?}")
    [[ -x "$binary" && $(findmnt -n -o FSTYPE -T "$binary") == tmpfs ]] ||
        fail 'Use a FUSE binary from guest tmpfs.'
    source "$case_dir/../fuse/mount.sh"
elif [[ "$filesystem" != ext4 ]]; then
    fail 'Expected ext4 or fuse.'
fi

cleanup() {
    local status=$?
    trap - EXIT
    set +e
    cleanup_mount || status=1
    if (( status == 0 )); then
        rm -rf -- "$run_dir"
    else
        printf 'Append diagnostics in %s\n' "$run_dir" >&2
        if [[ -f "$run_dir/fuse.log" ]]; then tail -n 20 "$run_dir/fuse.log" >&2; fi
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [[ "$filesystem" == fuse ]]; then
    "$binary" format-device "$backend"
else
    mkfs.ext4 -q -F -b 4096 -m 0 -E nodiscard,lazy_itable_init=0,lazy_journal_init=0 "$backend"
fi

control() {
    "$adapter" control "$socket" "$1"
}

start_mount
read -r _ _ previous_flush < <(control stats)
timeout 30 "$append" run "$engine" "$group" "$mount_dir" > "$run_dir/measurement.tsv"
IFS=$'\t' read -r reported_engine reported_group depth warm measured measured_bytes _ < "$run_dir/measurement.tsv"
[[ "$reported_engine" == "$engine" && "$reported_group" == "$group" && "$depth" == 4 &&
   "$warm" == 64 && "$measured" == 256 && "$measured_bytes" == 131072 ]] ||
    fail 'The append engine did not report the complete acknowledgement receipt.'
acknowledged_records=$((warm + measured))
checkpoint=$(control checkpoint)
read -r _ _ acknowledged_flush <<< "$checkpoint"
(( acknowledged_flush > previous_flush )) || fail 'The timed appends did not reach a new FTL flush.'
printf 'Application acknowledged %s records through file sync.\n' "$acknowledged_records"
grep -m 1 -F "flush $acknowledged_flush published FTL checkpoint" "${socket%/*}/server.log"
timeout 20 "$append" tail "$mount_dir"
stop_mount
blockdev --flushbufs "$backend"
[[ $(control cut) == "$checkpoint" ]] || fail 'The FTL did not reopen the acknowledged checkpoint.'
start_mount
timeout 20 "$append" verify "$mount_dir" "$acknowledged_records"
stop_mount
mount_absent "$mount_dir" && [[ -z "$fuse_pid" ]] || fail 'Could not verify mount and server cleanup.'
(
    set -o noclobber
    printf '%s\t%s\t%s\n' "$filesystem" "$acknowledged_flush" "$(cat "$run_dir/measurement.tsv")" > "$output/result.tsv"
)
printf 'Reopened FTL flush %s and verified all %s acknowledged records.\n' \
    "$acknowledged_flush" "$acknowledged_records"
