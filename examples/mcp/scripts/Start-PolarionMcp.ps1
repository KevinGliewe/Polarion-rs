<#
.SYNOPSIS
    Starts the MCP server (polarion mcp) with the settings saved by Save-PolarionSettings.ps1.

.DESCRIPTION
    MCP clients start their servers with an environment of their own, which does not always
    include the user's environment variables. This script reads the POLARION_* user variables
    from the registry (HKCU\Environment) and the token from %APPDATA%\polarion-mcp\token.txt
    (DPAPI), sets them for the server only and starts it. Standard input and output are
    passed through unchanged, so it works as a stdio MCP server.

    Variables already set in the environment (e.g. by the client's "env" block) win over the
    saved ones, so one saved setup can serve several client entries with different scopes.
    (On Windows a variable cannot be set to an empty value; use -ReadOnly for a read-only
    entry.)

.PARAMETER Config
    The server's config file (TOML): a path, or the name of a file in ..\config
    (full-access, every-tool, no-deletes, read-only).

.PARAMETER ReadOnly
    Ignore the saved write settings: a read-only server, whatever was saved.

.PARAMETER Http
    Serve Streamable HTTP at this address (e.g. 127.0.0.1:8787) instead of stdio.

.PARAMETER Exe
    The polarion binary (default: polarion on PATH). It is started as "polarion mcp".

.EXAMPLE
    pwsh -NoProfile -File Start-PolarionMcp.ps1 -Config full-access
#>
[CmdletBinding()]
param(
    [string] $Config,
    [switch] $ReadOnly,
    [string] $Http,
    [string] $Exe = 'polarion'
)

$ErrorActionPreference = 'Stop'

if ([IO.Path]::GetFileNameWithoutExtension($Exe) -eq 'polarion-mcp') {
    # Up to 0.3 the server was a program of its own; started with "mcp" it would only print usage.
    [Console]::Error.WriteLine("Start-PolarionMcp: -Exe names polarion-mcp, which no longer exists; pass the polarion binary (the server is 'polarion mcp')")
    exit 1
}

$names = @(
    'POLARION_URL', 'POLARION_PROJECT', 'POLARION_INSECURE',
    'POLARION_WRITE_PROJECTS', 'POLARION_ALLOW_GLOBAL_WRITES', 'POLARION_WRITE_TARGETS',
    'POLARION_MCP_FILES', 'POLARION_MCP_TOOLS', 'POLARION_MCP_REQUIRE_GUIDES',
    'POLARION_MCP_AUTH_TOKEN', 'POLARION_MCP_ALLOW_UNAUTHENTICATED', 'POLARION_MCP_ALLOWED_HOSTS',
    'POLARION_MCP_ALLOWED_ORIGINS'
)
foreach ($name in $names) {
    # What the client set wins over what was saved.
    if (-not (Test-Path "Env:$name")) {
        $saved = [Environment]::GetEnvironmentVariable($name, 'User')
        if ($null -ne $saved) { Set-Item "Env:$name" -Value $saved }
    }
}

if ($ReadOnly) {
    foreach ($name in 'POLARION_WRITE_PROJECTS', 'POLARION_ALLOW_GLOBAL_WRITES', 'POLARION_WRITE_TARGETS') {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
}

if (-not $env:POLARION_TOKEN -and -not ($env:POLARION_USER -and $env:POLARION_PASSWORD)) {
    $tokenFile = Join-Path $env:APPDATA 'polarion-mcp\token.txt'
    $plain = [Environment]::GetEnvironmentVariable('POLARION_TOKEN', 'User')
    if (-not (Test-Path $tokenFile) -and $plain) {
        # A token kept as a plain user variable rather than with Save-PolarionSettings.ps1.
        $env:POLARION_TOKEN = $plain
    } elseif (-not (Test-Path $tokenFile)) {
        # stderr only: stdout belongs to the MCP protocol.
        [Console]::Error.WriteLine("Start-PolarionMcp: no token; run Save-PolarionSettings.ps1 first ($tokenFile is missing)")
        exit 1
    } else {
        $secure = Get-Content $tokenFile | ConvertTo-SecureString
        $env:POLARION_TOKEN = [System.Net.NetworkCredential]::new('', $secure).Password
    }
}

$arguments = @('mcp')
if ($Config) {
    $path = $Config
    if (-not (Test-Path $path)) {
        $path = Join-Path $PSScriptRoot "..\config\$Config.toml"
    }
    $arguments += '--config', (Resolve-Path $path).Path
}
if ($Http) {
    $arguments += '--http', $Http
}

# Not in a pipeline and not captured: polarion inherits this process's standard input and
# output directly.
& $Exe @arguments
exit $LASTEXITCODE
