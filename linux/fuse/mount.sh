# Shared private FUSE mount and teardown.
source "$(dirname -- "${BASH_SOURCE[0]}")/../mount-state.sh"

fuse_mount_matches() {
    local path=$1 name=$2
    local identity
    identity=$(findmnt -nr -o SOURCE,FSTYPE -M "$path") || return
    [[ "$identity" == "$name fuse" || "$identity" == "$name fuse.io-journey" ]]
}

owned_mount() {
    fuse_mount_matches "$mount_dir" "$mount_name"
}

start_mount() {
    mount_absent "$mount_dir" || fail 'Use an inspectable, unmounted FUSE path.'
    "$binary" mount "$backend" "$mount_dir" "$mount_name" >> "$run_dir/fuse.log" 2>&1 &
    fuse_pid=$!
    local attempt
    for attempt in {1..100}; do
        if owned_mount; then return 0; fi
        kill -0 "$fuse_pid" 2>/dev/null || fail 'FUSE exited before mounting.'
        sleep 0.1
    done
    fail 'FUSE did not mount the owned backend.'
}

stop_mount() {
    # A rejected child has no worker of its own to clean up.
    [[ -n "$fuse_pid" ]] || return 0
    unmount_owned_path "$mount_dir" owned_mount || return 1
    local status=0 attempt
    for attempt in {1..50}; do
        if ! kill -0 "$fuse_pid" 2>/dev/null; then
            wait "$fuse_pid" || status=$?
            fuse_pid=
            return "$status"
        fi
        sleep 0.1
    done
    return 1
}

cleanup_mount() {
    if stop_mount; then return 0; fi
    if [[ -n "$fuse_pid" ]] && mount_absent "$mount_dir" &&
        [[ $(readlink "/proc/$fuse_pid/exe") == "$binary" ]] &&
        grep -z -F -x -- "$mount_name" "/proc/$fuse_pid/cmdline" >/dev/null; then
        kill -TERM "$fuse_pid" || return
        stop_mount
    fi
    return 1
}
