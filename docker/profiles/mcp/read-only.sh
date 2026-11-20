# Reads only (examples/mcp/config/read-only.toml). Enforced: the write scope is removed, whatever
# the container was given.
export POLARION_MCP_CONFIG="${POLARION_MCP_CONFIG-/etc/polarion/mcp/read-only.toml}"
unset POLARION_WRITE_PROJECTS POLARION_WRITE_TARGETS POLARION_ALLOW_GLOBAL_WRITES
# The server also takes the write scope as options (`polarion mcp --write-projects ...`): refused
# here, so that arguments after the image name cannot bring it back.
for arg in "$@"; do
    case "$arg" in
    --write-projects | --write-projects=* | --write-targets | --write-targets=* | --allow-global-writes | --allow-global-writes=*)
        echo "POLARION_PROFILE=read-only: $arg is not allowed" >&2
        exit 64
        ;;
    esac
done
