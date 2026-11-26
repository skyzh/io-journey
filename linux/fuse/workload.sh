#!/usr/bin/env bash
set -euo pipefail

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

[[ $(id -u) == 0 && -c /dev/fuse ]] || fail 'Run the lab as guest root.'
[[ $# == 1 || $# == 8 ]] || fail 'Use the FUSE smoke entry.'

fuse_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$fuse_dir/mount.sh"
require_private_mount_namespace
require_private_mount_parent /var/tmp
binary=$(realpath -- "$1")
[[ -x "$binary" ]] || fail 'Expected executable at '"$binary"
run_dir=$(mktemp -d /var/tmp/io-journey-fuse.XXXXXX)
mount_dir="$run_dir/mount"
mount_name="${run_dir##*/}"
fuse_pid=
backend="$run_dir/lab.img"
mode=image

cleanup() {
    local status=$?
    trap - EXIT
    set +e
    cleanup_mount || status=1
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

if (( $# == 8 )); then
    mode=ublk
    adapter=$2 socket=$3 server_pid=$4 identity=$5 device_id=$6
    [[ "$device_id" =~ ^[0-9]+$ ]] || fail 'Invalid lab device ID.'
    "$adapter" check-owned "$device_id" "$server_pid" "$identity" "$socket"
    [[ "$7" == "${socket%/*}/mount" && -d "$7" && "$8" == "$(basename -- "${socket%/*}")" ]] ||
        fail 'Use the mount path and identity assigned by the device owner.'
    mount_dir=$7 mount_name=$8
    require_private_mount_parent "$mount_dir"
    backend="/dev/ublkb$device_id"
    mount_absent "$mount_dir" || fail 'Use an inspectable, unmounted run path.'
    ! findmnt -rn -S "$backend" >/dev/null || fail 'The owned device is already mounted.'
    "$binary" format-device "$backend"
    read -r _ _ previous_flush < <("$adapter" control "$socket" stats)
else
    mkdir "$mount_dir"
    "$binary" new-image "$backend"
fi

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
mount_absent "$mount_dir" || fail 'Could not confirm removal of the FUSE mount.'
[[ -z "$fuse_pid" ]] || fail 'The owned FUSE server was not reaped.'
printf 'Verified %s replay, two reachable blocks, and cleanup of the FUSE mount and server.\n' "$mode"
