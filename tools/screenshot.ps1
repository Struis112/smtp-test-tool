# Capture the smtp-test-tool-gui window into a PNG, in a requested
# theme.  Tries `PrintWindow` first (works regardless of overlapping
# windows when it works); falls back to `BitBlt`-from-screen after
# moving the window to a known position and bringing it to foreground.
#
# Why two paths.  egui renders via OpenGL (eframe + glow); some Windows
# DWM configurations return an empty buffer from `PrintWindow` for
# GL-backed windows.  The industry-standard workaround (used by OBS,
# Discord, etc.) is the `Windows.Graphics.Capture` API; we keep the
# tool dependency-light by using the older RedrawWindow + PrintWindow
# pair with a screen-region fallback rather than pulling a Rust crate
# in just for the screenshot harness.
#
# Usage:
#   tools/screenshot.ps1 -Theme dark  -OutPng docs/screenshots/gui-dark.png
#   tools/screenshot.ps1 -Theme light -OutPng docs/screenshots/gui-light.png

param(
    [Parameter(Mandatory = $true)] [ValidateSet('dark', 'light')] [string] $Theme,
    [Parameter(Mandatory = $true)] [string] $OutPng,
    [int] $WarmupSeconds = 6,
    # Locale code to force in the staged profile (en, nl, de, zh, ja, ko, ar, ...).
    # When set, the GUI runs OS font discovery for non-Latin scripts; bump
    # -WarmupSeconds to ~12 because load_system_fonts() is not free.
    [string] $Locale = ''
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

# Win32 P/Invoke surface we need.
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W32 {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);

    // PrintWindow with PW_RENDERFULLCONTENT (0x02) is supposed to read
    // pixels straight from the window's backing store, but on some
    // OpenGL-backed eframe builds it returns an empty (clear-colour)
    // buffer.  We retry after a forced RedrawWindow.
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);

    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(
        IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")] public static extern bool RedrawWindow(
        IntPtr hWnd, IntPtr lprcUpdate, IntPtr hrgnUpdate, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int nCmdShow);

    public const int  SW_SHOWNORMAL      = 1;
    public const int  SW_RESTORE         = 9;
    public const uint SWP_NOSIZE         = 0x0001;
    public const uint SWP_NOZORDER        = 0x0004;
    public const uint SWP_SHOWWINDOW     = 0x0040;
    public const uint RDW_INVALIDATE     = 0x0001;
    public const uint RDW_UPDATENOW      = 0x0100;
    public const uint RDW_FRAME          = 0x0400;
    public const uint RDW_ALLCHILDREN    = 0x0080;
}
"@

# ---- stage a deterministic config next to the exe ------------------------
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$exe      = Join-Path $repoRoot 'target\release\smtp-test-tool-gui.exe'
if (-not (Test-Path $exe)) {
    throw "exe not found: $exe (run 'cargo build --release --bin smtp-test-tool-gui')"
}
$cfgPath  = Join-Path (Split-Path -Parent $exe) 'smtp_test_tool.toml'
$backup   = $null
if (Test-Path $cfgPath) {
    $backup = "$cfgPath.shotbak"
    Move-Item $cfgPath $backup -Force
}
@"
active = "default"

[profiles.default]
user = "ops@contoso.com"
smtp_host = "smtp-mail.outlook.com"
smtp_port = 587
smtp_security = "starttls"
imap_host = "outlook.office365.com"
imap_port = 993
imap_security = "ssl"
pop_host = "outlook.office365.com"
pop_port = 995
pop_security = "ssl"
pop_enabled = false
theme = "$Theme"
"@ | Set-Content -Path $cfgPath -Encoding utf8

if ($Locale -ne '') {
    Add-Content -Path $cfgPath -Value "locale = `"$Locale`"" -Encoding utf8
}

# Wipe eframe's persistence so the window starts at our default size
# rather than whatever the user last dragged it to.
$ePersist = Join-Path $env:APPDATA 'SMTP Test Tool'
if (Test-Path $ePersist) { Remove-Item -Recurse -Force $ePersist }

# ---- launch + wait for window --------------------------------------------
$proc = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds $WarmupSeconds

try {
    $p = Get-Process -Id $proc.Id -ErrorAction Stop
    if ($p.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "process $($proc.Id) has no visible main window yet (raise -WarmupSeconds?)"
    }
    $hwnd = $p.MainWindowHandle

    # Pin position so the capture rectangle is predictable AND no other
    # window from the user's desktop accidentally overlaps the GUI.
    [void][W32]::ShowWindow($hwnd, [W32]::SW_RESTORE)
    [void][W32]::SetWindowPos($hwnd, [IntPtr]::Zero, 50, 50, 0, 0,
        [W32]::SWP_NOSIZE -bor [W32]::SWP_NOZORDER -bor [W32]::SWP_SHOWWINDOW)
    [void][W32]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 700

    # Force a paint round-trip BEFORE the capture.  This is the standard
    # workaround for the GL-backed-PrintWindow-returns-empty problem.
    [void][W32]::RedrawWindow($hwnd, [IntPtr]::Zero, [IntPtr]::Zero,
        [W32]::RDW_INVALIDATE -bor [W32]::RDW_UPDATENOW -bor
        [W32]::RDW_FRAME -bor [W32]::RDW_ALLCHILDREN)
    Start-Sleep -Milliseconds 400

    $r = New-Object W32+RECT
    [void][W32]::GetWindowRect($hwnd, [ref] $r)
    $w  = $r.Right  - $r.Left
    $ht = $r.Bottom - $r.Top
    if ($w -le 0 -or $ht -le 0) { throw "window has zero size" }

    # ---- attempt 1: PrintWindow with PW_RENDERFULLCONTENT ----------------
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g   = [System.Drawing.Graphics]::FromImage($bmp)
    $dc  = $g.GetHdc()
    $ok  = $false
    try {
        $ok = [W32]::PrintWindow($hwnd, $dc, 2)  # PW_RENDERFULLCONTENT
    } finally {
        $g.ReleaseHdc($dc)
        $g.Dispose()
    }

    # Sample 50 pixels in a diagonal stripe; if every one of them is the
    # exact same colour, PrintWindow gave us a clear-buffer frame and we
    # need to fall back to a screen-region capture.  This heuristic is
    # cheap and catches the empty-GL-surface case without false positives.
    $allSame = $true
    $sampleColor = $bmp.GetPixel(10, 10)
    for ($i = 1; $i -lt 50; $i++) {
        $x = [int]($w * $i / 60.0)
        $y = [int]($ht * $i / 60.0)
        if ($bmp.GetPixel($x, $y) -ne $sampleColor) { $allSame = $false; break }
    }

    if (-not $ok -or $allSame) {
        Write-Host "PrintWindow returned an empty surface; falling back to full-screen capture + crop."
        $bmp.Dispose()

        # Empirically (May 2026), narrow-rect CopyFromScreen also returns
        # blank for the eframe GL-backed window on Windows 11 + glow.
        # Capturing the WHOLE virtual screen and then cropping in software
        # is the reliable workaround - the DWM composites the GL surface
        # into the desktop bitmap which is read back fine in one go.
        $virt   = [System.Windows.Forms.SystemInformation]::VirtualScreen
        $screen = New-Object System.Drawing.Bitmap $virt.Width, $virt.Height
        $gs     = [System.Drawing.Graphics]::FromImage($screen)
        $gs.CopyFromScreen($virt.X, $virt.Y, 0, 0, $screen.Size)
        $gs.Dispose()

        # Crop to the window's screen-space rect (subtracting virtual
        # origin in case the user has a multi-monitor setup that puts
        # the primary monitor at a non-(0,0) origin).
        $srcRect = New-Object System.Drawing.Rectangle (
            ($r.Left - $virt.X), ($r.Top - $virt.Y), $w, $ht)
        $bmp = New-Object System.Drawing.Bitmap $w, $ht
        $g   = [System.Drawing.Graphics]::FromImage($bmp)
        $g.DrawImage($screen, (New-Object System.Drawing.Rectangle 0, 0, $w, $ht),
                     $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
        $g.Dispose()
        $screen.Dispose()
    }

    $outDir = Split-Path -Parent $OutPng
    if ($outDir -and -not (Test-Path $outDir)) {
        New-Item -ItemType Directory -Path $outDir | Out-Null
    }
    $bmp.Save($OutPng, [System.Drawing.Imaging.ImageFormat]::Png)
    Write-Host "captured $OutPng  ($w x $ht, theme=$Theme)"
} finally {
    try { Stop-Process -Id $proc.Id -Force } catch {}
    Remove-Item -Force $cfgPath -ErrorAction SilentlyContinue
    if ($backup -and (Test-Path $backup)) {
        Move-Item $backup $cfgPath -Force
    }
}
