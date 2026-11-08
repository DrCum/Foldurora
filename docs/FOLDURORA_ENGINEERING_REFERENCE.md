# Foldurora engineering reference

Technical guidance retained from the inherited viewer instructions. The
[Foldurora policy](../AGENTS.md) governs workflow and overrides old Aurora
process requirements. Consult these sections only when relevant.

## Behavior, protocol and licensing

Use Folderstorm as the reference for its added behavior and Firestorm where
Second Life protocol or existing viewer behavior is involved. Implement in
Aurora's Rust/Vulkan architecture rather than copying historical OpenGL
constraints. Preserve defaults and visible behavior unless the user requests
a change; explain intentional differences.

Useful reference locations include `indra/newview/`, `indra/llcharacter/`,
`indra/llmessage/`, `indra/llrender/` and
`indra/newview/app_settings/settings.xml`. Prefer an existing read-only
reference checkout or pinned remote source; do not create an unnecessary
large clone. Name the originating function/file in port comments. Preserve
the GPL-compatible licensing and notices described in [NOTICE.md](../NOTICE.md).

## Creator permissions and RLVa

Every operation that reads, displays, copies, exports, saves, sends or reuses
an asset or identifier must enforce the reference permissions or stricter.
This covers textures, materials, meshes, animations, sounds, scripts,
notecards, environments, gestures, wearables and object contents.

- Find the actual reference check first: for example
  `LLViewerInventoryItem::getPermissionMask`/`getProtectedAssetUUID`,
  `LLPermissions::allowOperationBy`, `LLFloaterTexturePicker`,
  `LLPanelFace::onCopyTexture`/`onPasteTexture`,
  `LLMaterialEditor::canClipboardObjectsMaterial`,
  `LLPreviewTexture::canSaveAs`, or `LLToolDragAndDrop` acceptance checks.
- Use actual shared helpers in the current fork rather than hand-computed
  permission masks. Inspect the current APIs: historical task catalogues may
  name upstream helpers not yet integrated. Extend the shared rule when
  necessary instead of inventing unavailable calls or relaxing a check.
- Apply the same rule to both UI styles, alternate commands, developer tools,
  scripts, RLVa replies, clipboard, chat, previews, imports/exports and demos.
  Server enforcement never replaces viewer-side refusal or disabled controls.
- Show/copy/share asset IDs only when the shared rule permits it. Generated
  offline demo assets do not confer rights to real grid assets. A release
  build must not expose the user's cache store.
- Cover owner, group, other-owner, Library, no-copy, no-transfer, no-modify,
  full-permission and applicable RLVa cases. Retain worn/drop restrictions
  and final identity/permission rechecks at the operation boundary.

## Rust, concurrency and performance

- Handle errors from network, files, assets and settings; do not unwrap
  external data. Test assertions and documented internal invariants differ.
  Every unsafe block needs a `SAFETY` justification.
- Keep expensive work off the UI/frame thread. Use bounded background work
  and appropriate parallel/GPU work; avoid waits on GPU completion in the
  frame loop. Preserve ownership, cancellation and resource limits.
- Measure material performance changes using the existing profiling tools
  described in [TESTING.md](TESTING.md). Record evidence, not assumed gains.
- Keep modules focused. Read/update relevant [architecture](../ARCHITECTURE.md)
  sections when responsibilities change. Dependency/toolchain changes need
  concrete justification and GPL-compatible licensing; they are not routine
  cleanup.

## Shared UI and localization

Follow the existing viewer theme and [BRANDING.md](BRANDING.md), with the
user's Foldurora branding taking precedence over inherited product naming.

- Use `Palette` colors, including `on_accent`, and theme-provided radii.
  Reuse `widgets::Floater`, `Modal`, `overlay` and `overlay_frame` surfaces.
  Avoid custom backgrounds, borders, shadows and hard-coded radii/colors.
- Reuse shared action/primary/destructive buttons, progress, keycaps, switches,
  checkboxes, radio choices, collapsible sections, color buttons and dividers.
  Menus use `ui/menu.rs` rows. Respect existing theme guards; content exceptions
  require the existing documented mechanism, not a styling bypass.
- Use the established icon for a function, normally the existing Tabler set.
  Keep icon choice consistent across UI and cursors; preserve the supplied
  Foldurora application icons.
- User-visible text goes through `aurora_i18n::tr!`; keep EN, FR and JA keys
  aligned. Follow [TRANSLATING.md](TRANSLATING.md) for variables and plurals.
- Inspect changed UI alongside neighboring controls. State precisely whether
  evidence is CPU/offline, default-glass/Vulkan, native Windows, or human
  acceptance; one does not establish the others.
