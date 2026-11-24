$ErrorActionPreference = 'Stop'
$repo = 'Ray-D-Song/lexe'
$arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
  'X64' { 'x64' }
  'Arm64' { 'arm64' }
  default { throw "Unsupported architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)" }
}
$installDir = if ($env:LEXE_INSTALL_DIR) { $env:LEXE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Lexe' }
$installDir = [IO.Path]::GetFullPath($installDir)
$asset = "lexe-windows-$arch.exe"
$base = "https://github.com/$repo/releases/latest/download"
$target = Join-Path $installDir 'lexe.exe'

if (Test-Path -LiteralPath $target) {
  $item = Get-Item -Force -LiteralPath $target
  if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or -not ((& $target --version 2>$null) -match '^lexe v')) {
    throw "Refusing to replace non-Lexe file: $target"
  }
}
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$staged = Join-Path $installDir ".lexe.$PID.exe"
$sums = Join-Path $installDir ".lexe.$PID.SHA256SUMS"
try {
  Invoke-WebRequest "$base/$asset" -OutFile $staged
  Invoke-WebRequest "$base/SHA256SUMS" -OutFile $sums
  $expected = ((Get-Content -LiteralPath $sums | Where-Object { $_ -match "  $([regex]::Escape($asset))$" }) -split '\s+')[0]
  if (-not $expected -or (Get-FileHash -Algorithm SHA256 -LiteralPath $staged).Hash -ne $expected.ToUpperInvariant()) { throw 'SHA256 verification failed' }
  Move-Item -Force -LiteralPath $staged -Destination $target
} finally {
  Remove-Item -Force -ErrorAction SilentlyContinue -LiteralPath $staged, $sums
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';' | Where-Object { $_ -eq $installDir }).Count -eq 0) {
  [Environment]::SetEnvironmentVariable('Path', (($userPath.TrimEnd(';') + ';' + $installDir).TrimStart(';')), 'User')
}
if (($env:Path -split ';' | Where-Object { $_ -eq $installDir }).Count -eq 0) { $env:Path = "$env:Path;$installDir" }
Write-Host "Installed Lexe to $target. Open a new PowerShell session to use lexe."
