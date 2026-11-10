<#
.SYNOPSIS
    Offline Windows 10 (1703+)/11 probe for the host's top-level window region.
.DESCRIPTION
    Opens only its own synthetic window. Compare Normal, Region and Clear;
    Forced DWM is an experiment, not a proposed production fix. Closing prints
    JSON observations. No viewer, grid, account files or screenshots are opened.
    Run in a fresh STA PowerShell process. CompileOnly checks the embedded C#
    without loading WinForms or invoking Windows APIs, including on Linux.
#>
param([switch]$CompileOnly)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $CompileOnly -and [Environment]::OSVersion.Platform -ne 'Win32NT') {
    throw 'The interactive probe requires Windows 10 (1703+)/11. Use -CompileOnly for a source check.'
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace Foldurora {
    public static class HostFrameProbe {
        [StructLayout(LayoutKind.Sequential)]
        public struct Rect { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)]
        public struct Point { public int X, Y; }

        [DllImport("user32.dll")] static extern int GetWindowRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] static extern int GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] static extern int ClientToScreen(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr hwnd, int index);
        [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
        [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr hwnd, IntPtr region);
        [DllImport("user32.dll")] static extern int SetWindowRgn(IntPtr hwnd, IntPtr region, int redraw);
        [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int left, int top, int right, int bottom);
        [DllImport("gdi32.dll")] static extern int CombineRgn(IntPtr dest, IntPtr first, IntPtr second, int mode);
        [DllImport("gdi32.dll")] static extern int DeleteObject(IntPtr value);
        [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attr, out int value, uint size);
        [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr hwnd, uint attr, ref int value, uint size);

        public sealed class Observation {
            public string DwmQueryHResult;
            public bool? NcRenderingEnabled;
            public int RegionType; // 0: no application region OR query error; 1/2/3: null/simple/complex.
            public string Style, ExStyle;
            public uint Dpi;
            public Rect Outer, ClientOnDesktop;
        }

        static void Geometry(IntPtr hwnd, out Rect outer, out Rect client) {
            Point origin = new Point();
            if (GetWindowRect(hwnd, out outer) == 0 || GetClientRect(hwnd, out client) == 0 ||
                ClientToScreen(hwnd, ref origin) == 0)
                throw new InvalidOperationException("Could not read probe geometry.");
            client.Right += origin.X; client.Bottom += origin.Y;
            client.Left = origin.X; client.Top = origin.Y;
        }

        public static void Clear(IntPtr hwnd) {
            if (SetWindowRgn(hwnd, IntPtr.Zero, 1) == 0)
                throw new InvalidOperationException("Could not clear probe region.");
        }

        public static string Policy(IntPtr hwnd, bool force) {
            int policy = force ? 2 : 0; // DWMNCRP_ENABLED / DWMNCRP_USEWINDOWSTYLE.
            int hr = DwmSetWindowAttribute(hwnd, 2, ref policy, 4); // DWMWA_NCRENDERING_POLICY.
            return "0x" + unchecked((uint)hr).ToString("X8");
        }

        public static void Hole(IntPtr hwnd, int stripBottom) {
            Rect outer, client;
            Geometry(hwnd, out outer, out client);
            int left = client.Left - outer.Left + 16;
            int top = client.Top - outer.Top + stripBottom + 16;
            int right = client.Right - outer.Left - 16;
            int bottom = client.Bottom - outer.Top - 16;
            if (right <= left || bottom <= top)
                throw new InvalidOperationException("Probe client is too small for a hole.");
            IntPtr full = CreateRectRgn(0, 0, outer.Right - outer.Left, outer.Bottom - outer.Top);
            IntPtr cut = CreateRectRgn(left, top, right, bottom);
            try {
                if (full == IntPtr.Zero || cut == IntPtr.Zero || CombineRgn(full, full, cut, 4) == 0)
                    throw new InvalidOperationException("Could not allocate/combine probe region.");
                if (SetWindowRgn(hwnd, full, 1) == 0)
                    throw new InvalidOperationException("Could not apply probe region.");
                full = IntPtr.Zero; // Windows now owns the HRGN. Never delete it after success.
            } finally {
                if (cut != IntPtr.Zero) DeleteObject(cut);
                if (full != IntPtr.Zero) DeleteObject(full);
            }
        }

        public static Observation Snapshot(IntPtr hwnd) {
            Observation result = new Observation();
            Geometry(hwnd, out result.Outer, out result.ClientOnDesktop);
            int enabled;
            int hr = DwmGetWindowAttribute(hwnd, 1, out enabled, 4); // DWMWA_NCRENDERING_ENABLED, BOOL.
            result.DwmQueryHResult = "0x" + unchecked((uint)hr).ToString("X8");
            result.NcRenderingEnabled = hr >= 0 ? (bool?)(enabled != 0) : null;
            result.Dpi = GetDpiForWindow(hwnd);
            result.Style = "0x" + unchecked((uint)GetWindowLongW(hwnd, -16)).ToString("X8");
            result.ExStyle = "0x" + unchecked((uint)GetWindowLongW(hwnd, -20)).ToString("X8");
            IntPtr region = CreateRectRgn(0, 0, 0, 0);
            if (region == IntPtr.Zero) throw new InvalidOperationException("Could not allocate query region.");
            try { result.RegionType = GetWindowRgn(hwnd, region); }
            finally { DeleteObject(region); }
            return result;
        }
    }
}
'@

if ($CompileOnly) {
    Write-Output 'Embedded C# compiled; Windows APIs and interactive probe were not run.'
    return
}
if ([Threading.Thread]::CurrentThread.GetApartmentState() -ne 'STA') {
    throw 'Run powershell.exe -NoProfile -STA -File scripts/probe-host-frame.ps1 in a fresh process.'
}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

$script:probeRecords = New-Object 'System.Collections.Generic.List[object]'
$script:probeFailure = $null
$script:probeBusy = $false
$script:probeMode = 'Normal'
$script:probePolicyResult = $null
$probeForm = New-Object System.Windows.Forms.Form
$probeTimer = New-Object System.Windows.Forms.Timer
$probeTimer.Interval = 200
$previousDpi = [Foldurora.HostFrameProbe]::SetThreadDpiAwarenessContext([IntPtr](-4))
if ($previousDpi -eq [IntPtr]::Zero) { throw 'Could not enable per-monitor-v2 DPI for the probe thread.' }

function Save-ProbeObservation([string]$moment) {
    $script:probeRecords.Add([pscustomobject]@{
        Phase = $script:probeMode
        Moment = $moment
        PolicyHResult = $script:probePolicyResult
        Native = [Foldurora.HostFrameProbe]::Snapshot($probeForm.Handle)
    })
}

function Set-ProbeMode([string]$mode) {
    if ($script:probeBusy) { return }
    $script:probeBusy = $true
    $probeTimer.Stop()
    try {
        $script:probeMode = $mode
        [Foldurora.HostFrameProbe]::Clear($probeForm.Handle)
        $script:probePolicyResult = [Foldurora.HostFrameProbe]::Policy($probeForm.Handle, $mode -eq 'Forced DWM')
        if ($mode -eq 'Region' -or $mode -eq 'Forced DWM') {
            [Foldurora.HostFrameProbe]::Hole($probeForm.Handle, $probeStrip.Bottom)
        }
        $probeForm.Text = "Foldurora native frame probe - $mode"
        Save-ProbeObservation 'immediate'
        $probeTimer.Start()
    } catch {
        $script:probeFailure = $_
        $probeForm.Close()
    } finally { $script:probeBusy = $false }
}

try {
    $probeForm.AutoScaleMode = 'None'
    $probeForm.ClientSize = New-Object System.Drawing.Size(880, 560)
    $probeForm.MinimumSize = New-Object System.Drawing.Size(700, 400)
    $probeForm.StartPosition = 'CenterScreen'
    $probeStrip = New-Object System.Windows.Forms.FlowLayoutPanel
    $probeStrip.Dock = 'Top'
    $probeStrip.Height = 120
    foreach ($mode in @('Normal', 'Region', 'Clear', 'Forced DWM')) {
        $button = New-Object System.Windows.Forms.Button
        $button.Text = $mode
        $button.AutoSize = $true
        $button.Add_Click({ param($sender, $eventArgs) Set-ProbeMode $sender.Text })
        [void]$probeStrip.Controls.Add($button)
    }
    $help = New-Object System.Windows.Forms.Label
    $help.AutoSize = $true
    $help.Text = "Compare Normal -> Region -> Clear. Forced DWM is experimental.`r`nThe hole exposes your desktop; no screenshot is taken. Moving/resizing clears it.`r`nClose this window to return JSON observations. Inspect native caption/buttons/border."
    $probeStrip.SetFlowBreak($button, $true)
    [void]$probeStrip.Controls.Add($help)
    [void]$probeForm.Controls.Add($probeStrip)
    $probeForm.Add_Shown({ Set-ProbeMode 'Normal' })
    $probeForm.Add_ResizeBegin({ Set-ProbeMode 'Clear' })
    $probeForm.Add_Resize({
        if ($script:probeMode -eq 'Region' -or $script:probeMode -eq 'Forced DWM') { Set-ProbeMode 'Clear' }
    })
    $probeTimer.Add_Tick({
        $probeTimer.Stop()
        try { Save-ProbeObservation 'after-200ms-message-loop' }
        catch { $script:probeFailure = $_; $probeForm.Close() }
    })
    [void]$probeForm.ShowDialog()
} finally {
    $probeTimer.Dispose()
    $probeForm.Dispose() # Destroy only the probe HWND; Windows releases any installed HRGN.
    [void][Foldurora.HostFrameProbe]::SetThreadDpiAwarenessContext($previousDpi)
}
[pscustomobject]@{
    Probe = 'Standalone Win32 region probe; not the production viewer'
    WindowsVersion = [Environment]::OSVersion.VersionString
    PowerShellVersion = $PSVersionTable.PSVersion.ToString()
    Observations = $script:probeRecords.ToArray()
} | ConvertTo-Json -Depth 6
if ($script:probeFailure) { throw $script:probeFailure }
