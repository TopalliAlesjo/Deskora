# Technical PNG/ICO export of the supplied artwork; no redrawing or recoloring.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$assetRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../assets'))
Add-Type -AssemblyName System.Drawing
$drawingReferences = @('System.Drawing.Common', 'System.Drawing.Primitives')
foreach ($assemblyName in @('System.Private.Windows.GdiPlus', 'System.Private.Windows.Core')) {
    if (Test-Path (Join-Path $PSHOME "$assemblyName.dll")) { $drawingReferences += $assemblyName }
}
Add-Type -ReferencedAssemblies $drawingReferences -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
public static class DeskoraAssetExport {
    static Rectangle Bounds(Bitmap source) {
        int left = source.Width, top = source.Height, right = -1, bottom = -1;
        for (int y = 0; y < source.Height; y++)
            for (int x = 0; x < source.Width; x++)
                if (source.GetPixel(x, y).A >= 16) {
                    left = Math.Min(left, x); top = Math.Min(top, y);
                    right = Math.Max(right, x); bottom = Math.Max(bottom, y);
                }
        if (right < left) throw new InvalidDataException("Empty artwork");
        return Rectangle.FromLTRB(Math.Max(0, left-2), Math.Max(0, top-2),
            Math.Min(source.Width, right+3), Math.Min(source.Height, bottom+3));
    }
    static Bitmap Render(Bitmap source, Rectangle crop, int width, int height) {
        var result = new Bitmap(width, height, PixelFormat.Format32bppArgb);
        using (var graphics = Graphics.FromImage(result)) {
            graphics.Clear(Color.Transparent);
            graphics.CompositingMode = CompositingMode.SourceCopy;
            graphics.InterpolationMode = InterpolationMode.HighQualityBicubic;
            graphics.PixelOffsetMode = PixelOffsetMode.HighQuality;
            double scale = Math.Min((double)width / crop.Width, (double)height / crop.Height);
            int w = Math.Max(1, (int)Math.Round(crop.Width * scale));
            int h = Math.Max(1, (int)Math.Round(crop.Height * scale));
            graphics.DrawImage(source, new Rectangle((width-w)/2, (height-h)/2, w, h), crop, GraphicsUnit.Pixel);
        }
        return result;
    }
    public static void Export(string root) {
        using (var banner = new Bitmap(Path.Combine(root, "source/deskora-banner.png"))) {
            var crop = Bounds(banner);
            int width = Math.Min(1200, crop.Width);
            int height = (int)Math.Round((double)width * crop.Height / crop.Width);
            using (var result = Render(banner, crop, width, height))
                result.Save(Path.Combine(root, "deskora-logo.png"), ImageFormat.Png);
        }
        using (var source = new Bitmap(Path.Combine(root, "source/deskora-icon.png"))) {
            var crop = Bounds(source);
            using (var result = Render(source, crop, 256, 256))
                result.Save(Path.Combine(root, "deskora-icon.png"), ImageFormat.Png);
            int[] sizes = {16, 20, 24, 32, 40, 48, 64, 128, 256};
            byte[][] images = new byte[sizes.Length][];
            for (int i = 0; i < sizes.Length; i++)
                using (var result = Render(source, crop, sizes[i], sizes[i]))
                using (var buffer = new MemoryStream()) {
                    result.Save(buffer, ImageFormat.Png);
                    images[i] = buffer.ToArray();
                }
            using (var writer = new BinaryWriter(File.Create(Path.Combine(root, "deskora.ico")))) {
                writer.Write((ushort)0); writer.Write((ushort)1); writer.Write((ushort)sizes.Length);
                int offset = 6 + 16 * sizes.Length;
                for (int i = 0; i < sizes.Length; i++) {
                    writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                    writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                    writer.Write((byte)0); writer.Write((byte)0);
                    writer.Write((ushort)1); writer.Write((ushort)32);
                    writer.Write(images[i].Length); writer.Write(offset);
                    offset += images[i].Length;
                }
                foreach (var image in images) writer.Write(image);
            }
        }
    }
}
'@
[DeskoraAssetExport]::Export($assetRoot)
Get-ChildItem -LiteralPath $assetRoot -File | Select-Object Name, Length
