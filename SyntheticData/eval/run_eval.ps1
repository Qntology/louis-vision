param(
    [ValidateSet("diag", "holdout")]
    [string]$Split = "diag",
    [int]$CasesPerDomain = 16,
    [string]$Domains = "ALL",
    [switch]$ReleaseSeal,
    [switch]$SkipDump
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path "$PSScriptRoot\..\..").Path
$SynthDir = Join-Path $RepoRoot "SyntheticData"

if ($Split -eq "holdout" -and -not $ReleaseSeal) {
    Write-Error "ERROR: -Split holdout requires -ReleaseSeal (Rule 7 holdout guard)."
    exit 2
}

$freeBytes = (Get-PSDrive C).Free
if ($freeBytes -lt 10GB) {
    Write-Error "ERROR: Insufficient free disk space on C:\ ($([math]::Round($freeBytes / 1GB, 2)) GB < 10 GB Rule 1 floor)."
    exit 2
}

$env:CARGO_BUILD_JOBS = "1"
$env:CARGO_PROFILE_DEV_DEBUG = "0"

# Clean previous generated artifacts without touching eval scripts or README
foreach ($sub in @("domains", "relational")) {
    $p = Join-Path $SynthDir $sub
    if (Test-Path $p) {
        Remove-Item -Recurse -Force $p
    }
}
$manifest = Join-Path $SynthDir "manifest.json"
if (Test-Path $manifest) {
    Remove-Item -Force $manifest
}

$cliArgs = @(
    "run",
    "--manifest-path", (Join-Path $RepoRoot "apps\louis\Cargo.toml"),
    "-j", "1",
    "--",
    "--eval-nn-search",
    "--split", $Split,
    "--cases-per-domain", "$CasesPerDomain",
    "--domains", $Domains,
    "--out-dir", $SynthDir,
    "--layout", "structured"
)
if ($ReleaseSeal) {
    $cliArgs += "--release-seal"
}
if (-not $SkipDump) {
    $cliArgs += "--dump-synth"
}

Write-Host "[LOUIS-SYNTH] Running serial N:N corpus generation + search eval ($Split, cases=$CasesPerDomain)..."
& cargo @cliArgs | Out-Null
if ($LASTEXITCODE -ne 0) {
    Write-Error "[LOUIS-SYNTH] louis-docs --eval-nn-search failed with exit $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "[LOUIS-SYNTH] Summarizing report and checking SyntheticData/eval/gates.toml..."
& python (Join-Path $PSScriptRoot "summarize_report.py")
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host "[LOUIS-SYNTH] PASS: SyntheticData updated at $SynthDir"
exit 0
