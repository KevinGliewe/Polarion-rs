# Writes to POLARION_WRITE_PROJECTS, but no deletes and no user accounts
# (examples/mcp/config/no-deletes.toml: switched off for polarion_call too). No global writes
# unless POLARION_ALLOW_GLOBAL_WRITES=1 is given.
export POLARION_MCP_CONFIG="${POLARION_MCP_CONFIG-/etc/polarion/mcp/no-deletes.toml}"
