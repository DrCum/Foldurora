# Workspace Inventory images — 9 November 2026

Workspace padding can use owned Inventory textures/snapshots, including separate
display choices, Fill/Fit/Stretch/Center/Tile/Span and brightness. Span uses one
common image across the padding union; per-display choices remain saved for when
Span is turned off. Choosing a solid common color keeps separate display choices.
Black clears them. Images are always clipped to the current non-world strips.

The existing native texture chooser has a fixed separate workspace role. Other
editors keep their selection and geometry. The manager emits a source-bound
request; explicit confirmation captures its original display target. Another
workspace action, manager close, source/focus/privacy/settings change retires
the pending choice. Native authority, the exact reference and a fresh display
topology are checked again before the existing private CAS lane publishes it.
Brightness uses explicit Apply, not a file write for every slider movement.

The private catalogue stores item UUID, asset UUID and original native grid
namespace; no names, credentials or filesystem paths. These references are not
permissions. The current native login/account/grid, owned ordinary/archive Ready
view, already fetched ancestry, full copy/modify/transfer rights and RLVa
showinv/viewtexture checks precede image requests and use. Confirmation chooses
an already loaded ordinary owned item for the chosen asset (deterministic by
item UUID for duplicates). No Library, link, hidden/archive exemption or saved
reference can initiate an Inventory fetch. Explicit chooser browsing uses the
existing native picker. Its cached pixels also need a fresh ordinary proof.

Image admission uses the existing shared texture streamer and worker bounds,
normal texture budgets and copies at most256px per side. At most18 small copies
exist:17 possible saved choices plus one explicit chooser preview. Hover and
background UI interests are independent, idempotent and share one streamer
reference with session-long users. Closing either bounded user cannot release
the other's interest or a world reference. Repeat sampling belongs only to the
workspace's own small egui handle. Unused/source/recovery/logout retirement
drops that handle and its interest; late work cannot authorize a new source.
There is no new renderer, worker pool, readback or permanent-image request.

Fresh owner visibility is reacquired immediately before both world-padding and
chooser paint. Qualification is cached only for the same actual source/binding,
ordinary visibility proof and bounded reference list; inventory, permissions,
privacy and archive publication changes retire that proof. This keeps repeated
folder traversal out of an unchanged frame without treating cached approval as
authority. Current texture dimensions define Center/Tile when available; a
bounded preview is the fallback while native metadata arrives.

Reference: DrCum/Folderstorm `abbe3cb0ca66ee2484f277219f6a27706fab7fc6`,
`fsworkspacebackgroundmodel.{h,cpp}` and `fsworkspacebackground.cpp`. This port
intentionally requires full permission and ordinary owned references rather than
an arbitrary texture UUID. The native shared asset-cache behavior remains.
High-resolution wallpaper admission and managed local-file import remain
unfinished. Local files must be copied into bounded private profile storage;
persisting a live filesystem path is not an acceptable substitute. This slice
does not claim complete Folderstorm background parity.

Formatting, qualified Linux viewer all-target ci Clippy,13 i18n cases and the
unchanged full-tree appearance scan pass.13 actual-source model/placement cases
pass (5 new,8 reused). Five further genuine streamer/cache/native-policy/chooser/
UI regressions compile but were not executed locally. An unused import and
collapsible-if development failure are preserved. Final review also added a
fresh chooser-paint proof after possible changes earlier in the same UI pass;
the final compile check includes it. No security assertion or sharing policy
was waived. No full Linux viewer link, Windows package or native acceptance is
claimed. Frozen a903 is already active with the existing Windows Builder and
is unchanged; this is not a competing build request.

Native follow-up: verify source/privacy/permission loss hides the image, opposite
display images and common Span, Preview/Cancel/Previous, brightness Apply,
restart/account switching, hosted movement/mixed DPI, and coexistence with a
normal editor picker/hover preview. Real-grid rights/restrictions remain Sodie's.
