if (-not (Test-Path .git)) { throw "Not in root directory"}
try {
    $ErrorActionPreference = "Stop"
    $commithash = git log -1 --pretty=format:%h
    Get-Process -Name pashari -ErrorAction SilentlyContinue | Stop-Process
    cargo build --release
    if ($LASTEXITCODE -eq 0) { iscc /DMyAppVersion=1.0.5-$commithash installer\pashari.iss }
    & ((Get-ChildItem dist_installer | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName)
} catch {
    throw
} finally {
    $ErrorActionPreference = "Continue"
}
