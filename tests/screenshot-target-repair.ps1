# Exercise the builder's exact repair block without touching a real toolchain.
$ErrorActionPreference = 'Stop'
$builder = Join-Path $PSScriptRoot '../examples/wasm-tauri-screenshot/build-guest.ps1'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($builder, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$probe = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Test-ScreenshotTargetLibraries' }, $true)
$repair = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.IfStatementAst] -and $node.Extent.Text.StartsWith('if (-not (Test-ScreenshotTargetLibraries))') }, $true)
if (-not $probe -or -not $repair) { throw 'Builder repair block not found' }
. ([scriptblock]::Create($probe.Extent.Text))
$repairBlock = [scriptblock]::Create($repair.Extent.Text)
$target = 'wasm32-wasip1-threads'
$mockRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'mock-sysroot'))

function soldr {
    $command = $args -join ' '
    $script:calls.Add($command)
    $global:LASTEXITCODE = 0
    if ($command -match 'rustc --print target-libdir') { return (Join-Path $mockRoot 'lib/rustlib/wasm32-wasip1-threads/lib') }
    if ($command -match 'rustc --print sysroot') { return $mockRoot }
    if ($command -match 'rustup target remove') {
        if ($script:mode -eq 'remove-fails') { $global:LASTEXITCODE = 1 }
        $script:removed = $true
        return
    }
    if ($command -match 'rustup target add') {
        if ($script:mode -eq 'install' -or ($script:removed -and $script:mode -ne 'still-missing')) { $script:materialized = $true }
        return
    }
    throw "Unexpected tool invocation: $command"
}
function Get-ChildItem {
    param($LiteralPath, $Filter, $ErrorAction)
    if ($script:materialized -or ($script:mode -eq 'core-only' -and $Filter -eq 'libcore-*.rlib')) { 'mock-library.rlib' }
}
function Test-Path { param($LiteralPath) return $script:manifestExists }
function New-Item {
    param($ItemType, $Path, $ErrorAction)
    if ($ItemType -ne 'File' -or $Path -ne (Join-Path $mockRoot "lib/rustlib/manifest-rust-std-$target")) { throw 'Unexpected write target' }
    $script:createdManifest = $true
}

foreach ($case in @('healthy', 'install', 'stale', 'existing-manifest', 'core-only', 'remove-fails', 'still-missing')) {
    $script:mode = $case
    $script:materialized = $case -eq 'healthy'
    $script:manifestExists = $case -eq 'existing-manifest'
    $script:createdManifest = $false
    $script:removed = $false
    $script:calls = [System.Collections.Generic.List[string]]::new()
    $failed = $false
    try { & $repairBlock } catch { $failed = $true }
    $expectedFailure = $case -in @('remove-fails', 'still-missing')
    if ($failed -ne $expectedFailure) { throw "Unexpected outcome for $case" }
    $repairExpected = $case -notin @('healthy', 'install')
    if ($script:removed -ne $repairExpected) { throw "Unexpected removal for $case" }
    if ($script:createdManifest -ne ($repairExpected -and -not $script:manifestExists)) { throw "Unexpected manifest mutation for $case" }
    if ($case -eq 'healthy' -and $script:calls.Count -ne 1) { throw 'Healthy toolchain was mutated' }
    if ($case -eq 'remove-fails' -and @($script:calls | Where-Object { $_ -match 'target add' }).Count -ne 1) { throw 'Continued after failed removal' }
    Write-Output "PASS $case"
}
