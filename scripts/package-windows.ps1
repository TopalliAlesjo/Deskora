[CmdletBinding()]
param(
    # Fail instead of skipping when Inno Setup (ISCC.exe) is not installed.
    [switch]$RequireInstaller
)
$ErrorActionPreference = 'Stop'

function Find-InnoCompiler {
    $command = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) { return $candidate }
    }
    return $null
}
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $projectRoot
try {
    # Cargo reports progress on stderr: Windows PowerShell 5.1 would turn it into a
    # terminating error under 'Stop', so only the exit code decides here.
    & {
        $ErrorActionPreference = 'Continue'
        cargo build --locked --release 2>&1 | ForEach-Object { "$_" } | Out-Host
    }
    if ($LASTEXITCODE -ne 0) { throw 'Deskora release build failed.' }
    $distRoot = Join-Path $projectRoot 'dist'
    New-Item -ItemType Directory -Force -Path $distRoot | Out-Null
    $stage = Join-Path $distRoot ('package-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Copy-Item -LiteralPath 'target/release/deskora.exe' -Destination (Join-Path $stage 'Deskora.exe')
        Copy-Item -LiteralPath 'LICENSE', 'THIRD_PARTY_NOTICES.md' -Destination $stage
        Copy-Item -LiteralPath 'licenses' -Destination $stage -Recurse
        @'
Deskora - Windows VNC viewer

1. Avvia Deskora.exe.
2. Inserisci l'indirizzo del computer remoto e la password VNC, se richiesta.
3. Premi Connetti. Sul computer remoto deve essere attivo un server VNC.

Porta iniziale: 5900, modificabile in Opzioni.
Ctrl+Alt+Maiusc+F: schermo intero.
VNC classico non cifra il traffico: usa una rete fidata o una VPN.
Le preferenze vengono salvate in %APPDATA%\Deskora, senza la password.
Per la modalita' portabile crea un file preferences.json vuoto accanto a Deskora.exe.
'@ | Set-Content -LiteralPath (Join-Path $stage 'LEGGIMI.txt') -Encoding utf8
        # Cargo packages may preserve Unix-epoch dates, outside ZIP's date range.
        $zipMinimumDate = [datetime]::new(1980, 1, 1)
        Get-ChildItem -LiteralPath $stage -Recurse -File | ForEach-Object {
            if ($_.LastWriteTime -lt $zipMinimumDate) { $_.LastWriteTime = $zipMinimumDate }
        }
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $distRoot 'Deskora-Windows.zip') -Force
        Copy-Item -LiteralPath (Join-Path $stage 'Deskora.exe') -Destination (Join-Path $distRoot 'Deskora.exe') -Force

        $iscc = Find-InnoCompiler
        if ($iscc) {
            $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
            if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo metadata.' }
            $version = ($metadata.packages | Where-Object name -eq 'deskora' | Select-Object -First 1).version
            & $iscc '/Qp' "/DAppVersion=$version" "/DStageDir=$stage" (Join-Path $projectRoot 'installer/deskora.iss')
            if ($LASTEXITCODE -ne 0) { throw 'Deskora installer build failed.' }
        } elseif ($RequireInstaller) {
            throw 'Inno Setup 6 (ISCC.exe) is required to build Deskora-Installer.exe.'
        } else {
            Write-Warning 'Inno Setup 6 not found: Deskora-Installer.exe was not built.'
        }
    } finally {
        $resolvedStage = [IO.Path]::GetFullPath($stage)
        $allowedPrefix = [IO.Path]::GetFullPath($distRoot) + [IO.Path]::DirectorySeparatorChar
        if (!$resolvedStage.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Package staging directory must stay inside dist.'
        }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
    Get-ChildItem -LiteralPath $distRoot -File | Select-Object Name, Length
} finally {
    Pop-Location
}
