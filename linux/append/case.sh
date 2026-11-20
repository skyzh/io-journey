#!/usr/bin/env bash
set -euo pipefail

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $# == 9 && $(id -u) == 0 && $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs &&
   $(readlink /proc/self/ns/mnt) != "$(readlink /proc/1/ns/mnt)" ]] ||
    fail 'Use the append benchmark in a private guest mount namespace.'
filesystem=$1 engine=$2 group=$3 output=$4
adapter=$5 socket=$6 server_pid=$7 identity=$8 device_id=$9
[[ "$device_id" =~ ^[0-9]+$ ]] || fail 'Invalid lab device ID.'
"$adapter" check-owned "$device_id" "$server_pid" "$identity" "$socket"
backend="/dev/ublkb$device_id"
! findmnt -rn -S "$backend" >/dev/null || fail 'The owned device is already mounted.'
[[ -d "$output" && $(findmnt -n -o FSTYPE -T "$output") == tmpfs ]] ||
    fail 'Keep the measurements on guest tmpfs.'
append=$(realpath -- "${IO_JOURNEY_APPEND_BIN:?}")
[[ -x "$append" && $(findmnt -n -o FSTYPE -T "$append") == tmpfs ]] ||
    fail 'Use an append binary from guest tmpfs.'

case_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
run_dir=$(mktemp -d /dev/shm/io-journey-append-case.XXXXXX)
mount_dir="$run_dir/mount"
mount_name="${run_dir##*/}"
fuse_pid=

start_mount() {
    mount -t ext4 "$backend" "$mount_dir"
}

stop_mount() {
    if mountpoint -q "$mount_dir"; then
        [[ $(findmnt -nr -o SOURCE,FSTYPE -M "$mount_dir") == "$backend ext4" ]] || return 1
        umount -- "$mount_dir"
    fi
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

mkdir "$mount_dir"
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
checkpoint=$(control checkpoint)
read -r _ _ acknowledged_flush <<< "$checkpoint"
(( acknowledged_flush > previous_flush )) || fail 'The timed appends did not reach a new FTL flush.'
timeout 20 "$append" tail "$mount_dir"
stop_mount
blockdev --flushbufs "$backend"
[[ $(control cut) == "$checkpoint" ]] || fail 'The FTL did not reopen the acknowledged checkpoint.'
start_mount
timeout 20 "$append" verify "$mount_dir"
stop_mount
! mountpoint -q "$mount_dir" && [[ -z "$fuse_pid" ]] || fail 'The owned mount or server survived.'
(
    set -o noclobber
    printf '%s\t%s\t%s\n' "$filesystem" "$acknowledged_flush" "$(cat "$run_dir/measurement.tsv")" > "$output/result.tsv"
)
printf 'Reopened FTL flush %s and verified all 320 acknowledged records.\n' "$acknowledged_flush"
