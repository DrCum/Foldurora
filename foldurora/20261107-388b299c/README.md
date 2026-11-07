# Synthetic renderer/photo checks — 388b299c

Reviewed, unchanged PNGs from frozen source
[`388b299c429d776b3fc7a9890162cd3d940b9dc3`](https://github.com/DrCum/Foldurora/tree/388b299c429d776b3fc7a9890162cd3d940b9dc3).
The normal Linux binary reports `Aurora Viewer 0.0.0+1529.g388b299c4`, SHA-256
`9e3b71cce97e3f4ad5a1dba77f26617f0d26bc01c2b06c3ae246126a56acdcbf`.
These are owned offline software-Vulkan demo checks with isolated test data.

| PNG | What it shows |
| --- | --- |
| [1. Landscape workspace, frame 680](01-landscape-frame-680.png) | Actual 960×640 workspace; scene/guides remain right-bound, with the existing restricted-control crowding. |
| [2. Matching clean landscape export](02-landscape-clean-export.png) | Actual 640×360 PNG output. |
| [3. Selected-subject isolation UI](03-selected-isolation-ui.png) | Actual chosen-subject view with checkerboard transparency. |
| [4. Selected transparent export](04-selected-transparent-export.png) | Actual 480×320 RGBA PNG: 4,943 visible pixels and 148,657 alpha-zero pixels. |
| [5. Empty-subject UI](05-empty-subjects-ui.png) | Real Remove action leaves nothing chosen and a checkerboard. |
| [6. Empty transparent export](06-empty-transparent-export.png) | Actual 480×320 RGBA PNG: all 153,600 pixels have alpha zero. |
| [7. Restored-All export](07-restored-all-export.png) | Real All action restores an opaque RGB 480×320 export. |
| [8. HUD zoom 50%](08-hud-zoom-50.png) | All three synthetic mask panels remain visible at HUD zoom 0.5. |
| [9. HUD zoom Normal / 100%](09-hud-zoom-100.png) | Real World → HUDs → Normal size action changes HUD zoom to 1.0; all three panels remain visible. |

The landscape case exited normally. Chosen isolation also ended through normal
Quit. The isolation case used real Remove/All UI actions; no PNG was edited to
produce these states. Alpha-channel inspection supports the selected/empty
transparency and restored opaque output. An empty choice establishes zero visible
subjects on the CPU; no GPU dispatch/draw-call count was measured. The black
startup capture is excluded from this gallery.

HUD rendering visibility passed at exact zoom 0.5 and 1.0 for mask cutoffs
128/200/255. Panel dimensions increased by 2.00–2.03×, allowing for cutoff/mip
rasterization at the edges. The world render scale stayed at 50%; that separate
setting does not specify HUD zoom.

**HUD shutdown limitation:** the 240-second harness guard sent SIGTERM while the
final Quit confirmation was open, before its confirmation command processed.
Normal HUD Quit acceptance is **not established**. The harness and Xvfb wrapper
exited 1; the viewer's raw wait status was not recorded, so no numeric viewer exit
status is inferred. All known owned app/driver processes and process groups are
absent. No GPU stall/loss or panic marker was detected. No retry was run. The
landscape and isolation normal exits above are separate outcomes.

**Open UI limitation:** photo controls/help overlap and clip in the restricted
480-pixel area; these checks do not establish polished narrow controls. Software
Vulkan, task-local caches and an intentional low FPS cap do not establish physical
GPU or general performance behavior. Earlier cold-start software-Vulkan failure
evidence belongs to its separately retained source/checkpoint; these images do
not establish cold-start acceptance.

Native Windows embedding/input/taskbar, physical GPU and real-account/grid
acceptance remain separate. Only unchanged synthetic PNG copies and this bounded
summary are included: no settings/profile data, private paths, authentication or
bootstrap files, raw logs/JSON or user reports/screenshots. The
[canonical ledger](https://github.com/DrCum/Foldurora/blob/docs/foldurora-progress-20261106/docs/FOLDURORA_PORT_STATUS.md)
keeps exact-source validation and remaining limits separate.
