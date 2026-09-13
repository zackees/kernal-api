# PowerShell 7 counterpart of build-guest.sh. The caller owns build storage.
param([switch]$TrapAfterCapture, [switch]$BlockAfterCapture, [switch]$PrepareTargetOnly)
$ErrorActionPreference = 'Stop'
$repoDirectory = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$guestDirectory = Join-Path $PSScriptRoot 'guest'
$target = 'wasm32-wasip1-threads'

if (-not $env:CARGO_TARGET_DIR -or -not [System.IO.Path]::IsPathFullyQualified($env:CARGO_TARGET_DIR)) {
    throw 'CARGO_TARGET_DIR must name absolute caller-managed writable storage'
}
$guestTargetDirectory = Join-Path $env:CARGO_TARGET_DIR 'kernal-api-wasm-tauri-guest'
$guestFeatures = @()
if ($TrapAfterCapture -and $BlockAfterCapture) { throw 'choose only one post-capture fault' }
if ($TrapAfterCapture) {
    $guestTargetDirectory += '-trap'
    $guestFeatures = @('--features', 'proof-trap-after-capture')
}
if ($BlockAfterCapture) {
    $guestTargetDirectory += '-block'
    $guestFeatures = @('--features', 'proof-block-after-capture')
}
$hadSoldrLinker = Test-Path Env:SOLDR_LINKER
$previousSoldrLinker = $env:SOLDR_LINKER
Push-Location -LiteralPath $guestDirectory
try {
    $env:SOLDR_LINKER = 'default'
    # Restored toolchain caches can retain rustup component bookkeeping while
    # omitting target libraries. Verify actual files in the guest toolchain.
    function Test-ScreenshotTargetLibraries {
        $libdir = (& soldr --no-cache rustc --print target-libdir --target $target).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Cannot locate guest target libraries' }
        if (-not [System.IO.Path]::IsPathFullyQualified($libdir)) { throw 'Guest target libdir is not absolute' }
        return (@(Get-ChildItem -LiteralPath $libdir -Filter 'libcore-*.rlib' -ErrorAction SilentlyContinue).Count -gt 0 -and
            @(Get-ChildItem -LiteralPath $libdir -Filter 'libstd-*.rlib' -ErrorAction SilentlyContinue).Count -gt 0)
    }
    if (-not (Test-ScreenshotTargetLibraries)) {
        & soldr --no-cache rustup target add $target
        if ($LASTEXITCODE -ne 0) { throw 'Guest target installation failed' }
        if (-not (Test-ScreenshotTargetLibraries)) {
            $sysroot = (& soldr --no-cache rustc --print sysroot).Trim()
            if ($LASTEXITCODE -ne 0 -or -not [System.IO.Path]::IsPathFullyQualified($sysroot)) { throw 'Cannot locate guest sysroot' }
            $manifest = Join-Path $sysroot "lib/rustlib/manifest-rust-std-$target"
            # Preserve a real uninstall manifest. Only repair missing bookkeeping
            # for this exact, already-proven-incomplete target, as in threaded-smoke.
            if (-not (Test-Path -LiteralPath $manifest)) {
                New-Item -ItemType File -Path $manifest -ErrorAction Stop | Out-Null
            }
            & soldr --no-cache rustup target remove $target
            if ($LASTEXITCODE -ne 0) { throw 'Incomplete guest target removal failed' }
            & soldr --no-cache rustup target add $target
            if ($LASTEXITCODE -ne 0) { throw 'Guest target reinstallation failed' }
            if (-not (Test-ScreenshotTargetLibraries)) { throw 'Guest target still lacks core/std libraries after reinstall' }
        }
    }
    # Reuse the exact target repair for other guests without building a screenshot.
    # Returning through finally restores the caller's directory and linker setting.
    if ($PrepareTargetOnly) { return }
    & soldr --no-cache cargo build --locked --manifest-path Cargo.toml --target $target --release --target-dir $guestTargetDirectory @guestFeatures
    if ($LASTEXITCODE -ne 0) { throw "guest build failed with exit code $LASTEXITCODE" }
}
finally {
    if ($hadSoldrLinker) {
        $env:SOLDR_LINKER = $previousSoldrLinker
    }
    else {
        Remove-Item Env:SOLDR_LINKER -ErrorAction SilentlyContinue
    }
    Pop-Location
}

$built = Join-Path $guestTargetDirectory "$target/release/kernal-api-wasm-tauri-guest.wasm"
$admitted = Join-Path $guestTargetDirectory "$target/release/kernal-api-wasm-tauri-guest.admitted.wasm"
Copy-Item -LiteralPath $built -Destination $admitted -Force
& soldr cargo run --locked --manifest-path (Join-Path $repoDirectory 'tools/wasm-abi-generator/Cargo.toml') -- --embed-threaded-metadata $admitted
if ($LASTEXITCODE -ne 0) { throw "guest metadata embedding failed with exit code $LASTEXITCODE" }
Write-Output $admitted
