# Loads MainWindow.xaml + Theme.xaml in a plain WPF host (custom controls swapped for same-size placeholders), fills
# in the README demo texts, shows it at the system DPI and prints every named element's pixel rectangle.
param([string]$Src, [string]$Out, [string]$View = 'extended')
[AppContext]::SetSwitch('Switch.MS.Internal.DoNotApplyLayoutRoundingToMarginsAndBorderThickness', $false)
Add-Type -AssemblyName PresentationCore, PresentationFramework, WindowsBase, System.Xaml
$app = New-Object System.Windows.Application
$theme = [System.Windows.Markup.XamlReader]::Parse((Get-Content "$Src\UI\Theme.xaml" -Raw))
$app.Resources.MergedDictionaries.Add($theme)
$x = Get-Content "$Src\UI\MainWindow.xaml" -Raw
$x = $x -replace 'x:Class="[^"]*"', ''
$x = $x -replace '\s(Click|Checked|Unchecked|TextChanged)="[^"]*"', ''
$x = $x -replace '<ui:PathText ', '<FrameworkElement ' -replace '<ui:SpeedGraph ', '<FrameworkElement ' -replace '<ui:Donut ', '<FrameworkElement '
$x = $x -replace 'xmlns:ui="clr-namespace:Choppy.UI"', ''
$w = [System.Windows.Markup.XamlReader]::Parse($x)
function N($n) { $w.FindName($n) }
$ext = $View -eq 'extended'
(N 'FromName').Text = 'D: 4TB - Shadow'; (N 'ToName').Text = 'X: 4TB - uTorrent'
(N 'AppIcon').Source = [System.Windows.Media.Imaging.BitmapFrame]::Create([Uri]'file:///C:/Users/GolaN/AppData/Local/Temp/claude/C--GolaN-stuff---RunAs-Scripts--Claude-Choppy--claude-worktrees-rust-f3d6bd/8ade6f77-d10c-4772-a0f8-bcf5fba8e53e/scratchpad/ref/frame0.png'); (N 'FromPath').Height = 20; (N 'ToPath').Height = 20
(N 'BadgeMode').Text = 'Adaptive'; (N 'BadgeDetail').Text = '1 stream'
(N 'PercentExt').Text = '46%'; (N 'PercentCompact').Text = '27%'
(N 'SizeDone').Text = '1.6 GB'; (N 'SizeOf').Text = ' of '; (N 'SizeTotal').Text = '3.4 GB'
(N 'FilesText').Text = '2 of 6 files'; (N 'ElapsedVal').Text = '29 s'; (N 'RemainLabel').Text = '    Remaining '; (N 'RemainVal').Text = '33 s'
(N 'CNow').Text = '58.8 MB/s'; (N 'CAvgLabel').Text = '    AVG  '; (N 'CAvg').Text = '57.1 MB/s'
(N 'C2Label').Text = 'Copying  '; (N 'C2Name').Text = 'Drone - coastline 4K.mp4'
(N 'SpAvgNum').Text = '57.0'; (N 'SpAvgUnit').Text = ' MB/s'; (N 'SpNum').Text = '57.5'; (N 'SpUnit').Text = ' MB/s'
(N 'CurDone').Text = '1.0 GB'; (N 'CurOf').Text = ' of '; (N 'CurTotal').Text = '1.1 GB'; (N 'CurName').Text = 'Drone - coastline 4K.mp4'
(N 'LimitBox').Text = '57'; (N 'TgLimit').IsChecked = $true; (N 'TgClose').IsChecked = $true
$vis = [System.Windows.Visibility]::Visible; $col = [System.Windows.Visibility]::Collapsed
foreach ($e in 'Badge','PercentExt','FilesText','ExtPanel','BtnSkip','BtnQueue') { (N $e).Visibility = $(if ($ext) { $vis } else { $col }) }
foreach ($e in 'PercentCompact','CompactSpeeds','CompactLine') { (N $e).Visibility = $(if ($ext) { $col } else { $vis }) }
[System.Windows.Controls.Grid]::SetColumnSpan((N 'BarTrack'), $(if ($ext) { 6 } else { 5 }))
[System.Windows.Controls.Grid]::SetColumnSpan((N 'Stats'), $(if ($ext) { 6 } else { 5 }))
(N 'ViewText').Text = $(if ($ext) { 'Compact' } else { 'Extended' })
# legend rows like MainWindow.AddLegend
$legend = N 'Legend'
foreach ($r in @(@('Wedding 2025 - full ceremony.mp4','1.5 GB'),@('Drone - coastline 4K.mp4','1.1 GB'),@('Birthday party.mp4','600 MB'),@('Road trip north - day 2.mov','250 MB'),@('Other 2 files','18 MB'))) {
  $g = New-Object System.Windows.Controls.Grid
  $g.Margin = New-Object System.Windows.Thickness(0, $(if ($legend.Children.Count -eq 0) { 0 } else { 6 }), 0, 0)
  foreach ($cw in 'Auto','*','Auto') { $cd = New-Object System.Windows.Controls.ColumnDefinition; $cd.Width = (New-Object System.Windows.GridLengthConverter).ConvertFromString($cw); $g.ColumnDefinitions.Add($cd) }
  $sq = New-Object System.Windows.Controls.Border; $sq.Width = 10; $sq.Height = 10; $sq.VerticalAlignment = 'Center'; $sq.Margin = New-Object System.Windows.Thickness(0,1,10,0)
  $nm = New-Object System.Windows.Controls.TextBlock; $nm.Text = $r[0]; $nm.FontSize = 14; [System.Windows.Controls.Grid]::SetColumn($nm, 1)
  $sz = New-Object System.Windows.Controls.TextBlock; $sz.Text = $r[1]; $sz.FontSize = 14; $sz.Margin = New-Object System.Windows.Thickness(14,0,0,0); [System.Windows.Controls.Grid]::SetColumn($sz, 2)
  $g.Children.Add($sq) | Out-Null; $g.Children.Add($nm) | Out-Null; $g.Children.Add($sz) | Out-Null
  $legend.Children.Add($g) | Out-Null
}
$w.WindowStartupLocation = 'Manual'; $w.Left = 100; $w.Top = 50; $w.ShowActivated = $false
$w.Show(); $w.UpdateLayout()
$d = [System.Windows.Media.VisualTreeHelper]::GetDpi($w); $m = [pscustomobject]@{ M11 = $d.DpiScaleX; M22 = $d.DpiScaleY }
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add(('window {0} x {1} (dip {2} x {3}) scale {4}' -f ($w.ActualWidth*$m.M11), ($w.ActualHeight*$m.M22), $w.ActualWidth, $w.ActualHeight, $m.M11))
function Dump($e, $name) {
  if ($e -isnot [System.Windows.FrameworkElement] -or -not $e.IsVisible) { return }
  $p = $e.TransformToAncestor($w).Transform((New-Object System.Windows.Point(0,0)))
  $lines.Add(('{0,-16} x={1,7:N2} y={2,7:N2} w={3,7:N2} h={4,7:N2}' -f $name, ($p.X*$m.M11), ($p.Y*$m.M22), ($e.ActualWidth*$m.M11), ($e.ActualHeight*$m.M22)))
}
function Walk($e, $depth) {
  $n = if ($e -is [System.Windows.FrameworkElement] -and $e.Name) { $e.Name } elseif ($e -is [System.Windows.Controls.TextBlock]) { 'TB:' + $e.Text } elseif ($e -is [System.Windows.Controls.Border]) { 'Border' } elseif ($e -is [System.Windows.Shapes.Shape]) { $e.GetType().Name } else { $null }
  if ($n) { Dump $e (('  ' * [Math]::Min($depth,6)) + $n) }
  for ($i = 0; $i -lt [System.Windows.Media.VisualTreeHelper]::GetChildrenCount($e); $i++) { Walk ([System.Windows.Media.VisualTreeHelper]::GetChild($e, $i)) ($depth + 1) }
}
Walk $w 0
$lines | Set-Content $Out
$w.Close()


