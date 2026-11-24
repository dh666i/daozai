$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

public sealed class DaozaiSnip : Form {
    [DllImport("user32.dll")]
    private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")]
    private static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("dwmapi.dll")]
    private static extern int DwmFlush();

    private Point start, end;
    private bool selecting;
    private Rectangle selected;
    private readonly Rectangle desktop;
    private readonly System.Windows.Forms.Timer timeout = new System.Windows.Forms.Timer();

    private DaozaiSnip() {
        desktop = SystemInformation.VirtualScreen;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        AutoScaleMode = AutoScaleMode.None;
        Bounds = desktop;
        ShowInTaskbar = false;
        TopMost = true;
        Opacity = 0.30;
        BackColor = Color.Black;
        Cursor = Cursors.Cross;
        DoubleBuffered = true;
        timeout.Interval = 60000;
        timeout.Tick += delegate { Close(); };
    }
    protected override void OnShown(EventArgs e) {
        base.OnShown(e);
        SetWindowPos(Handle, new IntPtr(-1), desktop.X, desktop.Y, desktop.Width, desktop.Height, 0x0040);
        Activate();
        timeout.Start();
    }
    protected override void OnKeyDown(KeyEventArgs e) {
        if (e.KeyCode == Keys.Escape) Close();
        base.OnKeyDown(e);
    }
    protected override void OnMouseDown(MouseEventArgs e) {
        if (e.Button == MouseButtons.Right) { Close(); return; }
        if (e.Button != MouseButtons.Left) return;
        start = end = e.Location;
        selecting = Capture = true;
    }
    protected override void OnMouseMove(MouseEventArgs e) {
        if (selecting) { end = e.Location; Invalidate(); }
    }
    private Rectangle Selection() {
        return Rectangle.FromLTRB(Math.Min(start.X, end.X), Math.Min(start.Y, end.Y),
            Math.Max(start.X, end.X), Math.Max(start.Y, end.Y));
    }
    protected override void OnMouseUp(MouseEventArgs e) {
        if (!selecting || e.Button != MouseButtons.Left) return;
        end = e.Location;
        selecting = Capture = false;
        selected = Rectangle.Intersect(desktop, RectangleToScreen(Selection()));
        Close();
    }
    protected override void OnPaint(PaintEventArgs e) {
        base.OnPaint(e);
        if (selecting) {
            var rect = Selection();
            e.Graphics.FillRectangle(Brushes.LightSkyBlue, rect);
            e.Graphics.DrawRectangle(Pens.White, rect);
        }
    }
    protected override void Dispose(bool disposing) {
        if (disposing) timeout.Dispose();
        base.Dispose(disposing);
    }
    public static int Run(bool copy) {
        // The same thread selects and captures in physical screen coordinates.
        SetThreadDpiAwarenessContext(new IntPtr(-4));
        Rectangle rect;
        using (var window = new DaozaiSnip()) {
            window.ShowDialog();
            rect = window.selected;
        }
        if (rect.Width < 2 || rect.Height < 2) return 2;
        DwmFlush();
        using (var image = new Bitmap(rect.Width, rect.Height, PixelFormat.Format24bppRgb)) {
            using (var graphics = Graphics.FromImage(image)) {
                graphics.CopyFromScreen(rect.Location, Point.Empty, rect.Size, CopyPixelOperation.SourceCopy);
            }
            if (copy) {
                // Persist the pixels after this STA process exits.
                var data = new DataObject();
                data.SetImage(image);
                Clipboard.SetDataObject(data, true, 10, 100);
            } else {
                using (var png = new MemoryStream()) {
                    image.Save(png, ImageFormat.Png);
                    using (var output = Console.OpenStandardOutput()) png.WriteTo(output);
                }
            }
        }
        return 0;
    }
}
'@
try {
    exit [DaozaiSnip]::Run($env:DAOZAI_SNIP_MODE -eq 'image')
} catch {
    [Console]::Error.WriteLine($_.Exception.GetBaseException().Message)
    exit 1
}
