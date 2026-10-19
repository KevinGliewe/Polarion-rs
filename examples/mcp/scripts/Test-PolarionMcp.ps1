<#
.SYNOPSIS
    Starts polarion-mcp through Start-PolarionMcp.ps1, as an MCP client would, and shows what
    it offers: the write scope from the server's instructions and the tools it lists.

.DESCRIPTION
    Sends initialize and tools/list over stdio. Listing tools does not contact Polarion; with
    -Call it also calls get_project, which logs in and reads the default project.

.PARAMETER Config
    As for Start-PolarionMcp.ps1 (full-access, every-tool, no-deletes, read-only, or a path).

.PARAMETER ReadOnly
    As for Start-PolarionMcp.ps1.

.PARAMETER Call
    Also call get_project: checks URL, token and default project.

.PARAMETER Exe
    The polarion-mcp binary (default: polarion-mcp on PATH).

.EXAMPLE
    .\Test-PolarionMcp.ps1 -Config full-access -Call
#>
[CmdletBinding()]
param(
    [string] $Config,
    [switch] $ReadOnly,
    [switch] $Call,
    [string] $Exe = 'polarion-mcp'
)

$ErrorActionPreference = 'Stop'

$launcher = Join-Path $PSScriptRoot 'Start-PolarionMcp.ps1'
$arguments = @('-NoProfile', '-NonInteractive', '-File', $launcher, '-Exe', $Exe)
if ($Config) { $arguments += '-Config', $Config }
if ($ReadOnly) { $arguments += '-ReadOnly' }

$start = [System.Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
foreach ($a in $arguments) { $start.ArgumentList.Add($a) }
$start.RedirectStandardInput = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.UseShellExecute = $false
$process = [System.Diagnostics.Process]::Start($start)
$stderr = $process.StandardError.ReadToEndAsync()

function Send($message) {
    $process.StandardInput.WriteLine(($message | ConvertTo-Json -Depth 10 -Compress))
    $process.StandardInput.Flush()
}

function Receive([int] $Id) {
    while ($true) {
        $line = $process.StandardOutput.ReadLine()
        if ($null -eq $line) {
            $process.WaitForExit()
            throw "polarion-mcp exited ($($process.ExitCode)): $($stderr.Result.Trim())"
        }
        $message = $line | ConvertFrom-Json
        if ($message.id -eq $Id) { return $message }
    }
}

try {
    Send @{ jsonrpc = '2.0'; id = 1; method = 'initialize'; params = @{
            protocolVersion = '2025-06-18'
            capabilities    = @{}
            clientInfo      = @{ name = 'Test-PolarionMcp'; version = '1' }
        }
    }
    $init = Receive 1
    if ($init.error) { throw "initialize failed: $($init.error.message)" }
    Send @{ jsonrpc = '2.0'; method = 'notifications/initialized' }

    Send @{ jsonrpc = '2.0'; id = 2; method = 'tools/list' }
    $tools = (Receive 2).result.tools
    $size = ($tools | ConvertTo-Json -Depth 100 -Compress).Length

    Write-Host "Server:  $($init.result.serverInfo.name) $($init.result.serverInfo.version)"
    $scope = [regex]::Match($init.result.instructions, '(This server is read-only\.|Writes (are enabled for|are restricted to)[^.]*\.)').Value
    Write-Host "Scope:   $scope"
    Write-Host "Tools:   $($tools.Count) (about $([math]::Round($size / 1KB)) KB of definitions)"
    Write-Host ''
    Write-Host ($tools.name -join ', ')

    if ($Call) {
        Send @{ jsonrpc = '2.0'; id = 3; method = 'tools/call'; params = @{ name = 'get_project'; arguments = @{} } }
        $result = (Receive 3).result
        Write-Host ''
        if ($result.isError) {
            Write-Host "get_project failed: $($result.content[0].text)" -ForegroundColor Red
        } else {
            $project = $result.content[0].text | ConvertFrom-Json
            Write-Host "get_project: $($project.id) ($($project.name))" -ForegroundColor Green
        }
    }
} finally {
    $process.StandardInput.Close()
    if (-not $process.WaitForExit(5000)) { $process.Kill() }
}
