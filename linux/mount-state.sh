# Util-linux mountpoint returns 32 for a path confirmed to be unmounted.
mount_state() {
    local status=0
    mountpoint -q -- "$1" || status=$?
    if (( status != 0 && status != 32 )); then
        printf 'Could not check mount state of %s (exit %s).\n' "$1" "$status" >&2
    fi
    return "$status"
}

require_private_mount_namespace() {
    local current initial
    if ! current=$(readlink /proc/self/ns/mnt) || ! initial=$(readlink /proc/1/ns/mnt) ||
        [[ -z "$current" || -z "$initial" || "$current" == "$initial" ]]; then
        printf '%s\n' 'Run in a private mount namespace using unshare --mount --propagation private.' >&2
        return 1
    fi
}

require_private_mount_parent() {
    local path=$1 propagation
    if ! propagation=$(findmnt -n -o PROPAGATION -T "$path") || [[ "$propagation" != private ]]; then
        printf 'The filesystem containing %s must have private mount propagation.\n' "$path" >&2
        return 1
    fi
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
