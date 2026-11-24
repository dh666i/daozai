param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'target\recorder' }
$output = [IO.Directory]::CreateDirectory($OutputDirectory).FullName
$cache = [IO.Directory]::CreateDirectory((Join-Path $projectRoot 'target\recorder-dependencies')).FullName
$package = Join-Path $cache 'screenrecorderlib.7.0.1.nupkg'
$expected = '5032DB2821316391FE0322739D658BD8CE4EF059B02542E97B09FA1E47C04E84'
if (!(Test-Path -LiteralPath $package) -or (Get-FileHash -LiteralPath $package).Hash -ne $expected) {
    $temp = "$package.$([Guid]::NewGuid().ToString('N')).tmp"
    try {
        Invoke-WebRequest -UseBasicParsing -Uri 'https://api.nuget.org/v3-flatcontainer/screenrecorderlib/7.0.1/screenrecorderlib.7.0.1.nupkg' -OutFile $temp
        if ((Get-FileHash -LiteralPath $temp).Hash -ne $expected) { throw 'ScreenRecorderLib package checksum mismatch.' }
        Copy-Item -LiteralPath $temp -Destination $package -Force
    } finally {
        if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp }
    }
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($package)
$library = Join-Path $output 'ScreenRecorderLib.dll'
try {
    [IO.Compression.ZipFileExtensions]::ExtractToFile($zip.GetEntry('build/x64/ScreenRecorderLib.dll'), $library, $true)
    [IO.Compression.ZipFileExtensions]::ExtractToFile($zip.GetEntry('LICENSE'), (Join-Path $output 'ScreenRecorderLib-LICENSE.txt'), $true)
} finally { $zip.Dispose() }

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vsRoot) { throw 'Visual C++ build tools are required to package the redistributable DLLs.' }
$crt = Get-ChildItem -Path (Join-Path $vsRoot 'VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT') -Directory |
    Sort-Object { [version]$_.Parent.Parent.Name } -Descending | Select-Object -First 1
if (!$crt) { throw 'Visual C++ redistributable files are missing from the build tools.' }
foreach ($name in @('msvcp140.dll', 'concrt140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
    Copy-Item -LiteralPath (Join-Path $crt.FullName $name) -Destination $output -Force
}

$compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$exe = Join-Path $output 'DaozaiRecorder.exe'
& $compiler /nologo /target:exe /platform:x64 /optimize+ "/out:$exe" "/reference:$library" (Join-Path $projectRoot 'helpers\recorder\Program.cs')
if ($LASTEXITCODE -ne 0) { throw 'Failed to build DaozaiRecorder.exe.' }
Write-Output $exe
