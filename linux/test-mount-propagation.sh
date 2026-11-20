#!/usr/bin/env bash
set -euo pipefail

linux_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$linux_dir/mount-state.sh"

mount_parent=/dev/shm/example-run/mount
findmnt() {
    [[ "$*" == "-n -o PROPAGATION -T $mount_parent" ]] || return 2
    if [[ "$fixture" == unavailable ]]; then return 1; fi
    printf '%s\n' "$fixture"
}

fixture=private
require_private_mount_parent "$mount_parent"
for fixture in shared unavailable; do
    status=0
    output=$(require_private_mount_parent "$mount_parent" 2>&1) || status=$?
    [[ $status == 1 && "$output" == "The filesystem containing $mount_parent must have private mount propagation." ]]
done
printf '%s\n' 'Only a private mount parent passes preflight.'

readlink() {
    if [[ "$1" == /proc/1/ns/mnt ]]; then
        [[ "$namespace" != unavailable ]] || return 1
        printf '%s\n' 'mnt:[1]'
    elif [[ "$namespace" == same ]]; then
        printf '%s\n' 'mnt:[1]'
    else
        printf '%s\n' 'mnt:[2]'
    fi
}
namespace=distinct
require_private_mount_namespace
for namespace in same unavailable; do
    status=0
    output=$(require_private_mount_namespace 2>&1) || status=$?
    [[ $status == 1 && "$output" == 'Run in a private mount namespace using unshare --mount --propagation private.' ]]
done
printf '%s\n' 'An unreadable or initial namespace fails preflight.'
