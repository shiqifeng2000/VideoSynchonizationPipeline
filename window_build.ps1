# powershell -ExecutionPolicy Bypass -File .\window_build.ps1

# Get the directory where the script is located
$cur_dir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $cur_dir

Write-Host "scp in" -ForegroundColor Cyan
# Note: Ensure OpenSSH Client is installed on Windows for scp to work
scp robin@10.10.87.239:/data/workspace/src.zip "$cur_dir\"

Write-Host "unzipping" -ForegroundColor Cyan
# -Force mimics the -o (overwrite) flag
Expand-Archive -Path "$cur_dir\src.zip" -DestinationPath "$cur_dir" -Force

# Run Cargo build
cargo build --release

# Ensure destination directory exists
$dst_dir = Join-Path $cur_dir "output"
if (!(Test-Path $dst_dir)) {
    New-Item -ItemType Directory -Path $dst_dir
}

# Copy compiled executables
Copy-Item "$cur_dir\target\release\*glue*" -Destination "$dst_dir\window"

# Change to destination and create tarball
Set-Location $dst_dir\window
Write-Host "Creating tar archive..." -ForegroundColor Cyan
# PowerShell 5.1/7+ can use tar directly if Windows 10/11
tar -cvf glue.tar *

Write-Host "scp out" -ForegroundColor Cyan
scp glue.tar robin@10.10.87.239:/data/workspace/

# Return to original directory
Set-Location $cur_dir
