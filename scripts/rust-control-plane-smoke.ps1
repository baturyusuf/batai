$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$binary = (Resolve-Path (Join-Path $PSScriptRoot '..\src-tauri\target\debug\batai-control.exe')).Path
$smokeRoot = Join-Path ([IO.Path]::GetTempPath()) ('batai-rust-smoke-' + [guid]::NewGuid().ToString('N'))
$stdout = Join-Path $smokeRoot 'daemon.stdout.log'
$stderr = Join-Path $smokeRoot 'daemon.stderr.log'
$daemon = $null

try {
  New-Item -ItemType Directory -Path (Join-Path $smokeRoot '.batai\tasks') -Force | Out-Null
  $env:BATAI_PROJECT_ROOT = $smokeRoot
  $env:BATAI_PORT = '0'
  $daemon = Start-Process -FilePath $binary -ArgumentList @('daemon') -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr

  $descriptorPath = Join-Path $smokeRoot '.runtime\daemon.json'
  for ($attempt = 0; $attempt -lt 100 -and -not (Test-Path -LiteralPath $descriptorPath); $attempt++) {
    Start-Sleep -Milliseconds 50
  }
  if (-not (Test-Path -LiteralPath $descriptorPath)) { throw 'Daemon descriptor was not created' }
  $descriptor = Get-Content -LiteralPath $descriptorPath -Raw | ConvertFrom-Json

  $health = Invoke-RestMethod -Uri "$($descriptor.endpoint)/api/health"
  if (-not $health.ok -or $health.runtime -ne 'READY' -or $health.owner -ne 'daemon') {
    throw 'Daemon health contract is not ready'
  }
  $state = Invoke-RestMethod -Uri "$($descriptor.endpoint)/api/state"
  if (-not $state.project) { throw 'HTTP state did not come from the daemon runtime' }

  $initialize = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"rust-only-smoke","version":"1"}}}'
  $initialized = '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
  $toolCall = '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"batai_read_project_state","arguments":{}}}'
  $responses = (($initialize, $initialized, $toolCall) -join "`n" | & $binary mcp) | ForEach-Object { $_ | ConvertFrom-Json }
  $callResult = $responses | Where-Object { $_.id -eq 2 }
  if (-not $callResult.result.structuredContent.project) {
    throw 'MCP bridge did not read the shared daemon snapshot'
  }

  & $binary stop
  for ($attempt = 0; $attempt -lt 100 -and -not $daemon.HasExited; $attempt++) {
    Start-Sleep -Milliseconds 50
    $daemon.Refresh()
  }
  if (-not $daemon.HasExited) { throw 'Daemon did not stop gracefully' }
  Write-Output 'Rust daemon + HTTP + MCP smoke passed without Node.'
}
finally {
  if ($daemon -and -not $daemon.HasExited) {
    Stop-Process -Id $daemon.Id -Force
    Wait-Process -Id $daemon.Id -Timeout 5 -ErrorAction SilentlyContinue
  }
  if ($daemon) {
    $daemon.Dispose()
    Start-Sleep -Milliseconds 50
  }
  if (Test-Path -LiteralPath $smokeRoot) {
    Remove-Item -LiteralPath $smokeRoot -Recurse -Force
  }
}
