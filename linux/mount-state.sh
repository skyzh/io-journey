# Util-linux mountpoint returns 32 for a path confirmed to be unmounted.
mount_state() {
    local status=0
    mountpoint -q -- "$1" || status=$?
    if (( status != 0 && status != 32 )); then
        printf 'Could not check mount state of %s (exit %s).\n' "$1" "$status" >&2
    fi
    return "$status"
}

mount_absent() {
    local status=0
    mount_state "$1" || status=$?
    (( status == 32 ))
}

unmount_owned_path() {
    local path=$1 owner_matches=$2 status=0
    mount_state "$path" || status=$?
    if (( status == 32 )); then return 0; fi
    (( status == 0 )) || return 1
    "$owner_matches" || return 1
    umount -- "$path" || return 1
    mount_absent "$path"
}
