# Shared private FUSE mount and teardown.
owned_mount() {
    local identity
    identity=$(findmnt -nr -o SOURCE,FSTYPE -M "$mount_dir") || return
    [[ "$identity" == "$mount_name fuse" || "$identity" == "$mount_name fuse.io-journey" ]]
}

start_mount() {
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
    if mountpoint -q "$mount_dir"; then
        owned_mount || return 1
        umount -- "$mount_dir" || return 1
    fi
    if [[ -n "$fuse_pid" ]]; then
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
    fi
}

cleanup_mount() {
    if stop_mount; then return 0; fi
    if [[ -n "$fuse_pid" ]] && ! mountpoint -q "$mount_dir" &&
        [[ $(readlink "/proc/$fuse_pid/exe") == "$binary" ]] &&
        grep -z -F -x -- "$mount_name" "/proc/$fuse_pid/cmdline" >/dev/null; then
        kill -TERM "$fuse_pid" || return
        stop_mount
    fi
    return 1
}
