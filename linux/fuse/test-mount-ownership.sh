#!/usr/bin/env bash
set -euo pipefail

fuse_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$fuse_dir/mount.sh"

mount_name=assigned-child
mount_dir=/example/private-run/mount
run_dir=/example/private-run
binary=/example/adapter
backend=/example/device
fixture_identity="$mount_name fuse"
mounted=1
events=
fuse_pid=

fail() { printf '%s\n' "$*" >&2; exit 1; }
mountpoint() {
    [[ "${@: -1}" == "$mount_dir" ]] || return 1
    if (( mounted )); then return 0; fi
    return 32
}
findmnt() {
    [[ "${@: -1}" == "$mount_dir" ]] || return 1
    printf '%s\n' "$fixture_identity"
}
umount() {
    [[ "$2" == "$mount_dir" ]] || return 1
    mounted=0
    events+='unmount '
}
kill() {
    [[ "$1" == -0 ]] || events+='signal '
    return 1
}
wait() {
    [[ "$1" == "$fuse_pid" && "$mounted" == 0 ]] || return 1
    events+='reap '
    return 1
}

cleanup_mount
[[ "$mounted" == 1 && -z "$events" ]] || fail 'A child without a mount attempt changed an existing mount.'
status=0
output=$(start_mount 2>&1) || status=$?
[[ $status == 1 && "$output" == 'Use an inspectable, unmounted FUSE path.' ]]

fuse_pid=12345
fixture_identity='another-owner fuse'
if cleanup_mount; then fail 'Cleanup accepted a foreign mount.'; fi
[[ "$mounted" == 1 && -z "$events" && "$fuse_pid" == 12345 ]]

fixture_identity="$mount_name fuse"
if cleanup_mount; then fail 'Cleanup hid the partial worker failure.'; fi
[[ "$mounted" == 0 && "$events" == 'unmount reap ' && -z "$fuse_pid" ]]
printf '%s\n' 'Rejected children leave the mount alone. An owned partial start unmounts before reap.'
