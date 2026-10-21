# Reads only (examples/mcp/config/read-only.toml). Enforced: the write scope is removed, whatever
# the container was given.
export POLARION_MCP_CONFIG="${POLARION_MCP_CONFIG-/etc/polarion/mcp/read-only.toml}"
unset POLARION_WRITE_PROJECTS POLARION_WRITE_TARGETS POLARION_ALLOW_GLOBAL_WRITES
