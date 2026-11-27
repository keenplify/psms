# Releases the version in Cargo.toml from Windows. This computer checks everything, builds and
# tests Windows (with the NSIS installer) and, when Docker and Git Bash are available, both Linux
# architectures, then uploads them to a draft release. GitHub Actions builds only what is left
# (macOS), signs every download and publishes. On a Mac, use scripts/release.sh.
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
$target = "x86_64-pc-windows-msvc"
foreach ($tool in "gh", "cargo", "python", "node", "git") {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Install $tool first." }
}
function Run($exe, [string[]]$arguments) {
  & $exe @arguments
  if ($LASTEXITCODE) { throw "$exe $($arguments -join ' ') failed" }
}
$version = (python -c "import tomllib;print(tomllib.load(open('Cargo.toml','rb'))['package']['version'])").Trim()
$tag = "v$version"
if ((git branch --show-current).Trim() -ne "main") { throw "Release from main." }
if (git status --porcelain) { throw "Commit or stash local changes first." }
Run git @("fetch", "-q", "origin", "main", "--tags")
git ls-remote --exit-code --tags origin "refs/tags/$tag" *> $null
if ($LASTEXITCODE -eq 0) { throw "$tag already exists. Raise the version in Cargo.toml and desktop-ui/Cargo.toml first." }

Write-Host "Checking $tag on this PC..."
Run cargo @("fmt", "--check")
Run cargo @("clippy", "--locked", "--all-targets", "--", "-D", "warnings")
Run cargo @("test", "--locked", "--target", $target)
Push-Location desktop-ui
Run cargo @("fmt", "--check"); Run cargo @("clippy", "--locked", "--all-targets", "--", "-D", "warnings"); Run cargo @("test", "--locked")
Pop-Location
Run cargo @("build", "--locked", "--release", "--target", $target)
Run cargo @("build", "--locked", "--release", "--manifest-path", "desktop-ui/Cargo.toml", "--target-dir", "target", "--target", $target)
$bin = (Resolve-Path "target/$target/release").Path
Run python @("tests/smoke.py", "$bin/psms.exe")
Run python @("tests/mcp.py", "$bin/psms.exe")

$compiler = "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
if (-not (Test-Path $compiler)) { throw "Install NSIS (for example: winget install NSIS.NSIS)." }
Run $compiler @("/V2", "/DBIN_DIR=$bin", "/DVERSION=$version", "packaging/windows/psms.nsi")

# The release job unpacks each bundle into binaries/papamo-sms-<target>, like a CI artifact.
$bundles = "target/release-bundles"
if (Test-Path $bundles) { Remove-Item -Recurse -Force $bundles }
$dir = New-Item -ItemType Directory -Force "$bundles/papamo-sms-$target"
Copy-Item "$bin/psms.exe", "$bin/psms-desktop.exe", "$bin/psms-windows-x64-setup.exe" $dir
$built = $target
$bash = Get-Command bash -ErrorAction SilentlyContinue
docker info *> $null
if ($LASTEXITCODE -eq 0 -and $bash) {
  foreach ($arch in "arm64", "x64") { Run bash @("scripts/build-linux.sh", $arch) }
  $built += ",aarch64-unknown-linux-gnu,x86_64-unknown-linux-gnu"
} else {
  Write-Host "Docker or Git Bash is missing, so Actions builds Linux too."
}
$uploads = @()
foreach ($folder in Get-ChildItem -Directory $bundles) {
  $archive = "$bundles/local-$($folder.Name).tar.gz"
  Run tar @("-czf", $archive, "-C", $bundles, $folder.Name)
  $uploads += $archive
}

Write-Host "Publishing ${tag}: built here: $built; the rest on Actions..."
Run git @("push", "-q", "origin", "main")
Run git @("tag", $tag)
Run git @("push", "-q", "origin", $tag)
Run gh @("release", "create", $tag, "--draft", "--verify-tag", "--generate-notes", "--title", "pSMS $tag")
Run gh (@("release", "upload", $tag) + $uploads)
Run gh @("workflow", "run", "build.yml", "--ref", "main", "-f", "tag=$tag", "-f", "skip=$built")
Start-Sleep 5
gh run list --workflow build.yml --limit 1
