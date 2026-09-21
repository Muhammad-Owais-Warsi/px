$ErrorActionPreference = "Stop"

$repo = "Muhammad-Owais-Warsi/px"
$asset = "px-windows-x86_64.zip"
$bin = "px.exe"
$name = "px"

function Add-ToPath($dir) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -split ";" | Where-Object { $_ -eq $dir }) { return }
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
    $env:Path = "$env:Path;$dir"
    Write-Host "Added $dir to PATH"
}

# Resolve install directory.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if ($env:Path -split ";" | Where-Object { $_ -eq $cargoBin }) {
    $destDir = $cargoBin
} else {
    $destDir = Join-Path $env:LOCALAPPDATA "px\bin"
    if (!(Test-Path $destDir)) { New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
    Add-ToPath $destDir
}

# Resolve download URL.
if ($args -match "^v?\d+\.\d+\.\d+$") {
    $tag = if ($args -match "^v") { $args } else { "v$args" }
    $url = "https://github.com/$repo/releases/download/$tag/$asset"
} else {
    $url = "https://github.com/$repo/releases/latest/download/$asset"
}

# Download and extract.
$tmp = Join-Path $env:TEMP "px-install-$(Get-Random)"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$zip = Join-Path $tmp $asset
$hashUrl = "$url.sha256"

Write-Host "Downloading px..."
try {
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
} catch {
    Write-Host "px-install: failed to download $url"
    exit 1
}

# Verify hash if available.
try {
    $expected = (Invoke-WebRequest -Uri $hashUrl -UseBasicParsing).Content.Trim().Split(" ")[0]
    $actual = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) {
        Write-Host "px-install: hash mismatch (expected $expected, got $actual)"
        exit 1
    }
} catch {
    # No hash file — proceed.
}

Expand-Archive -Path $zip -DestinationPath $tmp -Force
$exe = Join-Path $tmp $bin
if (!(Test-Path $exe)) {
    Write-Host "px-install: $bin not found in archive"
    exit 1
}

# Install.
$installDir = if ($env:PATH -split ";" | Where-Object { $_ -eq "$env:USERPROFILE\.cargo\bin" }) {
    "$env:USERPROFILE\.cargo\bin"
} else {
    $d = Join-Path $env:LOCALAPPDATA "px\bin"
    New-Item -ItemType Directory -Force -Path $d | Out-Null
    $d
}

Copy-Item -Force $exe (Join-Path $installDir $bin)
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

if ($installDir -ne "$env:USERPROFILE\.cargo\bin") {
    Add-ToPath $installDir
}

# Verify.
& "$installDir\$bin" --version
Write-Host "px installed to $installDir"
