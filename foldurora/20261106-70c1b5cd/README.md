# Foldurora synthetic visual evidence

Reviewed offline captures from 6–7 November 2026 UTC. These seven unchanged
PNGs total 717,005 bytes. They contain fixture data, not human grid screenshots.

Images 1–6 use source
[`70c1b5cda927547b28e5a5ff9713e861771aaee7`](https://github.com/DrCum/Foldurora/tree/70c1b5cda927547b28e5a5ff9713e861771aaee7),
the corrected profile-popup candidate. The Linux development binary reports
`Aurora Viewer 0.0.0+1494.g70c1b5cda`, SHA-256
`ab4a01e7f149fc8559d37d54b3bd040dc6c70941a978273184e609e43b1c13ff`.
Image 7 uses the earlier source
[`5c741b6103328fb32813aca3a462ea930a9c441b`](https://github.com/DrCum/Foldurora/tree/5c741b6103328fb32813aca3a462ea930a9c441b);
that chat component is unchanged through `70c1b5cd`, but it is not a capture of
the latest complete UI.

| PNG | What it shows |
| --- | --- |
| [1. Japanese selector](01-ja-selector-open.png) | All five localized slots, native Linux CPU controller. |
| [2. Narrow long-label popup](02-en-narrow-long-label-popup.png) | 760×600 controller, 64-character ASCII label; popup frame/content remain inside the roster. |
| [3. Narrow simulated occupied slot](03-en-narrow-simulated-occupied-slot5.png) | A fixture-held slot-5 file lock, wrapped inline help and disabled Open. No worker was launched in this simulated-lock case. |
| [4. Slot 1 after reopening](04-slot1-reopened-clock-preference.png) | Real offline demo worker: Clock seconds remains off after normal Quit, actual process exit and reopening the same slot. |
| [5. Slot 2 default](05-slot2-clock-preference-default.png) | A later sequential demo worker keeps its independent default on setting while slot 1 remains off. |
| [6. Crash-details fallback](06-crash-details-synthetic.png) | Actual Linux CPU helper rendering an entirely synthetic no-stack report. Its “Windows” report fields are fabricated fixture content. |
| [7. Restored Nearby draft, earlier source](07-chat-nearby-draft-restored-5c741b61.png) | Actual egui chat body/software painter with synthetic source/conversation data; no native popout or socket. |

The controller fixtures used private XDG roots and no workers or login actions.
Captures 1–3 came from `run-NdHjIHbH`. The sequential worker fixture
`worker-run-tKsGOMYz` used software Vulkan, low graphics, 960×640 windows and
a 15 FPS cap. Slot 1's real UI setting and owned settings JSON persisted; slot 2
had a distinct settings/cache namespace. Cache-key file metadata was checked,
not key bytes or cache contents. Workers were explicitly detached on Linux,
where hosted presentation is unsupported. Workers ran sequentially, with actual
process exit checked before each next launch. All owned workers/controllers
ultimately exited normally; no concurrent GPU workers were used.

Xvfb/softbuffer can expose a stale front buffer. One action followed by two
owned Expose passes preceded inspection. A faint one-level RGB popup shadow can
reach two columns beyond the roster; frame/content bounds passed, without a
zero-outside-pixel claim. The simulated occupied-slot image does not establish
Windows process contention, and the crash fixture did not induce a GPU reset.

These images do not establish Windows embedding/taskbar/input behavior,
physical GPU coexistence, five live profiles, voice, sealed-cache content reuse
or real-account/grid behavior. No grid, credentials or ordinary profile data
were used. No raw logs, bootstrap files, cache keys or user-uploaded images are
included. Automated checks, offline native acceptance and human grid acceptance
remain distinct in the
[canonical progress ledger](https://github.com/DrCum/Foldurora/blob/docs/foldurora-progress-20261106/docs/FOLDURORA_PORT_STATUS.md).
