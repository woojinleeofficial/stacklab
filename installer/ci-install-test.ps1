# Silent install -> self-check of the installed exe -> silent uninstall (spec 14.4).
$ErrorActionPreference = "Stop"
$setup = Get-ChildItem target\installer\StackLab-Setup-*.exe | Select-Object -First 1
if (-not $setup) { throw "installer not built" }
$app = Join-Path $env:LOCALAPPDATA "Programs\StackLab"
$data = Join-Path $env:RUNNER_TEMP "stacklab-data"

Write-Host "installing $($setup.Name)"
$p = Start-Process $setup.FullName -ArgumentList "/VERYSILENT","/SUPPRESSMSGBOXES","/NORESTART","/CURRENTUSER" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "install exit code $($p.ExitCode)" }
foreach ($f in "stacklab.exe","unins000.exe","docs\USER_GUIDE.txt","docs\THIRD_PARTY_LICENSES.txt") {
  if (-not (Test-Path (Join-Path $app $f))) { throw "missing after install: $f" }
}

$env:STACKLAB_DATA_DIR = $data
$p = Start-Process (Join-Path $app "stacklab.exe") -ArgumentList "--self-check" -Wait -PassThru
$report = Get-Content (Join-Path $data "self-check.txt") -ErrorAction SilentlyContinue
Write-Host "self-check: $report (exit $($p.ExitCode))"
if ($p.ExitCode -ne 0) { throw "self-check failed" }

$p = Start-Process (Join-Path $app "unins000.exe") -ArgumentList "/VERYSILENT","/SUPPRESSMSGBOXES","/NORESTART" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "uninstall exit code $($p.ExitCode)" }
# the uninstaller finishes removing itself shortly after exiting
for ($i = 0; $i -lt 30 -and (Test-Path (Join-Path $app "stacklab.exe")); $i++) { Start-Sleep -Milliseconds 500 }
if (Test-Path (Join-Path $app "stacklab.exe")) { throw "stacklab.exe still present after uninstall" }
if (-not (Test-Path (Join-Path $data "stacklab.db"))) { throw "user data must survive uninstall" }
Write-Host "install / self-check / uninstall OK"
