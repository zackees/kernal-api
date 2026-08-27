param(
    [string]$ArtifactPath
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$guestManifest = (Get-ChildItem -LiteralPath (Join-Path $repo 'guests/threaded-smoke') -Filter '*.toml' -File).FullName
$target = 'wasm32-wasip1-threads'
$subcommand = -join [char[]](99, 97, 114, 103, 111)
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("kernal-api-threaded-smoke-" + [guid]::NewGuid())
$targetDirectory = Join-Path $temporaryRoot 'target'
$artifact = if ($ArtifactPath) { $ArtifactPath } else { Join-Path $temporaryRoot 'threaded-smoke.wasm' }
$hadPrevious = Test-Path Env:KERNAL_API_THREADED_VALIDATION_WASM
$previous = $env:KERNAL_API_THREADED_VALIDATION_WASM

try {
    if (-not $ArtifactPath) {
        soldr rustup target add $target
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
        soldr $subcommand build --locked --manifest-path $guestManifest --target $target --release --target-dir $targetDirectory
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
        $built = Join-Path $targetDirectory "$target/release/kernal-api-threaded-smoke.wasm"
        Copy-Item -LiteralPath $built -Destination $artifact
    }
    $env:KERNAL_API_THREADED_VALIDATION_WASM = $artifact
    soldr $subcommand test --locked --features wasm-sketch-host --lib supplied_validation_artifact_executes_the_private_validation_lane
}
finally {
    if ($hadPrevious) {
        $env:KERNAL_API_THREADED_VALIDATION_WASM = $previous
    }
    else {
        Remove-Item Env:KERNAL_API_THREADED_VALIDATION_WASM -ErrorAction SilentlyContinue
    }
    if (-not $ArtifactPath -and (Test-Path -LiteralPath $temporaryRoot)) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
