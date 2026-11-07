# Foldurora application artwork

The maintainer supplied `folder-anime-eyes-purple.svg` on 6 November 2026 for
Foldurora. `foldurora-logo.svg` preserves that file byte for byte. Its editable
vector geometry, original colors, aspect ratio and transparent background are
retained. No background plate or replacement drawing is added.

The existing Aurora artwork and upstream attribution remain in the repository;
this fork's active application references use the Foldurora assets below.
Executable names, desktop IDs, bundle resource destination names and viewer
channel/version rules are unchanged by this visual branding change.

| Asset | Use |
| --- | --- |
| `foldurora-logo.svg` | Source artwork and packaged Linux desktop icon |
| `foldurora-logo.png` | 720 × 720 application/login/loading/About/notification artwork, matching the previous logo canvas |
| `foldurora-mark.png` | 40 × 40 full-color menu mark; decoded without the Tabler alpha-mask conversion |
| `foldurora-window.png` | 64 × 64 startup/window icon for CPU tools and the initial main window |
| `foldurora.ico` | PNG-backed 16, 24, 32, 48, 64, 128 and 256 pixel Windows resource/shortcut icons |
| `foldurora.icns` | PNG-backed 16 through 1024 pixel macOS icons, including standard retina representations |
| `foldurora-branding.json` | Source/asset SHA-256 values, dimensions and rasterizer versions |

The native window setup caches only the small decoded RGBA buffer. It creates
platform icon handles for each window and retains an explicitly supplied icon.
The existing asynchronous large-logo decode later supplies the main viewer's
same-artwork icon, including after GPU-window recovery. No world renderer or
network connection is needed for startup icon loading.

To regenerate, use Python 3 with installed librsvg, cairo, GLib and GObject
shared libraries:

```sh
python3 scripts/generate-foldurora-branding.py
```

This is a developer asset tool; builds consume the committed files and do not
need Python or a vector converter. The tool accepts only bounded, self-contained
SVG artwork, renders directly at each size, and writes ICO/ICNS containers with
those PNG frames. It does not execute SVG scripts or fetch referenced resources.
Use the versions recorded in the manifest when byte-identical outputs matter.

The historical `wolf` skin key is retained for compatibility. Default artwork
and custom PNG replacements keep their colors; an existing `wolf.svg` skin
override remains a monochrome mask tinted with the theme's normal icon color.

Asset parsing, color/transparency and regeneration checks are distinct from
Windows Explorer/taskbar and macOS Dock/Finder acceptance. Native appearance
still needs a build and visual review on the corresponding platform.
