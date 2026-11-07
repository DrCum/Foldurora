# Compact Foldurora host — synthetic visual evidence

Six reviewed, unchanged PNGs from the final EN/FR/JA CPU fixture set at source
[`399ffd796ccba8e9468bbc9ff7b19eff5017d4bc`](https://github.com/DrCum/Foldurora/tree/399ffd796ccba8e9468bbc9ff7b19eff5017d4bc).
The selected images total 589,579 bytes; the full locale-isolated run generated
21 captures. Earlier failed fixture sets are not published here.

The fixture drives the production `HostUi`, existing CPU software painter and
embedded icons with five fake workers. It has no native controller window,
transport, renderer, GPU, profiles/account files or grid. Real widget clicks
return typed intents without dispatch. The layout uses a fixed 38-point strip,
full-width world area when management is hidden, and an explicit management
drawer. The hosted highlight follows acknowledged worker state, independently
of the selected profile. Controls retain typed source identities and permissions.

| PNG | What it shows |
| --- | --- |
| [1. EN wide](01-en-wide-five-workers.png) | Five fake workers, compact strip and full-width world placeholder. |
| [2. EN management](02-en-management-open.png) | Search, profile management and source-bound session actions in the explicit drawer. |
| [3. EN narrow popup at 150%](03-en-narrow-150pct-profile-dropdown.png) | 540×680 logical points / 810×1020 pixels, condensed tabs and bounded profile popup. |
| [4. EN Close confirmation](04-en-close-confirmation.png) | Confirmed detach-and-close flow with synthetic worker state. |
| [5. JA management](05-ja-management-open.png) | Japanese glyphs and translated actions wrapping inside the drawer. |
| [6. FR Rename](06-fr-profile-rename-dialog.png) | French modal text and a synthetic long profile label. |

The final capture checks inspect central icon pixels for a visible glyph,
control bounds above the world, full-width world geometry with management
closed, actual popup/modal outer areas and translated management-action
horizontal bounds. Narrow/150% captures and long labels are included. These
checks establish fixture layout and pixel rendering, not native hit testing.

The normal Linux binary from the same source separately built as
`Aurora Viewer 0.0.0+1502.g399ffd796`, SHA-256
`48c43f104a987dccd0d8c204c8149735211fe819995aca7a4258e1176f507fac`.
An actual owned EN Linux CPU-controller smoke also passed compact layout,
management, five-slot selector, real synthetic Rename/Save, confirmed Close and
same-executable/private-path restart retention. It launched no workers, login
or world renderer; its screenshots and private test data are not this fixture.

Windows popup-island composition/hit testing, embedded world/input behavior,
native worker switching/recovery, real identities/chat, mixed DPI, multiple
monitors and physical GPU/grid acceptance remain separate. No private JSON,
bootstrap files, authentication material, raw logs, user files or user screenshots
are included. Exact validation and pending follow-ups are recorded in the
[canonical ledger](https://github.com/DrCum/Foldurora/blob/docs/foldurora-progress-20261106/docs/FOLDURORA_PORT_STATUS.md).
