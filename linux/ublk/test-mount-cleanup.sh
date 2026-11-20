#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
[[ $(findmnt -n -o FSTYPE -T /dev/shm) == tmpfs ]]
source "$adapter_dir/cleanup.sh"
source "$adapter_dir/../fuse/mount.sh"
scratch=$(mktemp -d /dev/shm/io-journey-unmount-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT

# Model umount returning EBUSY for an owned mount. The parent cleanup
# itself stays unchanged through both attempts.
mountpoint() {
    [[ "$2" == "$mount_dir" && -f "$fixture/mounted" ]]
}

findmnt() {
    [[ "${@: -1}" == "$mount_dir" ]] || return 1
    printf '%s\n' "$fixture_identity"
}

umount() {
    [[ "$2" == "$mount_dir" ]] || return 1
    if [[ -e "$fixture/busy" ]]; then
        printf 'busy\n' >> "$fixture/events"
        return 32
    fi
    rm -- "$fixture/mounted"
    printf 'unmounted\n' >> "$fixture/events"
}

server_exited() {
    [[ -e "$fixture/stopped" ]]
}

reap_server() {
    [[ -e "$fixture/stopped" ]] || return 1
    printf 'reaped\n' >> "$fixture/events"
}

device_remains() {
    [[ ! -e "$fixture/deleted" ]]
}

timeout() {
    if [[ -e "$fixture/mounted" ]]; then
        printf 'premature %s\n' "$3" >> "$fixture/events"
        return 1
    fi
    case "$3" in
        control)
            [[ "$5" == stop ]] || return 1
            touch "$fixture/stopped"
            printf 'stop\n' >> "$fixture/events"
            ;;
        remove-owned)
            touch "$fixture/deleted"
            printf 'delete\n' >> "$fixture/events"
            ;;
        *) return 1 ;;
    esac
}

for mount_filesystem in ext4 fuse; do
    fixture="$scratch/$mount_filesystem"
    run_dir="$fixture/owned-run"
    mount_dir="$run_dir/mount" mount_name="${run_dir##*/}"
    binary="$run_dir/adapter" socket="$run_dir/control.sock" log="$run_dir/server.log"
    trace_instance= device_id=98765 device=/dev/ublkb98765 server_pid=999999999 node_identity=7:8:9
    mkdir -p "$mount_dir"
    touch "$fixture/mounted" "$fixture/busy"
    fixture_identity="$device ext4"
    if [[ "$mount_filesystem" == fuse ]]; then fixture_identity="$mount_name fuse"; fi

    if (trap cleanup EXIT; exit 0) > "$fixture/first.log" 2>&1; then
        printf 'Cleanup accepted an EBUSY mount.\n' >&2
        exit 1
    fi
    [[ -e "$fixture/mounted" && ! -e "$fixture/stopped" && ! -e "$fixture/deleted" ]]
    if [[ $(cat "$fixture/events") != busy ]]; then
        cat "$fixture/events" >&2
        exit 1
    fi
    grep -F 'Preserving the ublk server and device' "$fixture/first.log" >/dev/null

    rm -- "$fixture/busy"
    (trap cleanup EXIT; exit 0) > "$fixture/retry.log" 2>&1
    [[ ! -e "$fixture/mounted" && ! -e "$run_dir" ]]
    [[ $(cat "$fixture/events") == $'busy\nunmounted\nstop\nreaped\ndelete' ]]
    printf '%s cleanup kept the device live through EBUSY and deleted it after unmount.\n' "$mount_filesystem"
done
