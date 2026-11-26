#!/usr/bin/env bash
set -euo pipefail

adapter_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
source "$adapter_dir/queue-case.sh"
fixture=$(mktemp -d /var/tmp/io-journey-trace-test.XXXXXX)
trap 'rm -rf -- "$fixture"' EXIT
mkdir "$fixture/existing"
printf '1\n' > "$fixture/existing/tracing_on"

trace_instance=
if create_trace_instance "$fixture/existing" 2>/dev/null; then
    printf 'An existing trace instance was accepted.\n' >&2
    exit 1
fi
[[ -z "$trace_instance" && $(cat "$fixture/existing/tracing_on") == 1 ]]

create_trace_instance "$fixture/new"
[[ "$trace_instance" == "$fixture/new" && -d "$trace_instance" ]]
printf 'Trace creation collision leaves the existing instance unowned and untouched.\n'
