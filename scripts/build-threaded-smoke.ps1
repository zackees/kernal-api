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

# `--print target-libdir` computes the path whether or not the target is
# installed, so this is a question about files rather than about rustup's
# opinion of them.
function Get-GuestTargetLibDir {
    param([string]$Target)
    (soldr --no-cache rustc --print target-libdir --target $Target).Trim()
}

function Test-GuestTargetMaterialized {
    param([string]$Target)
    $libDir = Get-GuestTargetLibDir $Target
    if (-not $libDir) { return $false }
    [bool](Get-ChildItem -LiteralPath $libDir -Filter 'libcore-*.rlib' -ErrorAction SilentlyContinue)
}

try {
    if (-not $ArtifactPath) {
        # This is a temporary source-artifact characterization, not a cache
        # benchmark. Keep every Rust invocation on Soldr while avoiding an
        # incomplete cached cross-target materialization becoming a false pass.
        #
        # A restored CI toolchain cache can leave rustup's `components` list
        # naming this target while its `manifest-rust-std-<target>` file is
        # gone. rustup then trusts the list: `target add` answers "up to date"
        # and installs nothing, `target remove` cannot read the manifest and
        # rolls back, and `toolchain install --force` reports "up to date" as
        # well. The build fails afterwards as "E0463: can't find crate for
        # core" against the guest's own dependencies, which reads like a guest
        # problem and is not one.
        #
        # Verify the materialization rather than the bookkeeping, and repair by
        # making the bookkeeping true: an empty manifest is enough for `remove`
        # to succeed, after which `add` really downloads. The manifest is only
        # touched once the target's own libdir is already proven missing, so a
        # healthy toolchain is never disturbed.
        soldr --no-cache rustup target add $target
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
        if (-not (Test-GuestTargetMaterialized $target)) {
            $sysroot = (soldr --no-cache rustc --print sysroot).Trim()
            $manifest = Join-Path $sysroot "lib/rustlib/manifest-rust-std-$target"
            Set-Content -LiteralPath $manifest -Value $null -NoNewline
            soldr --no-cache rustup target remove $target
            $global:LASTEXITCODE = 0
            soldr --no-cache rustup target add $target
            if ($LASTEXITCODE -ne 0) {
                exit $LASTEXITCODE
            }
            if (-not (Test-GuestTargetMaterialized $target)) {
                Write-Error "$target has no libcore in $(Get-GuestTargetLibDir $target) after reinstall"
                exit 1
            }
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
        soldr cargo run --locked --manifest-path (Join-Path $repo 'tools/wasm-abi-generator/Cargo.toml') -- --embed-threaded-metadata $artifact
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }
    $env:KERNAL_API_THREADED_ARTIFACT_WASM = $artifact
    soldr --no-cache $subcommand test --locked --features wasm-sketch-host --lib supplied_threaded_artifact_admits_and_executes_the_public_profile
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    soldr --no-cache $subcommand test --locked --features wasm-sketch-worker --test wasm_worker_containment cargo_built_threaded_guest_ -- --ignored --test-threads=1
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
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
