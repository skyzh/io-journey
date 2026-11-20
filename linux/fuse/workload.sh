#!/usr/bin/env bash
set -euo pipefail

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $(id -u) == 0 && -c /dev/fuse ]] || fail 'Run the lab as guest root.'
[[ $(readlink /proc/self/ns/mnt) != "$(readlink /proc/1/ns/mnt)" ]] ||
    fail 'The lab needs a private mount namespace.'
[[ $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs ]] || fail '/dev/shm must be tmpfs.'
[[ $# == 1 || $# == 6 ]] || fail 'Use the FUSE smoke entry.'

fuse_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
binary=$(realpath -- "$1")
[[ -x "$binary" && $(findmnt -n -o FSTYPE -T "$binary") == tmpfs ]] ||
    fail 'The FUSE server binary must be in guest tmpfs.'
run_dir=$(mktemp -d /dev/shm/io-journey-fuse.XXXXXX)
mount_dir="$run_dir/mount"
mount_name="${run_dir##*/}"
fuse_pid=
backend="$run_dir/lab.img"
mode=image

owned_mount() {
    local identity
    identity=$(findmnt -nr -o SOURCE,FSTYPE -M "$mount_dir") || return
    [[ "$identity" == "$mount_name fuse" || "$identity" == "$mount_name fuse.io-journey" ]]
}

stop_mount() {
    if mountpoint -q "$mount_dir"; then
        owned_mount || return 1
        umount -- "$mount_dir" || return 1
    fi
    if [[ -n "$fuse_pid" ]]; then
        local status=0
        for attempt in {1..50}; do
            if ! kill -0 "$fuse_pid" 2>/dev/null; then
                wait "$fuse_pid" || status=$?
                fuse_pid=
                return "$status"
            fi
            sleep 0.1
        done
        return 1
    fi
}

cleanup() {
    local status=$?
    trap - EXIT
    set +e
    stop_mount || status=1
    if [[ -n "$fuse_pid" ]] && ! mountpoint -q "$mount_dir" &&
        [[ $(readlink "/proc/$fuse_pid/exe") == "$binary" ]] &&
        grep -z -F -x -- "$mount_name" "/proc/$fuse_pid/cmdline" >/dev/null; then
        kill -TERM "$fuse_pid"
        stop_mount
    fi
    if (( status == 0 )); then
        rm -rf -- "$run_dir"
    else
        printf 'FUSE diagnostics in %s\n' "$run_dir" >&2
        if [[ -f "$run_dir/fuse.log" ]]; then tail -n 20 "$run_dir/fuse.log" >&2; fi
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir "$mount_dir"
if (( $# == 6 )); then
    mode=ublk
    adapter=$2 socket=$3 server_pid=$4 identity=$5 device_id=$6
    [[ "$device_id" =~ ^[0-9]+$ ]] || fail 'Invalid lab device ID.'
    "$adapter" check-owned "$device_id" "$server_pid" "$identity" "$socket"
    backend="/dev/ublkb$device_id"
    ! findmnt -rn -S "$backend" >/dev/null || fail 'The owned device is already mounted.'
    "$binary" format-device "$backend"
    read -r _ _ previous_flush < <("$adapter" control "$socket" stats)
else
    "$binary" new-image "$backend"
fi

start_mount() {
    "$binary" mount "$backend" "$mount_dir" "$mount_name" >> "$run_dir/fuse.log" 2>&1 &
    fuse_pid=$!
    for attempt in {1..100}; do
        if owned_mount; then return 0; fi
        kill -0 "$fuse_pid" 2>/dev/null || fail 'FUSE exited before mounting.'
        sleep 0.1
    done
    fail 'FUSE did not mount the owned backend.'
}

load_checkpoint() {
    if [[ "$mode" == ublk ]]; then
        blockdev --flushbufs "$backend"
        [[ $("$adapter" control "$socket" cut) == "$saved_checkpoint" ]] ||
            fail 'The server did not reopen the saved FTL image.'
    else
        mv -- "$run_dir/checkpoint.img" "$backend"
    fi
}

save_checkpoint() {
    if [[ "$mode" == ublk ]]; then
        local flush_number
        saved_checkpoint=$("$adapter" control "$socket" checkpoint)
        read -r _ _ flush_number <<< "$saved_checkpoint"
        (( flush_number > previous_flush )) || fail 'FUSE sync did not reach an FTL flush.'
        printf 'Saved FUSE checkpoint at FTL flush %s before unmount.\n' "$flush_number"
        previous_flush=$flush_number
    else
        cp --reflink=never --sparse=always -- "$backend" "$run_dir/checkpoint.img"
    fi
}

client() {
    timeout 20 python3 -EB "$fuse_dir/workload.py" "$1" "$mount_dir"
}

start_mount
client append
save_checkpoint
client tail
stop_mount
load_checkpoint
start_mount
client verify
client replace
save_checkpoint
client truncate
stop_mount
load_checkpoint
start_mount
client verify-replacement
stop_mount

inspection=$("$binary" inspect "$backend")
[[ "$inspection" == "generation 2 allocated 2"$'\n'"\"events.log\""$'\t'"4111"$'\t'"2" ]] ||
    fail "Unexpected recovered allocation: $inspection"
! mountpoint -q "$mount_dir" || fail 'The owned FUSE mount survived.'
[[ -z "$fuse_pid" ]] || fail 'The owned FUSE server was not reaped.'
printf 'Verified %s replay, two reachable blocks, and cleanup of the FUSE mount and server.\n' "$mode"
