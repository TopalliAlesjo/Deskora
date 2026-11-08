# Deskora assets

`source/deskora-banner.png` and `source/deskora-icon.png` are the original
transparent PNGs supplied by the project owner, copied without modification.

`deskora-logo.png`, `deskora-icon.png` and the multi-resolution `deskora.ico`
are technical exports used by the app. The Windows resource file embeds the
icon and product name in the executable. PNGs used by the UI are embedded at
compile time; end users do not need to copy the assets directory.

To regenerate the exports on Windows with PowerShell 7:

```powershell
./scripts/prepare-brand-assets.ps1
```

The exporter trims near-transparent outer margins and scales the artwork while
preserving aspect ratio and alpha. No redrawing or recoloring is performed.
ICO sizes: 16, 20, 24, 32, 40, 48, 64, 128 and 256 pixels.
