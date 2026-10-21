# The image's default: the curated tools (examples/mcp/config/full-access.toml), everything else
# through polarion_call, and global writes (users, global folders, activity comments). The
# projects that may be written are POLARION_WRITE_PROJECTS; there is no wildcard.
export POLARION_MCP_CONFIG="${POLARION_MCP_CONFIG-/etc/polarion/mcp/full-access.toml}"
export POLARION_ALLOW_GLOBAL_WRITES="${POLARION_ALLOW_GLOBAL_WRITES-1}"
