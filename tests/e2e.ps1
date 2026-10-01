#requires -Version 5.1
<#
.SYNOPSIS
    End-to-end check for Ekbasis (and legacy aion alias).

.DESCRIPTION
    Builds the Ekbasis CLI, creates a throw-away git repository from `examples/rust-project`,
    installs Ekbasis into it, runs an experiment end to end (branch, change, build, test,
    benchmark, compare, store) and verifies the recorded artifacts.

.EXAMPLE
    pwsh -File tests/e2e.ps1
#>
param(
    # Keep the sandbox directory for manual inspection.
    [switch]$KeepSandbox,
    # Keep the experiment worktrees instead of deleting them.
    [switch]$KeepWorktrees,
    # Reuse a specific sandbox directory.
    [string]$Sandbox
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$project = Join-Path $repoRoot 'Cargo.toml'
$example = Join-Path $repoRoot 'examples/rust-project'
$experimentName = 'bloom-startup-test'

if (-not $Sandbox) {
    $Sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ('ekbasis-e2e-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
}

function Write-Step([string]$Text) {
    Write-Host ''
    Write-Host "== $Text" -ForegroundColor Cyan
}

function Assert([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "check failed: $Message" }
    Write-Host "  [ok] $Message" -ForegroundColor Green
}

Write-Host "Ekbasis end-to-end check" -ForegroundColor White
Write-Host "repository : $repoRoot"
Write-Host "sandbox    : $Sandbox"

Write-Step 'building the Ekbasis CLI'
& cargo build --manifest-path $project -p ekbasis-cli 2>&1 | Out-Host
Assert ($LASTEXITCODE -eq 0) 'ekbasis-cli builds'

$cli = Join-Path $repoRoot 'target/debug/ekbasis.exe'
if (-not (Test-Path $cli)) { $cli = Join-Path $repoRoot 'target/debug/ekbasis' }
Assert (Test-Path $cli) "ekbasis binary exists ($cli)"

Write-Step 'creating the target repository'
New-Item -ItemType Directory -Force -Path $Sandbox | Out-Null
Copy-Item -Recurse -Force (Join-Path $example '*') $Sandbox
Push-Location $Sandbox
try {
    & git init -q
    & git add -A
    & git -c user.name=E2E -c user.email=e2e@localhost commit -qm 'initial bloom demo'
    & git branch -M main
    Assert ((& git rev-parse --abbrev-ref HEAD).Trim() -eq 'main') 'target repository is on main'

    Write-Step 'ekbasis init'
    & $cli init 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis init succeeded'
    $stateDir = if (Test-Path (Join-Path $Sandbox '.ekbasis')) { '.ekbasis' } else { '.aion' }
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/config.yaml")) "$stateDir/config.yaml written"
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/timeline.db")) "$stateDir/timeline.db created"

    Write-Step 'ekbasis experiment create'
    $specSource = Join-Path $Sandbox 'bloom-startup-test.yaml'
    & $cli experiment create $experimentName --from $specSource --description 'e2e check' 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'experiment created from the bundled spec'
    Remove-Item -Force $specSource
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/experiments/$experimentName.yaml")) "experiment spec stored under $stateDir/experiments"

    Write-Step 'ekbasis experiment show'
    & $cli experiment show $experimentName 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis experiment show succeeded'

    Write-Step 'ekbasis experiment run'
    $runArgs = @('experiment', 'run', $experimentName, '--iterations', '5', '--warmup', '1')
    if ($KeepWorktrees) { $runArgs += '--keep-worktrees' }
    $runOutput = & $cli @runArgs 2>&1
    $runOutput | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis experiment run succeeded'
    Assert (($runOutput -join "`n") -match 'create branch') 'pipeline created the experiment branch'
    Assert (($runOutput -join "`n") -match 'compare & store') 'pipeline compared and stored the result'

    Write-Step 'verifying the recorded state'
    $branchName = if (@(& git branch --list "ekbasis/$experimentName").Count -ge 1) { "ekbasis/$experimentName" } else { "aion/$experimentName" }
    & git diff --quiet 'main' $branchName
    Assert ($LASTEXITCODE -ne 0) 'the alternative timeline differs from main'
    Assert (@(& git branch --list $branchName).Count -ge 1) "branch $branchName exists"
    $changed = (& git diff --name-only 'main' $branchName) -join ' '
    Assert ($changed -match 'config.toml') 'the experiment changed config.toml'
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/results/$experimentName.experiment.json")) 'experiment report written'
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/results/$experimentName.baseline.json")) 'baseline report written'
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/results/$experimentName.comparison.json")) 'comparison written'

    if (-not $KeepWorktrees) {
        $worktrees = (& git worktree list) -join "`n"
        Assert (($worktrees -notmatch '\.(ekbasis|aion)[\\/]worktrees')) 'temporary worktrees were cleaned up'
    }

    Write-Step 'ekbasis compare'
    $compareOutput = & $cli compare "$experimentName`:baseline" $experimentName 2>&1
    $compareOutput | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis compare succeeded'
    $compareText = $compareOutput -join "`n"
    Assert ($compareText -match 'startup \(mean\)') 'comparison reports the measured startup metric'
    Assert ($compareText -match 'improved') 'comparison reports the improvement the change caused'

    Write-Step 'ekbasis experiment list'
    $listOutput = & $cli experiment list --all 2>&1
    $listOutput | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis experiment list succeeded'
    Assert (($listOutput -join "`n") -match [regex]::Escape($experimentName)) 'the experiment shows up in the list'

    Write-Step 'ekbasis timeline'
    & $cli timeline 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis timeline succeeded'

    Write-Step 'ekbasis doctor'
    $doctorText = (& $cli doctor 2>&1) -join "`n"
    & $cli doctor 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis doctor succeeded'
    Assert ($doctorText -match 'guard rails') 'doctor reports the guard rails'
    Assert ($doctorText -match 'network block') 'doctor reports network block capability'

    Write-Step 'guard rails'
    $showText = (& $cli experiment show $experimentName 2>&1) -join "`n"
    Assert ($LASTEXITCODE -eq 0) 'experiment show lists the limits'
    Assert ($showText -match 'processes:') 'the limits line reports the process limit'
    Assert ($showText -match 'network:') 'the limits line reports the network policy'

    # `network: block` must refuse to run where it cannot be enforced — never run connected.
    $specPath = Join-Path $Sandbox "$stateDir/experiments/$experimentName.yaml"
    $originalSpec = [System.IO.File]::ReadAllText($specPath)
    try {
        $blockedSpec = $originalSpec -replace 'network: allow', 'network: block'
        Assert ($blockedSpec -ne $originalSpec) 'the spec carried a network policy to flip'
        [System.IO.File]::WriteAllText($specPath, $blockedSpec)
        $blocked = (& $cli experiment run $experimentName --force --iterations 1 --warmup 0 2>&1) -join "`n"
        $onLinux = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Linux)
        if ($onLinux) {
            Assert (($LASTEXITCODE -eq 0) -or ($blocked -match 'cannot be enforced')) 'network block either runs isolated or refuses honestly'
        } else {
            Assert ($LASTEXITCODE -ne 0) 'network: block refuses to run where it cannot be enforced'
            Assert ($blocked -match 'cannot be enforced') 'the refusal names the reason'
            Assert ($blocked -notmatch 'create branch') 'the refusal happened before any branch or worktree was created'
        }
    } finally {
        [System.IO.File]::WriteAllText($specPath, $originalSpec)
    }

    Write-Step 'guards'
    $second = & $cli experiment run $experimentName 2>&1
    Assert ($LASTEXITCODE -ne 0) 're-running without --force is refused (the timeline already exists)'
    Assert (($second -join "`n") -match '--force') 'the refusal explains how to rebuild the timeline'
    $rerun = & $cli experiment run $experimentName --force --iterations 3 2>&1
    Assert ($LASTEXITCODE -eq 0) 're-running with --force rebuilds the timeline'
    Assert (($rerun -join "`n") -match 'create branch') 'the rebuilt timeline went through the pipeline again'

    Write-Step 'markdown report'
    $generated = Join-Path $Sandbox "$stateDir/results/$experimentName.report.md"
    Assert (Test-Path $generated) 'the run wrote a Markdown report next to the JSON results'
    $reportOut = Join-Path $Sandbox 'ekbasis-report.md'
    & $cli report $experimentName --format markdown --out $reportOut 2>&1 | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis report succeeded'
    $markdown = Get-Content $reportOut -Raw
    Assert ($markdown -match '\| metric \| baseline \|') 'the Markdown contains the metric table'
    Assert ($markdown -match 'Welch') 'the Markdown contains the statistical test'
    Assert ($markdown -match 'Reproducibility') 'the Markdown contains the reproducibility section'
    $comment = (& $cli report $experimentName --format comment 2>&1) -join "`n"
    Assert ($comment -match '<!-- (ekbasis|aion)-report -->') 'the comment body carries the sticky marker'

    Write-Step 'ekbasis experiment diff'
    $diffOutput = & $cli experiment diff "$experimentName`:baseline" $experimentName 2>&1
    $diffOutput | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis experiment diff succeeded'
    $diffText = $diffOutput -join "`n"
    Assert ($diffText -match 'specification') 'the diff reports specification differences'
    Assert ($diffText -match 'measured') 'the diff reports measured differences'
    Assert ($diffText -match 'changed files') 'the diff lists the changed files'

    Write-Step 'CI mode (--candidate) for a faster branch'
    & git checkout -q -b feature/faster-startup main
    (Get-Content (Join-Path $Sandbox 'config.toml')) -replace '^preload = true.*$', 'preload = false' |
        Set-Content (Join-Path $Sandbox 'config.toml')
    & git add -A
    & git -c user.name=E2E -c user.email=e2e@localhost commit -qm 'speed up startup'
    $ciOutput = & $cli experiment run $experimentName --base main --candidate feature/faster-startup --iterations 3 --warmup 0 2>&1
    $ciOutput | Out-Host
    Assert ($LASTEXITCODE -eq 0) 'ekbasis experiment run --candidate succeeded'
    $ciText = $ciOutput -join "`n"
    Assert ($ciText -match 'candidate ref') 'the pipeline reported the candidate ref'
    Assert ($ciText -match 'improved') 'the CI comparison found the measured improvement'
    $ciComment = (& $cli report $experimentName --format comment 2>&1) -join "`n"
    Assert ($ciComment -match 'Changed files') 'the CI report lists the files that changed'

    Write-Step 'regression gate (--fail-on-regression)'
    & git checkout -q -b feature/slower-startup main
    (Get-Content (Join-Path $Sandbox 'config.toml')) -replace '^init_ms = 60.*$', 'init_ms = 400' |
        Set-Content (Join-Path $Sandbox 'config.toml')
    & git add -A
    & git -c user.name=E2E -c user.email=e2e@localhost commit -qm 'make startup slower on purpose'
    $gate = & $cli experiment run $experimentName --base main --candidate feature/slower-startup --iterations 3 --warmup 0 --fail-on-regression 2>&1
    $gate | Out-Host
    Assert ($LASTEXITCODE -ne 0) '--fail-on-regression fails the run'
    Assert (($gate -join "`n") -match 'regressed') 'the failure names the regression'
    Assert (Test-Path (Join-Path $Sandbox "$stateDir/results/$experimentName.report.md")) 'the report was written before the gate failed'

    Write-Host ''
    Write-Host 'ALL CHECKS PASSED' -ForegroundColor Green
    Write-Host "sandbox: $Sandbox"
}
finally {
    Pop-Location
    if (-not $KeepSandbox) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Sandbox
    }
}
