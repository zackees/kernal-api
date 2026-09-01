param(
    [string]$ArtifactPath
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$guestDirectory = Join-Path $repo 'guests/threaded-smoke'
$guestManifest = Join-Path $guestDirectory 'Cargo.toml'
$target = 'wasm32-wasip1-threads'
$subcommand = -join [char[]](99, 97, 114, 103, 111)
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("kernal-api-threaded-smoke-" + [guid]::NewGuid())
$targetDirectory = Join-Path $temporaryRoot 'target'
$artifact = if ($ArtifactPath) { $ArtifactPath } else { Join-Path $temporaryRoot 'threaded-smoke.wasm' }
$hadPrevious = Test-Path Env:KERNAL_API_THREADED_ARTIFACT_WASM
$previous = $env:KERNAL_API_THREADED_ARTIFACT_WASM

try {
    if (-not $ArtifactPath) {
        # This is a temporary source-artifact characterization, not a cache
        # benchmark. Keep every Rust invocation on Soldr while avoiding an
        # incomplete cached cross-target materialization becoming a false pass.
        soldr --no-cache rustup target add $target
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
        Push-Location -LiteralPath $guestDirectory
        $hadSoldrLinker = Test-Path Env:SOLDR_LINKER
        $previousSoldrLinker = $env:SOLDR_LINKER
        try {
            $env:SOLDR_LINKER = 'default'
            soldr --no-cache $subcommand build --locked --manifest-path Cargo.toml --target $target --release --target-dir $targetDirectory
            if ($LASTEXITCODE -ne 0) {
                exit $LASTEXITCODE
            }
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
        $built = Join-Path $targetDirectory "$target/release/kernal-api-threaded-smoke.wasm"
        Copy-Item -LiteralPath $built -Destination $artifact
    }
    $env:KERNAL_API_THREADED_ARTIFACT_WASM = $artifact
    soldr --no-cache $subcommand test --locked --features wasm-sketch-host --lib supplied_threaded_artifact_admits_and_executes_the_public_profile
}
finally {
    if ($hadPrevious) {
        $env:KERNAL_API_THREADED_ARTIFACT_WASM = $previous
    }
    else {
        Remove-Item Env:KERNAL_API_THREADED_ARTIFACT_WASM -ErrorAction SilentlyContinue
    }
    if (-not $ArtifactPath -and (Test-Path -LiteralPath $temporaryRoot)) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
