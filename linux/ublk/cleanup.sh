# The ublk parent owns this path even when a child mounts the filesystem.
owned_ublk_mount() {
    if [[ "$mount_filesystem" == fuse ]]; then
        fuse_mount_matches "$mount_dir" "$mount_name"
    else
        [[ -n "$device" && $(findmnt -nr -o SOURCE,FSTYPE -M "$mount_dir") == "$device ext4" ]]
    fi
}

cleanup() {
    local status=$? cleanup_failed=0 stop_status=0
    trap - EXIT
    set +e
    if [[ -n "$trace_instance" && -d "$trace_instance" ]]; then
        printf '0\n' > "$trace_instance/tracing_on" || cleanup_failed=1
        rmdir "$trace_instance" || cleanup_failed=1
    fi
    if mountpoint -q "$mount_dir"; then
        if owned_ublk_mount; then
            umount -- "$mount_dir" || cleanup_failed=1
        else
            cleanup_failed=1
        fi
    fi
    if mountpoint -q "$mount_dir"; then
        printf 'Preserving the ublk server and device while %s is still mounted.\n' "$mount_dir" >&2
        cleanup_failed=1
    elif [[ -n "$server_pid" ]]; then
        if [[ -z "$device_id" ]]; then
            read_allocation
        fi
        if ! server_exited; then
            timeout 7 "$binary" control "$socket" stop > "$run_dir/stop.log" 2>&1 || stop_status=$?
            if (( stop_status != 0 )); then
                printf 'socket stop failed. Reaping the owned server before deletion\n'
                if [[ $(readlink "/proc/$server_pid/exe") == "$binary" ]]; then
                    kill -TERM "$server_pid" || cleanup_failed=1
                fi
                if (( stop_status == 124 )); then cleanup_failed=1; fi
            fi
        fi
        if reap_server; then
            if device_remains; then
                if timeout 7 "$binary" remove-owned "$device_id" "$server_pid" "$node_identity" "$socket" >> "$run_dir/stop.log" 2>&1; then
                    printf 'deleted the run-owned ublk device after reaping its server\n'
                else
                    cleanup_failed=1
                fi
            fi
        else
            cleanup_failed=1
        fi
    fi
    if device_remains; then cleanup_failed=1; fi
    if mountpoint -q "$mount_dir"; then cleanup_failed=1; fi
    if [[ -n "$server_pid" && -d "/proc/$server_pid" ]]; then cleanup_failed=1; fi
    if (( status == 0 && cleanup_failed == 0 )); then
        printf 'verified cleanup of ublk device %s, metadata, mount, and server\n' "$device_id"
        rm -rf -- "$run_dir"
    else
        printf 'ublk smoke failed (cleanup status %s). Guest diagnostics at %s\n' "$cleanup_failed" "$run_dir" >&2
        if [[ -f "$log" ]]; then tail -n 20 "$log" >&2; fi
        if [[ -f "$run_dir/stop.log" ]]; then tail -n 10 "$run_dir/stop.log" >&2; fi
        if (( status == 0 )); then status=1; fi
    fi
    exit "$status"
}
