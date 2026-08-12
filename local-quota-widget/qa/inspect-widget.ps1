param(
    [Parameter(Mandatory = $true)]
    [int]$ProcessId,
    [Parameter(Mandatory = $true)]
    [string]$ScreenshotPath
)

$nativeCode = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class WidgetQaNative {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    public static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int index);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out Rect rect);

    public struct Rect {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static IntPtr[] WindowsForProcess(uint targetProcessId) {
        var result = new List<IntPtr>();
        EnumWindows((hWnd, _) => {
            uint processId;
            GetWindowThreadProcessId(hWnd, out processId);
            if (processId == targetProcessId) {
                result.Add(hWnd);
            }
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }
}
'@

Add-Type -TypeDefinition $nativeCode
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$deadline = [DateTime]::UtcNow.AddSeconds(12)
$windowHandle = [IntPtr]::Zero
do {
    $visibleHandles = @([WidgetQaNative]::WindowsForProcess([uint32]$ProcessId) |
        Where-Object { [WidgetQaNative]::IsWindowVisible($_) } |
        Select-Object -First 1)
    $windowHandle = if ($visibleHandles.Count -gt 0) {
        [IntPtr]$visibleHandles[0]
    } else {
        [IntPtr]::Zero
    }
    if ($windowHandle -eq [IntPtr]::Zero) {
        Start-Sleep -Milliseconds 200
    }
} while ($windowHandle -eq [IntPtr]::Zero -and [DateTime]::UtcNow -lt $deadline)

if ($windowHandle -eq [IntPtr]::Zero) {
    throw "No visible window found for process $ProcessId"
}

[WidgetQaNative+Rect]$rect = [WidgetQaNative+Rect]::new()
[WidgetQaNative]::GetWindowRect([IntPtr]$windowHandle, [ref]$rect) | Out-Null
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
$extendedStyle = [WidgetQaNative]::GetWindowLongPtr([IntPtr]$windowHandle, -20).ToInt64()

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen(
        $rect.Left,
        $rect.Top,
        0,
        0,
        (New-Object System.Drawing.Size $width, $height)
    )
    $bitmap.Save($ScreenshotPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

$automationNames = @()
try {
    $condition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ProcessIdProperty,
        $ProcessId
    )
    $root = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
        [System.Windows.Automation.TreeScope]::Descendants,
        $condition
    )
    if ($null -ne $root) {
        $elements = $root.FindAll(
            [System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.Condition]::TrueCondition
        )
        foreach ($element in $elements) {
            $name = $element.Current.Name
            if (-not [string]::IsNullOrWhiteSpace($name)) {
                $automationNames += $name
            }
        }
    }
} catch {
    $automationNames += "UIAutomation unavailable: $($_.Exception.Message)"
}

[pscustomobject]@{
    ProcessId = $ProcessId
    Handle = ('0x{0:X}' -f $windowHandle.ToInt64())
    Visible = [WidgetQaNative]::IsWindowVisible($windowHandle)
    X = $rect.Left
    Y = $rect.Top
    Width = $width
    Height = $height
    ExtendedStyle = ('0x{0:X}' -f $extendedStyle)
    ToolWindow = [bool]($extendedStyle -band 0x80)
    AppWindow = [bool]($extendedStyle -band 0x40000)
    Screenshot = $ScreenshotPath
    AutomationNames = ($automationNames | Select-Object -Unique) -join ' | '
} | ConvertTo-Json -Depth 3
