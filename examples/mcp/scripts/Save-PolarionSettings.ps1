<#
.SYNOPSIS
    Saves the Polarion connection and write settings for polarion-mcp (and the polarion CLI).

.DESCRIPTION
    The token is stored encrypted for the current Windows user (DPAPI) in
    %APPDATA%\polarion-mcp\token.txt; it never goes into an MCP client's configuration file.
    Everything else is stored as user environment variables (HKCU\Environment):
    POLARION_URL, POLARION_PROJECT, POLARION_WRITE_PROJECTS, POLARION_ALLOW_GLOBAL_WRITES,
    POLARION_MCP_FILES, POLARION_INSECURE and, with -HttpToken, POLARION_MCP_AUTH_TOKEN.

    Start-PolarionMcp.ps1 reads both and starts polarion-mcp, so every MCP client only needs
    to start that script.

.PARAMETER Url
    Polarion base URL, e.g. https://polarion.example.com/polarion

.PARAMETER Project
    Default project: ids without a project (MP-42, Specs/SRS) are in this project, and the
    project section of polarion_guide is read from it.

.PARAMETER WriteProjects
    Projects that may be written, by id. Leave empty for a read-only server.

.PARAMETER AllProjects
    Write every project the account can see: asks Polarion for the list (polarion projects
    list) and stores it in POLARION_WRITE_PROJECTS. Run again when projects are added.

.PARAMETER GlobalWrites
    Also allow writes that are not tied to a project: users, global folders, activity comments.

.PARAMETER FilesDir
    Exchange folder for attachments and PDF exports (created if missing).

.PARAMETER HttpToken
    Generate a new bearer token for the HTTP server (POLARION_MCP_AUTH_TOKEN, 64 hex digits).
    Needs PowerShell 7.

.PARAMETER Insecure
    Accept invalid TLS certificates (test servers with self-signed certificates only).

.PARAMETER KeepToken
    Keep the stored token instead of asking for a new one.

.EXAMPLE
    .\Save-PolarionSettings.ps1 -Url https://polarion.example.com/polarion -Project MyProject `
        -AllProjects -GlobalWrites -FilesDir "$HOME\polarion-files"

.EXAMPLE
    .\Save-PolarionSettings.ps1 -Url https://polarion.example.com/polarion -Project MyProject
    # read-only: no write projects, no global writes
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string] $Url,
    [Parameter(Mandatory)] [string] $Project,
    [string[]] $WriteProjects = @(),
    [switch] $AllProjects,
    [switch] $GlobalWrites,
    [string] $FilesDir,
    [switch] $HttpToken,
    [switch] $Insecure,
    [switch] $KeepToken
)

$ErrorActionPreference = 'Stop'

$dir = Join-Path $env:APPDATA 'polarion-mcp'
$tokenFile = Join-Path $dir 'token.txt'
New-Item -ItemType Directory -Force $dir | Out-Null

function Set-UserVariable([string] $Name, [string] $Value) {
    # An empty value removes the variable.
    [Environment]::SetEnvironmentVariable($Name, $(if ($Value) { $Value } else { $null }), 'User')
    Set-Item "Env:$Name" -Value $Value  # this session too, for the projects list below
}

if (-not ($KeepToken -and (Test-Path $tokenFile))) {
    $secure = Read-Host 'Polarion personal access token' -AsSecureString
    # Without -Key, ConvertFrom-SecureString encrypts with DPAPI: only this Windows user on this
    # machine can decrypt it.
    $secure | ConvertFrom-SecureString | Set-Content -Path $tokenFile -Encoding ascii
    Write-Host "Token saved (encrypted) to $tokenFile"
}

foreach ($plain in 'POLARION_TOKEN', 'POLARION_PASSWORD') {
    if ([Environment]::GetEnvironmentVariable($plain, 'User')) {
        Write-Warning ("$plain is also set as a plain user environment variable. Clients that pass " +
            "it on use it instead of the encrypted token; remove it with " +
            "[Environment]::SetEnvironmentVariable('$plain', `$null, 'User')")
    }
}

Set-UserVariable POLARION_URL $Url.TrimEnd('/')
Set-UserVariable POLARION_PROJECT $Project
Set-UserVariable POLARION_INSECURE $(if ($Insecure) { '1' } else { '' })

if ($AllProjects) {
    $polarion = Get-Command polarion -ErrorAction SilentlyContinue
    if (-not $polarion) {
        throw 'polarion (the CLI) is not on PATH: install it (cargo install polarion-cli) or pass -WriteProjects'
    }
    $secure = Get-Content $tokenFile | ConvertTo-SecureString
    $env:POLARION_TOKEN = [System.Net.NetworkCredential]::new('', $secure).Password
    try {
        $projects = & $polarion.Source --json projects list | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw "polarion projects list failed (exit code $LASTEXITCODE)" }
    } finally {
        Remove-Item Env:POLARION_TOKEN
    }
    $WriteProjects = @($projects | ForEach-Object { $_.id } | Where-Object { $_ } | Sort-Object -Unique)
    Write-Host "$($WriteProjects.Count) projects found"
}
Set-UserVariable POLARION_WRITE_PROJECTS ($WriteProjects -join ',')
Set-UserVariable POLARION_ALLOW_GLOBAL_WRITES $(if ($GlobalWrites) { '1' } else { '' })

if ($FilesDir) {
    New-Item -ItemType Directory -Force $FilesDir | Out-Null
    Set-UserVariable POLARION_MCP_FILES (Resolve-Path $FilesDir).Path
} else {
    Set-UserVariable POLARION_MCP_FILES ''
}

if ($HttpToken) {
    $bytes = [System.Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
    Set-UserVariable POLARION_MCP_AUTH_TOKEN ([Convert]::ToHexString($bytes).ToLowerInvariant())
    Write-Host 'New HTTP bearer token saved in POLARION_MCP_AUTH_TOKEN'
}

Write-Host ''
Write-Host "POLARION_URL                 = $env:POLARION_URL"
Write-Host "POLARION_PROJECT             = $env:POLARION_PROJECT"
Write-Host "POLARION_WRITE_PROJECTS      = $(if ($env:POLARION_WRITE_PROJECTS) { $env:POLARION_WRITE_PROJECTS } else { '(none: read-only)' })"
Write-Host "POLARION_ALLOW_GLOBAL_WRITES = $(if ($env:POLARION_ALLOW_GLOBAL_WRITES) { 'yes' } else { 'no' })"
Write-Host "POLARION_MCP_FILES           = $(if ($env:POLARION_MCP_FILES) { $env:POLARION_MCP_FILES } else { '(off)' })"
Write-Host ''
Write-Host 'Restart your MCP clients (and open a new terminal) to pick up the changes.'
