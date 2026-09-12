# PowerShell 7 counterpart of build-guest.sh. The caller owns build storage.
$ErrorActionPreference = 'Stop'
$repoDirectory = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$guestDirectory = Join-Path $PSScriptRoot 'guest'
$target = 'wasm32-wasip1-threads'

if (-not $env:CARGO_TARGET_DIR -or -not [System.IO.Path]::IsPathFullyQualified($env:CARGO_TARGET_DIR)) {
    throw 'CARGO_TARGET_DIR must name absolute caller-managed writable storage'
}
$guestTargetDirectory = Join-Path $env:CARGO_TARGET_DIR 'kernal-api-wasm-tauri-guest'
$hadSoldrLinker = Test-Path Env:SOLDR_LINKER
$previousSoldrLinker = $env:SOLDR_LINKER
Push-Location -LiteralPath $guestDirectory
try {
    $env:SOLDR_LINKER = 'default'
    & soldr --no-cache cargo build --locked --manifest-path Cargo.toml --target $target --release --target-dir $guestTargetDirectory
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
