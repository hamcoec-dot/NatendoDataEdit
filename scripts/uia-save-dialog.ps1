# Answers the native Save dialog of Natendo Data Edit through UI Automation (no key presses):
# waits for a top-level window of the app named "Export", types the file name into its
# "File name" box and presses Save. Writes the time of the press (ms since 1970) to a file
# so a test can time the export from there. Used by scripts/real-export-parts.cjs.
# Usage: pwsh -File scripts/uia-save-dialog.ps1 <full path to save as> <stamp file> [seconds to wait]
param([string]$Target, [string]$Stamp, [int]$Wait = 60)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$deadline = (Get-Date).AddSeconds($Wait)
$root = [System.Windows.Automation.AutomationElement]::RootElement
$pids = @()
while ((Get-Date) -lt $deadline) {
  $pids = @(Get-Process -Name natendo-data-edit -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  foreach ($p in $pids) {
    $cond = New-Object System.Windows.Automation.AndCondition(
      (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p)),
      (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)))
    # A dialog owned by the app's window shows up under that window in UI Automation.
    $cands = @()
    foreach ($top in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)) {
      $cands += $top
      foreach ($c in $top.FindAll([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)))) { $cands += $c }
    }
    foreach ($w in $cands) {
      if ($w.Current.Name -ne 'Export') { continue }
      $edit = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition(
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)),
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1001')))))
      $save = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.AndCondition(
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)),
        (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, '1')))))
      if ($edit -and $save) {
        $vp = $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
        $vp.SetValue($Target)
        Start-Sleep -Milliseconds 300
        [long]$ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Set-Content -Path $Stamp -Value $ms
        # The Save button is clicked with a button-click message to its own window (not a key press).
        Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);'
        $h = [System.IntPtr]$save.Current.NativeWindowHandle
        if ($h -ne [System.IntPtr]::Zero) { [void][W.U]::PostMessage($h, 0xF5, [System.IntPtr]::Zero, [System.IntPtr]::Zero) }
        else { $save.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke() }
        Start-Sleep -Seconds 4
        $after = @()
        foreach ($p2 in $pids) {
          $c2 = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p2)
          foreach ($t2 in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $c2)) {
            $after += ('"' + $t2.Current.Name + '"')
            foreach ($k in $t2.FindAll([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)))) { $after += ('  owned "' + $k.Current.Name + '"') }
          }
        }
        'saved; 4 s later the app has: ' + ($after -join ', ')
        exit 0
      }
    }
  }
  Start-Sleep -Milliseconds 200
}
$names = @()
foreach ($p in $pids) {
  $cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $p)
  foreach ($w in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)) { $names += ('"' + $w.Current.Name + '" (' + $w.Current.ControlType.ProgrammaticName + ')') }
}
'no dialog; windows of the app: ' + ($names -join ', ')
exit 1
