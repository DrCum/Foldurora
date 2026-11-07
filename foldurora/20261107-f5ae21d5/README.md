# Synthetic photo/export and native-icon evidence — f5ae21d5

Partial visual validation of frozen source
[`f5ae21d535da3a338bace51ffb03af7beeb040c6`](https://github.com/DrCum/Foldurora/tree/f5ae21d535da3a338bace51ffb03af7beeb040c6).
The normal Linux binary reports `Aurora Viewer 0.0.0+1510.gf5ae21d53`, SHA-256
`0a251831eeb7a8da36a279c0fe4b20ae54e75584542d598af6a23968efb2a4c3`.
This evidence belongs to that source, not the later upstream-integration source.
The seven unchanged PNG copies total 1,487,859 bytes.

Two mandatory owned offline demo cases exited normally using software Vulkan
(lavapipe), low graphics, a 15 FPS cap and isolated test data. An earlier
cold-cache attempt stopped at the startup watchdog before photo mode and produced
no successful image. That failed checkpoint remains private. The successful
mandatory batch used a warm task-local cache and INFO logging.
Cargo was idle. A separately approved small Interface case then completed
normally in about 143 seconds. All successful app/driver/display processes
closed normally, and all owned process groups are gone.

| PNG | What it shows |
| --- | --- |
| [1. Restricted landscape, frame 680](01-restricted-landscape-frame-680.png) | 960×640 workspace; scene and guides stay in the right-hand world viewport, with controls restricted to its 480-pixel width. |
| [2. Matching clean landscape export](02-restricted-landscape-clean-export.png) | Actual 640×360 PNG output at 16:9. |
| [3. Unrestricted controls, portrait, frame 680](03-unrestricted-portrait-frame-680.png) | 960×640 workspace; controls may use the workspace while scene/guides retain the right-hand world scope. |
| [4. Matching clean portrait export](04-unrestricted-portrait-clean-export.png) | Actual 360×640 PNG output at 9:16. |
| [5. Native Linux window icon](05-native-window-icon-64.png) | 64×64 icon read from the exact owned window's `_NET_WM_ICON` property. |
| [6. Full Interface workspace, frame 680](06-interface-workspace-frame-680.png) | Actual 960×640 workspace with ordinary Inventory, World Map and Preferences windows. |
| [7. Matching Interface export](07-interface-workspace-export.png) | Actual 480×320 PNG retains the full workspace and the scene's right-half position. |

Both frame-680 views and their exports were inspected. Frame-900 captures were
also verified and retained privately. The scene's green-panel edge measurements
agree with the centered guide-to-export transform within two output pixels in
landscape and one in portrait, allowing blur/rasterization tolerance. The bounded
check uses a fixed green-boundary threshold and three-row/column averages while
excluding guides and the dock center. The native 64-pixel icon was also inspected.

The Interface pair was also inspected. The scene's left edge maps from native
x480 to exported x240; a workspace UI/shadow edge maps (264,68) to (132,34), and
the Inventory frame begins at (13,70) versus (6,35), consistent with half scale
and pixel rounding. Ordinary workspace windows remain; photo controls are
intentionally omitted from the export. Other windows obscure much of the scene,
so this check uses visible top-band boundaries and retained workspace geometry.

**Open UI limitation:** at the restricted 480-pixel width, viewfinder controls
overlap heavily and clip. These successful scope/composition checks do not
establish full visual or usability acceptance of the viewfinder.
Narrow toolbar crowding also remains visible.

This is synthetic software-Vulkan evidence. Windows embedding/input/taskbar,
physical GPU behavior, real accounts/grid, complete photo controls and general
performance remain unvalidated here. No ordinary profiles or real-grid login
were used. Only unchanged synthetic PNG copies are published: no settings files,
private paths, authentication/bootstrap material, raw logs, raw analysis JSON or
user reports/screenshots. The
[canonical ledger](https://github.com/DrCum/Foldurora/blob/docs/foldurora-progress-20261106/docs/FOLDURORA_PORT_STATUS.md)
keeps source-specific builds, test receipts and remaining acceptance separate.
