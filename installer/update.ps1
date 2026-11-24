param([Parameter(Mandatory=$true)][string]$WorkPath, [Parameter(Mandatory=$true)][string]$TargetExe, [int]$ParentProcessId)
$ErrorActionPreference = 'Stop'
# 更新只使用当前 PowerShell 自带模块，避免继承其他终端版本的模块路径。
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$cacheRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Daozai\Updates')).TrimEnd('\')
$workRoot = [IO.Path]::GetFullPath($WorkPath).TrimEnd('\')
$target = [IO.Path]::GetFullPath($TargetExe)
$targetDir = Split-Path $target -Parent
$stage = Join-Path $targetDir ('daozai-update-' + [Guid]::NewGuid().ToString('N'))
$changes = New-Object System.Collections.ArrayList
$stagedFiles = @()
$keepBackup = $false
$exited = $false
function Assert-PlainPath([string]$PathToCheck) {
    $ancestor = [IO.Path]::GetFullPath($PathToCheck)
    while ($ancestor) {
        if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '更新目录不能包含目录链接。' }
        $ancestor = Split-Path $ancestor -Parent
    }
}
try {
    if (-not $workRoot.StartsWith($cacheRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw '更新缓存目录无效。' }
    Assert-PlainPath $workRoot
    Assert-PlainPath $target
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) { throw '找不到当前程序。' }
    $version = (Get-Content -LiteralPath (Join-Path $workRoot 'version.txt') -Raw).Trim()
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw '更新版本号无效。' }
    $archive = Join-Path $workRoot 'update.zip'
    $expectedHash = (Get-Content -LiteralPath (Join-Path $workRoot 'hash.txt') -Raw).Trim()
    if ($expectedHash -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expectedHash) { throw '更新包校验失败。' }
    $owned = @(Get-Content -LiteralPath (Join-Path $workRoot 'update-files.txt'))
    if (!$owned.Count -or $owned -notcontains 'recorder/DaozaiRecorder.exe') { throw '缺少录屏组件清单。' }
    foreach ($rel in $owned) {
        if ($rel -cnotmatch '^(LICENSE|THIRD_PARTY_NOTICES\.txt|recorder/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)+)$') { throw '无效的组件路径。' }
    }
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        $prefix = "Daozai-v$version-windows-x64/"
        foreach ($rel in ($owned + @('daozai.exe'))) {
            $dest = if ($rel -ceq 'daozai.exe') { $target } else { Join-Path $targetDir $rel }
            Assert-PlainPath $dest
            if ([IO.Directory]::Exists($dest)) { throw "组件位置已被目录占用：$rel" }
            $entries = @($zip.Entries | Where-Object { $_.FullName.Replace('\','/') -ceq ($prefix + $rel) })
            if ($entries.Count -ne 1 -or $entries[0].Length -le 0 -or $entries[0].Length -gt 200MB) { throw "更新包缺少有效组件：$rel" }
            $item = [PSCustomObject]@{ Destination=$dest; New=(Join-Path $stage ("new-" + $stagedFiles.Count)); Backup=(Join-Path $stage ("old-" + $stagedFiles.Count)); WasPresent=[IO.File]::Exists($dest) }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0], $item.New)
            $stagedFiles += $item
        }
    } finally { $zip.Dispose() }
    $actual = [Diagnostics.FileVersionInfo]::GetVersionInfo($stagedFiles[-1].New).FileVersion
    if ($actual -ne $version -and $actual -ne "$version.0") { throw '安装包版本不匹配。' }
    Set-Content -LiteralPath (Join-Path $workRoot 'ready') -Value 'ok'
    if ($ParentProcessId -gt 0) {
        $parent = Get-Process -Id $ParentProcessId -ErrorAction SilentlyContinue
        if ($parent -and -not $parent.WaitForExit(60000)) { throw '岛仔尚未退出，请稍后重试。' }
    }
    $exited = $true
    # 暂存和回滚副本均在目标卷，主程序最后替换。
    foreach ($item in $stagedFiles) {
        Assert-PlainPath $item.Destination
        if ($item.WasPresent) {
            [IO.File]::Replace($item.New, $item.Destination, $item.Backup)
        } else {
            [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($item.Destination)) | Out-Null
            [IO.File]::Move($item.New, $item.Destination)
        }
        [void]$changes.Add($item)
    }
    Start-Process -FilePath $target -WorkingDirectory $targetDir -WindowStyle Hidden
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Daozai'
    $registered = Get-ItemProperty -LiteralPath $key -ErrorAction SilentlyContinue
    if ($registered -and [string]::Equals($registered.InstallLocation.TrimEnd('\'), $targetDir, [StringComparison]::OrdinalIgnoreCase)) {
        Set-ItemProperty -LiteralPath $key -Name DisplayVersion -Value $version -ErrorAction SilentlyContinue
        $publisher = if ([Globalization.CultureInfo]::CurrentUICulture.TwoLetterISOLanguageName -eq 'zh') { '迟迟' } else { 'chichi' }
        Set-ItemProperty -LiteralPath $key -Name Publisher -Value $publisher -ErrorAction SilentlyContinue
        # 只统计岛仔自己的文件，软件管家不会把共享安装文件夹的大小算到岛仔头上。
        $own = @('daozai.exe','Uninstall.exe') + $owned | ForEach-Object { Join-Path $targetDir $_ } | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
        $sizeKb = [int][Math]::Ceiling((($own | ForEach-Object { (Get-Item -LiteralPath $_).Length }) | Measure-Object -Sum).Sum / 1KB)
        if ($sizeKb -gt 0) { Set-ItemProperty -LiteralPath $key -Name EstimatedSize -Value $sizeKb -Type DWord -ErrorAction SilentlyContinue }
        # 开始菜单「收集岛仔诊断」：旧版安装没有，或指向已不再附带的 cmd；统一改为调用程序内嵌的诊断收集。失败不影响更新。
        try {
            $link = Join-Path ([Environment]::GetFolderPath('Programs')) '收集岛仔诊断.lnk'
            $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($link)
            $shortcut.TargetPath = $target; $shortcut.Arguments = '--collect-diagnostics'
            $shortcut.WorkingDirectory = $targetDir; $shortcut.IconLocation = "$target,0"; $shortcut.Save()
        } catch { }
        foreach ($legacy in @('收集岛仔诊断.cmd', 'scripts\collect-diagnostics.ps1')) {
            try { $path = Join-Path $targetDir $legacy; if (Test-Path -LiteralPath $path -PathType Leaf) { Assert-PlainPath $path; Remove-Item -LiteralPath $path -Force } } catch { }
        }
        try { $scripts = Join-Path $targetDir 'scripts'; if ((Test-Path -LiteralPath $scripts) -and -not (Get-ChildItem -LiteralPath $scripts -Force)) { Remove-Item -LiteralPath $scripts -Force } } catch { }
    }
    # 只清理已经确认位于专用缓存内的本次更新目录。
    if ($workRoot.StartsWith($cacheRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        try { Assert-PlainPath $workRoot; Remove-Item -LiteralPath $workRoot -Recurse -Force } catch { }
    }
} catch {
    $message = $_.Exception.Message
    for ($i = $changes.Count - 1; $i -ge 0; $i--) {
        $item = $changes[$i]
        try {
            if ($item.WasPresent) { [IO.File]::Replace($item.Backup, $item.Destination, [NullString]::Value) }
            else { [IO.File]::Delete($item.Destination) }
        } catch { $keepBackup = $true; $message += " 回滚失败，备份保存在 $stage。" }
    }
    if ($workRoot.StartsWith($cacheRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        [IO.File]::WriteAllText((Join-Path $workRoot 'error.txt'), $message)
    }
    if (-not $exited -or $ParentProcessId -le 0) { exit 1 }
    if (!$keepBackup) { Start-Process -FilePath $target -WorkingDirectory $targetDir -WindowStyle Hidden }
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show("更新未完成：$message", '岛仔更新') | Out-Null
    exit 1
} finally {
    if (!$keepBackup -and [IO.Directory]::Exists($stage) -and [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($stage)) -eq $targetDir) {
        Assert-PlainPath $stage
        Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
    }
}
