#!/bin/sh
# Checks of the polarion-cli and polarion-mcp images, without a Polarion server:
#
#   docker/check-images.sh polarion-cli:dev polarion-mcp:dev
#
# The binary runs in both, the CLI image has the server too, an unknown profile is refused, every
# built-in polarion-mcp profile starts (its config file is read and checked; the server then waits
# for an MCP client on stdin, which is closed here), the CLI's profiles exist, `read-only` removes
# the write scope in both images, and in the polarion-mcp image it refuses write options.
set -eu
cli="$1"
mcp="$2"

docker run --rm "$cli" --version
docker run --rm "$mcp" --version
docker run --rm "$cli" mcp --help >/dev/null

if docker run --rm -e POLARION_PROFILE=unknown "$mcp" --version 2>/dev/null; then
    echo "an unknown profile was accepted" >&2
    exit 1
fi

profiles=$(docker run --rm --entrypoint sh "$mcp" -c 'cd /etc/polarion/profiles && ls *.sh' | sed 's/\.sh$//')
[ -n "$profiles" ]
for profile in $profiles; do
    out=$(docker run --rm -e POLARION_PROFILE="$profile" \
        -e POLARION_URL=http://127.0.0.1:9/polarion -e POLARION_TOKEN=unused \
        --entrypoint polarion-entrypoint "$mcp" sh -c 'unset POLARION_MCP_HTTP; exec polarion mcp' \
        </dev/null 2>&1 || true)
    case "$out" in
    *"initialize request"*) echo "polarion-mcp profile $profile: ok" ;;
    *)
        echo "polarion-mcp profile $profile: $out" >&2
        exit 1
        ;;
    esac
done

for profile in read-only full-access; do
    docker run --rm -e POLARION_PROFILE="$profile" "$cli" --version >/dev/null
    echo "polarion-cli profile $profile: ok"
done

for image in "$cli" "$mcp"; do
    docker run --rm -e POLARION_PROFILE=read-only -e POLARION_WRITE_PROJECTS=X \
        -e POLARION_WRITE_TARGETS= -e POLARION_ALLOW_GLOBAL_WRITES=1 \
        --entrypoint polarion-entrypoint "$image" \
        sh -c 'test -z "${POLARION_WRITE_PROJECTS+x}${POLARION_WRITE_TARGETS+x}${POLARION_ALLOW_GLOBAL_WRITES+x}"'
    echo "$image read-only: no write scope"
done

for option in --write-projects=X --allow-global-writes --write-targets=X-1; do
    if docker run --rm -e POLARION_PROFILE=read-only "$mcp" "$option" --version 2>/dev/null; then
        echo "polarion-mcp read-only accepted $option" >&2
        exit 1
    fi
done
echo "$mcp read-only: write options refused"
