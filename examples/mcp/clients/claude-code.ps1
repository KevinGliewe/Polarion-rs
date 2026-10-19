# Claude Code: register polarion-mcp (run in PowerShell 7; Windows PowerShell 5.1 drops the `--`).
# Pick ONE of the blocks below; they all use the name "polarion", so a second `add` needs a
# `claude mcp remove polarion --scope user` first, or another name.
#
# Replace C:/Polarion-rs with the folder of your checkout (or wherever you copied examples/mcp).
# --scope user: available in every project; --scope project writes .mcp.json into the current
# folder (see claude-code.mcp.json); --scope local (the default): this folder, only for you.

$launcher = 'C:/Polarion-rs/examples/mcp/scripts/Start-PolarionMcp.ps1'

# 1. Full access: every project in POLARION_WRITE_PROJECTS, global writes, curated tools.
claude mcp add polarion --scope user -- `
    pwsh -NoProfile -NonInteractive -File $launcher -Config full-access

# 2. Full access, every command as a tool of its own (large tool list).
# claude mcp add polarion --scope user -- `
#     pwsh -NoProfile -NonInteractive -File $launcher -Config every-tool

# 3. Writes, but no deletes and no user accounts.
# claude mcp add polarion --scope user -- `
#     pwsh -NoProfile -NonInteractive -File $launcher -Config no-deletes

# 4. Read-only, whatever was saved.
# claude mcp add polarion --scope user -- `
#     pwsh -NoProfile -NonInteractive -File $launcher -Config read-only -ReadOnly

# 5. Only a few objects may be written (a sandbox): the -e value wins over the saved settings.
# claude mcp add polarion-sandbox --scope user `
#     -e "POLARION_WRITE_TARGETS=MP-42,doc:Specs/Scratch,run:MP-Sandbox-Run" -- `
#     pwsh -NoProfile -NonInteractive -File $launcher -Config full-access

# 6. Without the launcher: polarion-mcp directly, everything in the entry (the token is then
#    stored in plain text in ~/.claude.json).
# claude mcp add polarion --scope user `
#     -e POLARION_URL=https://polarion.example.com/polarion `
#     -e POLARION_TOKEN=<token> `
#     -e POLARION_PROJECT=MyProject `
#     -e "POLARION_WRITE_PROJECTS=MyProject,OtherProject" `
#     -e POLARION_ALLOW_GLOBAL_WRITES=1 `
#     -e POLARION_MCP_FILES=C:/Users/me/polarion-files -- `
#     polarion-mcp --config C:/Polarion-rs/examples/mcp/config/full-access.toml

# 7. The shared HTTP server (Start-PolarionMcp.ps1 -Http 127.0.0.1:8787 running elsewhere).
# claude mcp add polarion --scope user --transport http http://127.0.0.1:8787/mcp `
#     --header "Authorization: Bearer $([Environment]::GetEnvironmentVariable('POLARION_MCP_AUTH_TOKEN', 'User'))"

# Check: lists the server and whether it connected.
claude mcp list
