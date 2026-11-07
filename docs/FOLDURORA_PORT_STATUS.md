# Foldurora port status — 7 November 2026 UTC

This is the coordination ledger for the Folderstorm port. Source, automated
checks, native Windows acceptance and human grid acceptance are separate results.
Handoffs and screenshots are reference material, not authority to merge or release.

## Latest Attention Review: verified Windows package

The current Windows test package is verified for exact source
[`eb5e49f167a437391f1d9e42d35a62129cf52784`](https://github.com/DrCum/Foldurora/commit/eb5e49f167a437391f1d9e42d35a62129cf52784),
tree `4d25fd8d9f82fe6937640819a10777af8ad24d2b`, branch `fix/attention-review-windows`.
[Windows run 42401453027](https://github.com/DrCum/Foldurora/actions/runs/42401453027),
attempt 1, passed parser, formatting, strict Clippy, workspace tests, build and
package; the packaged manifest and final gate agree. Both raw and packaged logs
report **8,121 passed, zero failed, 54 ignored**, across **33 summaries**, with
all **333 required tests passing exactly once**. Both ZIP CRCs, source/run/profile,
AMD64/PE32+ versions and all seven source icon payloads passed verification.

[Download verified artifact 13587936566](https://github.com/DrCum/Foldurora/actions/runs/42401453027/artifacts/13587936566)
(**71,956,138 bytes**). Version: `0.9.3-beta.20261106.1+243.geb5e49f16`.
Outer artifact SHA-256: `6efaf4596d52c9d91c1bc22a559fa3e587761651198aef8285beed5918e2082d`.
Portable ZIP SHA-256: `c4ca060fbc584a75640045828cf567b22232916b895caaa7acff06761faedd80`.
Executable SHA-256: `eb2ddf6aaa8eadbfb45d1ba9e1b77cf0f982fe326ff8bccf5402c2dc799bf6c7`.
Receipt: `.scratch/windows-ci-42401453027/receipt.json`, SHA-256
`093b90e0d5dfc3c452d4ee4d63ef73e38f639058cefbcdc4866c8bbbddc43f7e`.

The package adds explicit **Review in viewer** for existing conversations beside
canonical **Shared Chat**. **Usable monitors and live previews are not available
yet**. The [native/human handoff below](#review-eb5-windows-acceptance-handoff)
uses this exact package. No executable was installed or launched for verification;
native Windows input, human Review, GPU/multiple monitors, real IME and grid
acceptance remain pending. No camera/GPU-loss fix or full monitor parity is
established. The earlier 15d MVP, negative 024 Windows run and exact 024 Linux
component results below keep their separate scope; none transfers to newer code.

## Earlier Attention MVP: verified Windows fix and preserved negative checkpoint

The earlier MVP fix source is frozen at
[`15d33e6c9225bb476f224df02446e3d8c8c94c8d`](https://github.com/DrCum/Foldurora/commit/15d33e6c9225bb476f224df02446e3d8c8c94c8d),
on `fix/attention-windows-clippy`. It uses the equivalent production
`Preferences::includes` helper and adds that branch to the Windows workflow
allow list. Its [Windows run 42390650825](https://github.com/DrCum/Foldurora/actions/runs/42390650825)
is **verified green for exact 15d**, attempt 1. Actual PowerShell 5.1 parser,
formatting, strict Clippy, workspace tests, build and package commands all passed.
Both raw and packaged logs report **7,991 passed, zero failed, 52 ignored** across
32 summaries; all **203 required names** pass exactly once. Both ZIP CRCs,
source/run/dev-profile identity, AMD64/PE32+ version and all seven source icon
payloads were verified. The package manifest reports `all_checks_passed=true`.
The original `feat/attention-integration` branch remains frozen at negative 9a.

Verified [artifact 13583034550](https://github.com/DrCum/Foldurora/actions/runs/42390650825/artifacts/13583034550)
version: `0.9.3-beta.20261106.1+207.g15d33e6c9`.
Outer artifact SHA-256:
`6ced4df21b2be80b074356e09679fb43ee296a54ac47aaf58c1d54e5a7cf4afb`.
Portable ZIP SHA-256:
`9c41f011c4fc0625bc5e022ea0ecd9f81f4b05d3b22c8cf189304fd08e588e89`.
Executable SHA-256:
`1805013d2ffcfa089c324d485ac7ba76c43fa00fdd557809047c1fb460026f91`.
The build worktree retains `.scratch/windows-ci-42390650825/receipt.json`
(SHA-256 `d8f2ce9d870df387a3f7672cb01e331ecb5589a475557c62c43b0d5485ce5250`),
`LEDGER-UPDATE.md` and `icon-resource-verification.json`. No Windows executable
was installed or run for this verification. Native Windows input, GPU/multiple
monitors, real IME, real profile/preference IO and grid acceptance remain pending.
The separate native Linux component evidence below does not transfer to 15d.

The validated Linux and negative Windows checkpoint is frozen at
[`9a6918b4e7334e4da319449b56e985107e6b5cd5`](https://github.com/DrCum/Foldurora/commit/9a6918b4e7334e4da319449b56e985107e6b5cd5),
pushed on `feat/attention-integration`. It builds on the previous frozen Enter
guard candidate below. This first Attention slice lists unread Nearby/existing
Direct conversations from the five stable saved profiles, with explicit profile,
actual account/grid and conversation identity. Direct is on and Nearby is off
by default. The compact host bell and management entry open an owned CPU window;
profile/category filters are collapsed by default and scroll when expanded.
Open shared chat and Previous/Next perform passive exact-source navigation;
Dismiss reminder changes only the local reminder book until new activity.
There is no native Review action, monitor preview or chat protocol V3 in this MVP.

Exact-source Linux validation passed all **203 required checks** exactly once,
including real-egui stale input, selection, filter/save lifecycle, compact layout
and first-profile pointer interaction at **380×340**. All **54 production CPU
captures** passed required-text/clipping/privacy checks: 18 each for EN/FR/JA,
including 540×580, narrow 380×580 at 150%, and short 380×420 scenes. The full Linux
workspace run reports **8,020 passed, one inherited terrain precision failure,
53 ignored**. Clippy passed with the existing Linux `dead_code` allowance.
These are the measured Linux results; the inherited renderer failure remains
recorded and the full run is not described as entirely green.

[Six unchanged Attention CPU captures, original manifests and hashes](https://github.com/DrCum/Foldurora/blob/ac85be2021bb1c17ca8a75e2d7ea54c49a97392b/foldurora/20261107-9a6918b4-attention-cpu/README.md)
retain the exact source identity. The gallery is a six-image subset of the 54
passing captures, with selected/expanded filters, French narrow bottom filters,
Japanese short filters and simulated save/load failures. Images use the actual
production egui body and software painter with synthetic body-free snapshots.
They establish no native-window/OS-input, Windows, physical-GPU, real IME,
authentication, preference-file IO, World or grid acceptance. The 340-pixel
minimum-height result belongs to the actual-egui regression, not the short
420-pixel-high gallery images.

The validated test ELF is **770,055,472 bytes**, SHA-256
`a5f017ee4d2834c3cbbf6b6e4794007c16ca3c98bdf4025ae70db23dc043cbfc`.
It is distinct from a normal viewer executable. The build worktree retains
`.scratch/attention-validation-9a6918b4/validation-receipt.json` and
`source-expectations.json`; the original runtime receipt is also in the capture
publication. The exact 9a normal Linux viewer build now **passed in 321.083 s**
from clean tracked source. Version: `0.0.0+1579.g9a6918b4e`; size **670,553,544
bytes**; SHA-256
`b05f7500462de2e7f2dcf7fdf7ff9287f16ba3bbcad26fa1e0ea6e51b26901ed`.
The separate `.scratch/attention-validation-9a6918b4/normal-build-receipt.json`
records that normal build. It establishes no native launch or Windows result.

[Windows run 42385255872](https://github.com/DrCum/Foldurora/actions/runs/42385255872)
for exact 9a is verified as an **overall failed candidate**. Strict Clippy rejects
the unused production `Preferences::includes` at `attention_prefs.rs:41`;
test-only callers do not satisfy the production binary check. The workflow final
gate correctly rejected `CLIPPY_RESULT=failure`. Earlier provisional API step
conclusions must not be used to infer command success with `continue-on-error`.

The measured Windows tests nevertheless passed: **7,991 passed, zero failed,
52 ignored**, across 32 summaries, with all **203 required names** passing exactly
once in both raw and packaged logs. Parser, formatting, tests, build and package
passed; both package manifests explicitly report `all_checks_passed=false`.
Both ZIP CRCs, source/run/profile identity, AMD64/PE32+ executable version and all
seven embedded source icon payloads were verified. These static/package results
do not override the strict-Clippy failure or establish native acceptance.

Negative-candidate [artifact 13581251607](https://github.com/DrCum/Foldurora/actions/runs/42385255872/artifacts/13581251607)
version: `0.9.3-beta.20261106.1+205.g9a6918b4e`.
Outer artifact SHA-256:
`fd5477ab985f3ffbd980d3cf65dedc2c25afa5c0e9b1014e033bc2951d1284ae`.
Portable ZIP SHA-256:
`e0c3855a6ed431ed2a6b13c880473f702cb30a41629b4f42b9c8ba984a2bb015`.
Executable SHA-256:
`585b53ed11045451dacf35029a444cb0951a171818ba574c36c126f77256258b`.
The build worktree retains `.scratch/windows-ci-42385255872/receipt.json`,
`LEDGER-UPDATE.md`, raw/packaged logs and icon verification. No Windows executable
was installed or run for this verification. Keep this negative 9a receipt and
the verified-green 15d run separate from the prior verified 6b package below.

### Separate native Attention component and Review/monitor prerequisites

[Six unchanged native Attention captures and the independent receipt](https://github.com/DrCum/Foldurora/blob/4f59fd97ddbfcc3a81cf98542dcc504a28775191/foldurora/20261107-2d809422-attention-native-cpu/README.md)
record an exact-source **component pass** at
[`2d8094224e1b44f2cc557d944b9db10ad51e3886`](https://github.com/DrCum/Foldurora/commit/2d8094224e1b44f2cc557d944b9db10ad51e3886).
The production AttentionWindow received real isolated X11 input with synthetic
body-free snapshots and recorded local intents. All six cases passed: Enter,
Space, stale captures, navigation, Dismiss/reopen and simulated preferences.
The independent audit checked **102 states, 96 case checks and all 36 PNG
hashes/dimensions**. The gallery publishes six original PNGs without editing,
resizing, composition or recapture, with original-name/hash metadata.

Exact test ELF: **775,619,920 bytes**, SHA-256
`99ee531b6c3c5c274598982d656b88884f98138fa89f7991bbee09cb57442232`.
Driver SHA-256:
`f0f80738ba23a605ba1ccedee54e7b9144619e11da4d141b29ac280651599067`.
Suite `suite-ss3i70g1` completed in **63.864 seconds**; all six fixture PIDs
exited normally with code 0 and were reaped without forced cleanup. The six
Xvfb servers were reported as present/Z/exited-unreaped/not running; all were
already Z at the first grace observation, so no actual S-to-Z grace transition
was exercised. **Native wheel edges: 0**: all seven filter controls already fit.
Visible reachability is established here; native offscreen scrolling is not.

This evidence does **not** establish native Review, Shell/Shared Chat delivered-
edit routing, Worker/World authority or activation/read/send, authentication/IPC,
preference disk IO, Windows, physical GPU/multiple monitors, real IME, profiles,
credentials, grid acceptance or native read-on-draw safety/default changes.
The full 2d809 workspace result remains **8,137 passed, 16 failed, 55 ignored**:
15 new failures plus the inherited terrain precision failure. The six-case
component pass does not waive them or transfer acceptance to 15d or later code.
Original evidence remains under
`.scratch/review-monitor-integration-2d809422/native-driver-f0f80738-independent/`;
independent receipt SHA-256:
`4b00fdbcac41b5a5ea6dcdadc26170eda51295f5e61b9bb1af60808f3816dd71`.

The newer Review/monitor-prerequisite integration is frozen at
[`024ff943ac6feeef092fd83f906dc48e04a2dcbe`](https://github.com/DrCum/Foldurora/commit/024ff943ac6feeef092fd83f906dc48e04a2dcbe),
tree `8a7a34df39586866c5a53b1aab3fbf01981957f9`, on
`feat/attention-review-integration`. Its exact committed-source inventory finds
137 added test definitions over 9a, no removed/renamed/reignored definitions,
and 333 required Windows names. Inventory totals are expectations, not outcomes.
Qualified Linux Clippy passed in **116.535 seconds**, with the existing explicit
`-A dead_code` qualification. A targeted ignored-test run passed both actual
headless synthetic GPU cases exactly once: crop/colors/sRGB/channels/alpha,
and mapping success/reuse/cancel/drop without GPU validation errors.
These are synthetic GPU texture tests, not native Windows/NVIDIA/producer or
human acceptance. Receipts are
`.scratch/review-monitor-integration-024ff943/{source-expectations.json,early-checks-receipt.json,gpu-outcome-audit.json}`.

Exact 024 Linux validation now reports **8,154 passed, one inherited terrain
precision failure, 56 ignored**, with all **333 required names passing once**.
All **90 synthetic CPU Attention captures** passed (30 each EN/FR/JA). Its
separate six-case native Linux CPU Attention run also passed, using test ELF
SHA-256 `a385a6aabb6229d92820aaf62004668d8a459a15b220b7c0592d94eca03abe92`
and unchanged driver `f0f80738`; suite `suite-onywebhu` took **63.560 seconds**.
The independent archived audit checked **96 predicates, 102 states and all 36
PNG hashes/dimensions**; all six fixtures exited normally and were reaped without
forced cleanup. Xvfb was already Z/exited-unreaped at its first grace observation,
so no actual S-to-Z grace transition was exercised. Its receipt is
`.scratch/review-monitor-integration-024ff943/native-driver-f0f80738-independent/NATIVE-ATTENTION-024FF943-F0F80738-RECEIPT.json`,
SHA-256 `97d82496d114f16a514e6d1f85d6b1824ea782ad43212e7f5cf58c9afeeede1d`.
These remain synthetic component checks: all seven filters fit, native wheel
edges were zero, and no World/Shared Chat/authentication/preferences IO/Windows/
grid/real IME/native Review acceptance is established. Full receipts are retained
under `.scratch/review-monitor-integration-024ff943/`; the full run is not green
because the inherited renderer failure remains. No normal viewer build was run
on exact 024; its normal-build identity must not be inferred from a later source.

[Windows run 42396161551](https://github.com/DrCum/Foldurora/actions/runs/42396161551)
is preserved as a **negative exact-024 candidate**: **8,120 passed, one failed,
54 ignored**, across 33 summaries. Of 333 required names, 332 passed once and
`viewport_gpu_shader_validates_without_a_driver_and_capture_path_has_no_waits`
failed once in both logs. Source review confirmed a test-only CRLF delimiter
false positive in that source scanner; this explanation does not turn the failed
run green. Parser, formatting, strict Clippy, build and package passed. Both ZIP
CRCs, hashes, source/PE/version and all seven icons passed static verification;
the package remains negative. Receipt:
`.scratch/windows-ci-42396161551/receipt.json`, SHA-256
`d9515eca7baa96bd6f6a072f312ab5a0a4bf3811c7a8c049c3d9ed42b3d19d8d`.

The separate newline-tolerant test/workflow fix is published at
[`eb5e49f167a437391f1d9e42d35a62129cf52784`](https://github.com/DrCum/Foldurora/commit/eb5e49f167a437391f1d9e42d35a62129cf52784),
tree `4d25fd8d9f82fe6937640819a10777af8ad24d2b`, branch
`fix/attention-review-windows`.
[Windows run 42401453027](https://github.com/DrCum/Foldurora/actions/runs/42401453027)
is **verified green for exact eb5**, with **8,121 passed, zero failed, 54 ignored**
and all **333 required names passing once** in raw and packaged logs. All six
commands, manifest/final gate, ZIP CRCs, PE/version and seven icons passed.
The [verified package details above](#latest-attention-review-verified-windows-package)
identify its artifact/version/checksums; the earlier negative 024 result remains
negative. Windows native/human acceptance is still pending.

Exact eb5 Linux qualified Clippy passed in **114.740 seconds**, the targeted
scanner test passed, and the normal viewer build passed in **347.870 seconds**.
That separate Linux executable is **672,080,440 bytes**, version
`Aurora Viewer 0.0.0+1617.geb5e49f16`, SHA-256
`de62603127f0e6d84412abb57eacf43493a1bdf4ba0063e463d0f8118085d2d3`.
These are Linux build checks, not Windows or native acceptance. Receipts are
`.scratch/attention-review-windows-eb5e49f1/{targeted-scanner-receipt.json,normal-build-receipt.json}`;
no full Linux suite rerun is claimed for this test/workflow-only fix.

Canonical monitor, image model/backend/transport/native permit work remains
separately scoped. No live inactive producer, production image endpoint or full
monitor parity is established by these receipts. Earlier source-only checkpoints
and the 2d native gallery remain distinct from exact 024 and eb5 evidence.

The [Review eb5 Windows acceptance handoff](#review-eb5-windows-acceptance-handoff)
below is a prepared recipe for the verified exact package; no human result or
handoff delivery is implied.

Human/platform follow-up for the exact verified package:

- Confirm the bell/drawer entry, exact profile/account/grid/conversation labels,
  collapsed filters and usable scrolling/actions at 380×340 and 150% UI scale,
  including long profile names and EN/FR/JA text.
- Verify Open/Previous/Next honor the exact intended sender and preserve
  canonical Shared Chat drafts without marking unread messages as read or activating a World;
  verify Dismiss stays local and new activity restores its reminder.
- Check held pointer/key actions across source/policy changes, focus loss and
  close/reopen. Held Enter/Space after refocus must remain inert until a physical
  release and fresh press; a vanished selected source must not select another.
- Check preference save completion/failure, published checkmarks, retry and
  queued-save close/reopen retention with the real controller's private IO.
  Real IME, native Windows and grid checks remain separate from CPU fixtures.
- The user reported that camera behavior is now as expected, without stating
  the exact tested commit. Keep that report without attributing it to this
  package or declaring full camera, GPU/device-loss or port parity acceptance.

## Review eb5 Windows acceptance handoff

Prepared native/human recipe only; package checks are verified, human acceptance
is pending. No test request is sent by this document; continue other work meanwhile.

Verified source: [`eb5e49f167a437391f1d9e42d35a62129cf52784`](https://github.com/DrCum/Foldurora/commit/eb5e49f167a437391f1d9e42d35a62129cf52784);
tree `4d25fd8d9f82fe6937640819a10777af8ad24d2b`;
branch `fix/attention-review-windows`;
[Windows run 42401453027](https://github.com/DrCum/Foldurora/actions/runs/42401453027).
[Verified artifact 13587936566](https://github.com/DrCum/Foldurora/actions/runs/42401453027/artifacts/13587936566), version `0.9.3-beta.20261106.1+243.geb5e49f16`.
Outer SHA-256: `6efaf4596d52c9d91c1bc22a559fa3e587761651198aef8285beed5918e2082d`.
Portable ZIP SHA-256: `c4ca060fbc584a75640045828cf567b22232916b895caaa7acff06761faedd80`.
Executable SHA-256: `eb2ddf6aaa8eadbfb45d1ba9e1b77cf0f982fe326ff8bccf5402c2dc799bf6c7`.
Verified: 8,121/0/54, all 333 required names once, all six commands, both CRCs,
source/PE/version and seven icons. The preceding 024 Windows run remains negative.
Review/Shared Chat are available; usable monitors/live previews are not yet available.

The separate MVP at `15d33e6c9225bb476f224df02446e3d8c8c94c8d` has
[verified artifact 13583034550](https://github.com/DrCum/Foldurora/actions/runs/42390650825/artifacts/13583034550),
version `0.9.3-beta.20261106.1+207.g15d33e6c9`. It has no native Review;
its green checks do not validate eb5. Keep that baseline and all **14 prior local
artifacts and their source heads** intact. Add a separate eb5 folder/worktree.

The builder may use the verified eb5 CI artifact above, or a cached local build
from that exact commit with its own recorded checks/version/hashes. Do not mix
controller and worker executables: both must come from the same eb5 package.
No installation, profile migration/deletion, merge or release is part of this handoff.
For a local build, record source/tree and the resulting executable hash:

```powershell
git rev-parse HEAD 'HEAD^{tree}'
Get-FileHash ./aurora-viewer.exe -Algorithm SHA256
```

Offline agents may inspect only their owned temporary installation, processes
and windows. They must not enter credentials, open real profiles or log in to a
grid. Positive Review needs a **human-managed logged-in worker with an existing
resident IM**. The human handles login and real-session interaction. Start the
matching controller with `--session-host`; keep its owned workspace in front.

Perform these eight checks in both UI styles where available:

1. **Exact target and foreground.** Select a character and its existing IM in
   Attention, then click **Review in viewer** once. Only that character becomes
   active and that captured conversation opens after the handoff completes.
   Opening requires the matching workspace to be foreground. If it is rejected
   as not in front, returning focus must not replay it; changing the selected
   row while waiting must not replace the captured target.
2. **Normal native reading.** The opened native conversation keeps its usual
   read-on-draw behavior, so unread counts may change normally. Review must not
   issue a separate host **Mark conversation read** action. Passive host
   navigation and **Dismiss reminder** must not queue an explicit Read or
   activate a World; native read-on-draw remains normal.
3. **No accidental Send.** In separate cases, test held Enter/Space,
   uncommitted IME text, and queued clicks/keys while switching or reviewing.
   Held/repeated keys, old IME commits and queued clicks/keys
   must not send into the new composer. Release and start a fresh gesture before
   any intentional Send; nothing should be replayed automatically.
4. **Drafts and Undo.** Keep different unsent drafts for two characters and
   conversations. Navigate and close/reopen Shared Chat or the viewer's existing
   native chat popout.
   Each draft and Undo history stays with its own source/conversation; Review
   must not send, overwrite or clear another draft.
5. **Honest Cancel.** Try **Cancel review** while waiting for activation, then
   separately after **Review requested**. A cancellation confirmed before
   dispatch should stop it. After dispatch, Cancel cannot promise prevention or
   undo a conversation already opened; uncertainty must stay explicit.
   Failed/uncertain requests must not repeat, and an old Cancel must not affect
   a later Review.
6. **Stale source or permission.** If the captured source closes/relogs, becomes
   unavailable or loses permission while Review is pending, its old target must
   fail closed. No other character, replacement IM or default row may be used.
7. **Window lifecycle.** Minimize/restore, move/resize, change monitor/DPI and
   close/reopen Attention and the controller as appropriate. Check alignment,
   foreground/status accuracy and held-input release; no hidden Review or stale
   action should run on restore. Preserve unrelated windows and profiles.
8. **Original GPU report.** Separately repeat the original standalone-switching
   workload on the affected GPU/driver when the human can do so. Record any
   device loss and the exact build. A Review pass does not establish that the
   original NVIDIA RTX 3060 device-loss report is fixed.

Record the exact source, displayed version, package/executable hashes and
pass/fail per check for later acceptance. Retain only body-free statuses or
sanitized captures; do not collect credentials, private chat or full raw logs.

## Previous frozen native Enter guard candidate

The previous frozen test source is at
[`6b512b1a`](https://github.com/DrCum/Foldurora/commit/6b512b1aa44102c871579a8a69211c12e0c87f89)
on `fix/shared-chat-enter-integration`. Its
[Windows run 42359207310](https://github.com/DrCum/Foldurora/actions/runs/42359207310)
and [download artifact 13570911560](https://github.com/DrCum/Foldurora/actions/runs/42359207310/artifacts/13570911560)
are verified: **7,931 passed, zero failed, 51 ignored**, across 32 summaries in
both raw and packaged logs. All **143 required names**, including eight new
Enter cases, pass exactly once; four obsolete names and the Linux-only native
fixture are absent. Parser, formatting, strict Clippy, full tests, build and
packaging passed. Exact source/branch/run, manifest, remote archive digest, both
ZIP CRCs, AMD64/PE32+ identity and all seven source icon payloads match. The
primary review independently rechecked the copied package's source, hashes,
portable CRC and both logs' counts/names. No PR, main merge or release occurred.

Windows version: `0.9.3-beta.20261106.1+193.g6b512b1aa`.
Portable ZIP SHA-256: `ce90bea08a543074f4a87b471e06ab1825f5090cc32e4088d431df776207c76b`.
Executable SHA-256: `c70daaea4e55a64ffb28a3ce8aac7d7188ce7e4d6a746c934dffe3b39b4b3fe7`.
The build worktree retains `.scratch/windows-ci-42359207310/receipt.json`,
`icon-resource-verification.json` and `primary-review.json`. These automated
checks do not replace the manual/platform/grid acceptance limits below.

The native guard observes held Enter and focus snapshots before egui-winit
discards synthetic key presses. Synthetic releases cannot rearm Send, including
X11's first real held press after refocus reporting repeat=false. It preserves
fresh release/Enter, Shift+Enter multiline and Ctrl+Enter. Seven new assertions
failed against compiled old behavior; all eight final guard tests pass. The
full Linux runtime checkpoint `b54b2c28` measured **7,959 passed, one inherited
terrain GPU precision failure, 52 ignored**, with all **143 required names**
passing once. Final `6b512b1a` differs only in workflow/documentation; its runtime
source blobs are identical. Exact-source formatting, Linux Clippy with the
existing platform dead-code qualification and normal dev build passed. The
normal build took 338.894 seconds, from clean source, and reports
`0.0.0+1567.g6b512b1aa`, SHA-256
`ef61dd32a4bb2bbe831b2c241b0c3b568fe10226244bdc1572bd54f1982afc9c`.
An independent immutable executable freeze is retained.

The unchanged native CPU driver independently reproduced the old held-key
Send and then verified the fixed guard with no held intent, four eligible true
repeats before any Send, and one Send after real release/fresh Enter. That fixed
case failed later at a conversation-menu click whose live row bounds were not
retained. No production navigation defect was established. The fixture can
publish sizing-pass rectangles; the driver had sampled a row only once.

A separately reviewed driver revision changes only exact conversation-row
geometry acquisition, stability checks, pre-edge rechecks and original captures.
With the same fixed `b54b2c28` test ELF and all original text/intent/Enter criteria,
its full scripted native Linux CPU case **passed 15 assertions**, exactly **three
synthetic Send intents and one Read intent**. It exercised held/fresh Enter,
Shift/Ctrl, source/conversation drafts, captured Read, native CloseRequested/
reopen retention and held-Read source replacement. Three eligible true repeats
preceded physical release in the final trace; the earlier held screenshot/state
contains two. Checked test and wrapper both exited **0** after **64.407 seconds**;
no forced stop or live owned process/display remains. The terminated Xvfb zombie
awaits PID1 reaping. The popup was visibly above its selector in this new case;
that does not reconstruct the old failed click's press-time placement.

[Five unchanged native CPU captures and hashes](https://github.com/DrCum/Foldurora/blob/5ab5d407e7735af206b05685bc064bb81945ab7b/foldurora/20261107-b54b2c28-native-chat-input/README.md)
retain the actual source identity: `0.0.0+1565.gb54b2c28f`, test ELF SHA-256
`dd07fca80a79a0fff67c140a86af704974e7ab2f7072b6f99fe2e4af4246e77e`,
driver `068eb588` importing frozen `b85909b1`. These are real X11/CPU-window inputs
with synthetic policies and typed intents, not viewer authentication, real chat
IPC/World admission, message delivery, native unread mutation or grid acceptance.
The controlled RejectedNative statuses visible in captures are fixture receipts.
Windows, physical GPU, real IME, window-manager minimize/caption-button, released-
away/refocus and dropping-Shift native cases remain separate checks. The original
NVIDIA device-loss report remains unresolved. Attention and monitors follow this
foundation in the user's chosen order.

The build worktree retains `native-enter-unit-negative-receipt.json`,
`native-enter-fixed-workspace-receipt.json`,
`native-enter-final-clippy-receipt.json` and
`native-enter-final-normal-binary-receipt.json` under `.scratch`. The fixture
worktree retains the negative and partial cases plus
`.scratch/native-os-driver/batch-p7oevWRn/VALIDATION-B54B-STABLE-ROWS-NATIVE-CPU-RECEIPT.json`.

## Earlier camera and Shared Chat checkpoints

The verified Shared Chat Windows download is
[`1ef87bf6`, artifact 13566087312](https://github.com/DrCum/Foldurora/actions/runs/42348714309/artifacts/13566087312).
The newer unread/shutter follow-up is frozen at
[`0e40e48f`](https://github.com/DrCum/Foldurora/commit/0e40e48f631b1b4fdd0d677013e07b83a4f5fc61)
on `feat/native-read-integration`; its separate
[Windows run 42352272886](https://github.com/DrCum/Foldurora/actions/runs/42352272886)
and [artifact 13567703046](https://github.com/DrCum/Foldurora/actions/runs/42352272886/artifacts/13567703046)
are verified builds. **A Linux native held-Enter/refocus defect was subsequently
reproduced; native acceptance is withheld and its release-guard fix is separate.**
The older camera and Shared Chat packages remain frozen; do not mix their
executable/protocol or validation receipts.

The frozen camera source is [`6a3009f2`](https://github.com/DrCum/Foldurora/commit/6a3009f27a2a5e49e61ea0fdeb1f7e89e3eb2fa8),
on `fix/camera-controls-integration`. Its [Windows run 42343210582](https://github.com/DrCum/Foldurora/actions/runs/42343210582)
and [artifact 13563094085](https://github.com/DrCum/Foldurora/actions/runs/42343210582/artifacts/13563094085)
are verified: **7,879 passed, 0 failed, 50 ignored**, with all 90 required
regressions passing exactly once in both the packaged and raw job logs. Parser,
formatting, strict Clippy, viewer build and packaging passed. Both archive CRC
checks, manifest source, AMD64/PE32+ identity, executable version and all seven
embedded icon payloads match. This package has the two known follow-ups below.
The separate local Windows builder reports the same version and test counts,
passing parser/format/strict Clippy, and a clean pinned source. Its owned offline
demo exercised controls, photo saving, resizing, 110% UI scale and both toolbar
scopes, then quit normally with exit 0. Alt+wheel remains manual there because
the local input API cannot hold Alt while scrolling. The local ZIP/executable
hashes differ from CI and remain a separate reported receipt; its native bundle
and extra observations have not been independently inspected here. Neither
report resolves the frozen-source follow-ups below or establishes grid/physical
multi-monitor acceptance.
The earlier `388b299c` download below predates it.

The CI package version is `0.9.3-beta.20261106.1+168.g6a3009f27`.
Portable ZIP SHA-256: `ee833219792651001e9e5d5b1abc1ae7192a964dd341f30713d3626f5e66e7de`.
Executable SHA-256: `ea01f2212fe42f572cb04abcb81ea0699086ef2c6e145a98c12c4b980912d1ca`.
The source-worktree records are `.scratch/windows-ci-42343210582/receipt.json`
and `icon-resource-verification.json`; these are CI results, independent of any
local Windows rebuild or human/grid acceptance.
The local-builder report is retained separately as
`.scratch/windows-native-6a-local-builder-reported-receipt.json`.

Final Linux formatting/diff checks, lint with the existing Linux dead-code
qualification and normal build passed. Full Linux tests: **7,907 passed, one
inherited terrain GPU precision failure, 50 ignored**; all six camera regressions
passed, including narrow/unrestricted header input.

An owned offline software-Vulkan demo exercised actual mouse and keyboard input,
not application test hooks: guide selection, ordinary wheel zoom with Free camera
OFF, Alt+wheel focal changes, ring wheel and minus/plus after Settings, aperture,
focus, looks, a real 640×360 photo/readback and later input. The exact normal
Linux executable was `0.0.0+1542.g6a3009f27`, SHA-256
`41c0992e7b57ea6a808b8c7f5899133548d04cdb516ba4799d5f1dd31b06c01e`.
Confirmed normal Quit returned 0 after 617.289 seconds, wrapper 0; all owned
processes and the private display exited. [Unchanged source-scoped captures](https://github.com/DrCum/Foldurora/blob/2d9cca8cd066e7da8e6db41fe8925144507e4a8d/foldurora/20261107-6a3009f2-input/README.md)
show the observations. The source-worktree receipt is
`.scratch/camera-input-smoke/batch-vdrD0CAr/VALIDATION-6A-RECEIPT.json`.

**Frozen 6a still has two observed follow-ups:** its top dock overlaps part of the
histogram, and a quick UI click can also relocate the focus point before the next
egui redraw. The guide cycles, but the AF brackets move to its click while MF
3.3 m remains unchanged. These are not described as fixed in 6a, nor is this a
Windows/physical-GPU/grid acceptance result. Corrections are integrated only in
the later combined Shared Chat work. Its new current-pointer regression first
failed against the old helper; all seven camera checks pass after the small
position-aware exclusion correction.

The local `feat/shared-chat-integration` candidate builds on 6a and retains those
camera corrections. It adds one explicit sending-character/conversation surface,
five bounded canonical composers and ordered close/navigation/IME/repeat guards;
no native protocol or read/Attention/monitor capabilities are added. The first
combined full run exposed two Shared Chat issues: prefix edits hid selector hit
maps, and native repeated Enter could lose its repeat flag in egui. Reviewed
corrections passed the `7b2298e8` runtime checkpoint: all **139 host/chat tests**
and **seven camera regressions** pass. The full Linux result is **7,925 passed,
one inherited terrain GPU precision failure, 51 ignored**. Thirty EN/FR/JA
production CPU captures passed required-text/privacy checks; selected normal and
narrow screens were inspected. [Selected unchanged CPU captures](https://github.com/DrCum/Foldurora/blob/916081ff/foldurora/20261107-7b2298e8-shared-chat-cpu/README.md)
are source-scoped. These captures do not exercise native OS delivery
or a live grid. Independent source review found no concrete blocker.

Exact-source final checks/build completed at
[`1ef87bf6`](https://github.com/DrCum/Foldurora/commit/1ef87bf6d4cfbe54c071009539cdc99cca4d4f34),
which adds two lint-equivalent expressions and documentation. The final full
Linux `--no-fail-fast` run again reports **7,925 passed, one inherited terrain
precision failure, 51 ignored**, with all **109 required camera/chat regressions**
passing once. Formatting and Linux Clippy with the existing platform dead-code
qualification passed; the normal dev build passed in 297 seconds from clean
tracked source. Its executable is `0.0.0+1553.g1ef87bf6d`, SHA-256
`1c8c2caa79275caaa9607010e5b235d1ce7ace6dde3033c91ab0e9135f768f1f`.
The exact-source records are `.scratch/shared-1ef-full-workspace-receipt.json`
and `.scratch/shared-1ef-final-binary-receipt.json`. An earlier default fail-fast
run stopped at the inherited renderer failure; it is explicitly partial and is
not used as this full receipt.

The source is pushed on `feat/shared-chat-integration` and its
[Windows run 42348714309](https://github.com/DrCum/Foldurora/actions/runs/42348714309)
is fully verified: **7,897 passed, 0 failed, 51 ignored**, with all **109 required
names** passing once in both raw and packaged logs; the four obsolete test names
are absent. Parser, formatting, strict Clippy, build and packaging passed.
Manifest source, both ZIP CRCs, AMD64/PE32+ identity, executable version and all
seven embedded icon payloads match. Its version is
`0.9.3-beta.20261106.1+179.g1ef87bf6d`.
Portable ZIP SHA-256: `589620183735ef6bc0f51cf799bd21c591f2ad6687bf082973bb89bd9c1d440f`.
Executable SHA-256: `6e3cf0aa3d330947d5807bbc502c530ff69ec399de5e2718379c3d8e9118d933`.
The retained records are `.scratch/windows-ci-42348714309/receipt.json` and
`icon-resource-verification.json`. Native Shared Chat and human grid acceptance
remain pending, and this package does not contain the later unread/shutter work.

The exact `1ef87bf6` executable completed one owned Linux/software-Vulkan camera
case with normal viewer/wrapper exit 0 after **384.472 seconds**. The histogram
clears the dock, and guide/Settings clicks queued without a settled UI hover
leave the AF point centered. Actual Free camera OFF, ordinary wheel, Alt+wheel,
ring, focus/aperture and post-photo minus/plus responses were observed. A
640×360 RGB export succeeded. [Unchanged native captures and limits](https://github.com/DrCum/Foldurora/blob/b7c676e0/foldurora/20261107-1ef87bf6-camera-input/README.md)
belong to this source only; this is not Windows, physical-GPU or grid acceptance.

Its **first shutter click produced no observed export for about 30 seconds**;
another click after a fresh hover succeeded. The case did not instrument event
boundaries inside an egui frame, so the cause remains unproven. There is no
blanket shutter-input pass. Source inspection found a separate reproducible
completed-primary-click defect: press and release can both arrive between UI
frames. Its small correction and actual-egui regressions are in the later
candidate, absent from frozen `1ef87bf6`. All owned viewer/driver processes and
the display socket are gone; two terminated Xvfb zombies await PID1 reaping.

The separate pushed `feat/native-read-integration` candidate contains the native
unread/explicit-Read first slice and that shutter correction. It preserves both
native chat styles' read-on-draw timing/defaults, separates unread observations
from draft/Send authority, and keeps immutable body-free receipts/query recovery.
Only the separate chat protocol becomes V2; control stays V1. Attention,
automatic host reading, new conversations and monitors are absent. Independent
source reviews cleared the Read and shutter changes.

A compiled full-workspace negative control reproduced **all seven targeted
runtime assertion failures**: three Read admission/navigation/edit-prefix races
and four shutter cases. Its measured result was **7,944 passed, eight failed,
51 ignored**; the only other failure was the inherited terrain precision test.
No compile error is counted as behavioral proof. The positive full suite at
checkpoint `08099610` completed with **7,951 passed, one inherited terrain
precision failure, 51 ignored**; all **135 required regressions** pass, including
the seven unchanged negative-control fixtures. Final frozen source `0e40e48f`
adds only one lint-equivalent compound receipt-validation guard. Its exact-source
formatting, Linux Clippy with the existing platform dead-code exception, normal
dev build and **48 EN/FR/JA CPU captures** passed. Selected normal/narrow Read
pending, stale and uncertain screens were inspected. Its Linux executable is
`0.0.0+1561.g0e40e48f6`, SHA-256
`825e200e36e40e7519a38e8eaf17fd2429d2d6b08045075c93d3b8d9377d648e`.
The full-runtime and final-build receipts preserve that source distinction.

One narrow native Linux/software-Vulkan test on the exact frozen executable
started with zero exports and a clear-scene hover, then performed **one** fast
shutter burst without a settled shutter hover. One valid **640×360 RGB** photo
was observed within **2.063 seconds**, with no retry. Normal viewer/wrapper Quit
returned **0** after **222.917 seconds**. No live owned process/display remains;
the terminated Xvfb zombie awaits PID1 reaping. No actual same-egui-frame boundary,
OS input latency, Windows, physical-GPU or grid acceptance is inferred.
[Unchanged native and CPU captures](https://github.com/DrCum/Foldurora/blob/4c128f8e/foldurora/20261107-0e40e48f-read-and-shutter/README.md)
keep those scopes separate. The source-worktree records are
`.scratch/native-read-final-binary-receipt.json`,
`.scratch/native-read-visual-latest.json` and
`.scratch/shutter-input-smoke/batch-BpdunVpO/VALIDATION-0E40-SHUTTER-RECEIPT.json`.

The exact `0e40e48f` Windows build is verified: **7,923 passed, 0 failed, 51 ignored**,
with all **135 required names** once in raw and packaged logs and four obsolete
names absent. Parser, format, strict Clippy, build and packaging passed. Manifest,
both archive CRCs, AMD64/PE32+ identity, version
`0.9.3-beta.20261106.1+187.g0e40e48f6` and all seven embedded icon payloads match.
Portable ZIP SHA-256: `be35821bb7ea16caaeb768dd77549706532ce4af33c17c76f3608f381eaa8c11`.
Executable SHA-256: `b8e33c38fc7b7186a06e103a4e4466be7e4c0a9555b047687a6b4fcc30406570`.
The receipt `.scratch/windows-ci-42352272886/receipt.json` explicitly records the
later native finding; build success is not full native acceptance.

A separate Linux test-only fixture runs actual ChatWindow/egui-winit/CPU input
against synthetic snapshots and a bounded typed-intent inbox. It starts no real
controller broker, profile, registry, socket, worker, authentication or World
submission. The first attempt did not acquire editor focus before typing and is
a failed setup, not input acceptance. A driver-only focus/click separation then
satisfied actual selected-editor focus, editable/current Send permissions,
nonempty draft and no Pending/earlier intent before the repeat scenario.

That unchanged-production fresh case reproduced an **unintended synthetic Send
intent while Return remained physically held across focus loss/refocus**. Winit
reported a synthetic release/press for focus changes, then labeled the first held
keydown after refocus as a non-synthetic **repeat=false** press. The first
repeat=true event arrived only after the unexpected intent had consumed the
draft; the test correctly rejected that masked evidence. No World send or grid
delivery occurred. Both failed cases stopped with precise partial receipts and
owned cleanup; no native keyboard/full-Chat acceptance is claimed.

The failing fixture source is test-only `57184a5f` (equivalent to author `58d473ad`
on product `0e40e48f`). Its fresh-case evidence is retained under
`/workspace/work/shared-chat-native-os-fixture/.scratch/native-os-driver/batch-loJEJB0j`,
including the indexed Enter/focus trace, driver Return inputs and
`FAILED-57184-NATIVE-CPU-RECEIPT.json`. The later native Enter release-guard
candidate and the unchanged-driver narrow negative/fixed comparison are recorded
above. The subsequent full scripted case uses only the separately documented
driver geometry revision. Windows-specific input, IME, native World behavior and
human-managed grid acceptance remain separate. Attention and monitors follow
this foundation.

## Upstream catch-up refresh

The sixteen reviewed imports from upstream snapshot `576dd54a` remain included
in the current candidate; twenty-five other commits retain the dependency plan
in its `docs/FOLDURORA.md`. A read-only refresh at 09:34 UTC found upstream main
`8d360a2f`, exactly two newer commits: clicked sit offsets (`c6a4d06c`, #2784)
and object linking (`8d360a2f`, #2785). Their source review is complete; neither
is an intact text-clean cherry-pick on `6b512b1a`, and neither is included there.
A bounded sit-offset port is source-clear after preserving the existing
`select_for_menu` call in one unchanged context line; all seven production/test
files then apply-check clean, without enabling the deferred selection changes.
It still needs its own compiled camera/RLVa/viewport and human sit checks.

Linking assumes extracted shared eligibility helpers plus both compact and
Legacy Link-button gates from deferred `3ce55eba`. Resolving its module conflicts
alone would leave buttons enabled where the new keyboard/menu checks refuse.
An intact import belongs with the rights/build batch; a smaller linking-only
backport is possible only if it also ports those two gates and validates both
styles, property freshness, packet batching, replacement order, RLVa and host
modal behavior. Source/applicability checks establish no new runtime acceptance.
The exact patch review and checklist remain privately retained under
`/workspace/scratch/upstream-2784-2785-sol/REVIEW.md`.

The deferred cache/rights chain `026651a8` → `316e5e06` → `1ae9a375` → `3ce55eba`
was rechecked against `6b512b1a`; none of the four whole patches independently
applies cleanly. Its concrete plan preserves worker-profile-before-settings/
private-key initialization and per-process cache ownership, requires atomic
no-clobber migration, updates the fork's asset-ID cache to the shared effective
rights with invalidation, and fences the new no-copy modal against host/source
changes. Treat the last three commits as one acceptance unit: the intermediate
worn-texture behavior is weaker than the final base/attachment-lock checks.

Twelve cited implementation/header files were independently fetched from the
Legacy source pinned at `abbe3cb0`. `LLPanelFace`'s copy/transfer worn filters,
`LLToolDragAndDrop::willObjectAcceptInventory`'s unrestricted base requirement
and texture-application/RLVa attachment checks support the final cumulative
worn policy. Whole-chain parity still requires resolving two specific gaps:
nonnil requester with nil foreign/task owner must not gain owner rights over
`LLPermissions::allowOperationBy`'s everyone fallback; seated-root `@unsit`/
`@sittp` refusal must remain alongside attachment locks. No rights/runtime change
was introduced by this review. The detailed source plan, apply-check logs and
pinned file manifest remain in the isolated `cache-rights-integration-review`
worktree's `.scratch` directory.

The user-selected implementation order is **compact host redesign → shared chat
→ Attention → monitors with chat**. The compact host is implemented at frozen
checkpoint `399ffd79`, with Linux source, synthetic visual and owned EN CPU UI
evidence below. Its verified package remains an earlier compact checkpoint;
the verified Shared Chat download is `1ef87bf6` above, with its stated limits.
The earlier
`70c1b5cd` package has the preceding host layout. Native Windows
profile/draft and compact-host acceptance remain pending. Branding and photo-mode
viewport work are separate in-progress follow-ups, not fixes included in `399ffd79`.

Frozen source checkpoint `f5ae21d5` combines the branding, photo-viewport and
narrow-preview follow-ups on the pushed `feat/foldurora-photo-branding` branch.
Source review, formatting, diff checks, final Linux Clippy and the normal build
passed. Linux tests reported 7,870 passed, one inherited terrain failure and
50 ignored; all 15 new photo/branding/UI regressions passed. Its exact-source
Windows package is verified: 7,842 passed, 0 failed, 50 ignored, all 51 named
regressions passed once, and packaged icon resources match the source ICO.
Owned software-Vulkan checks
support bounded landscape, portrait and Interface exports plus the native Linux
icon. Restricted 480-pixel viewfinder controls still overlap and clip.
Native Windows photo/export/icon acceptance remains pending. The local builder has since received frozen camera `6a3009f2`; that build and
its scoped offline native checks remain separate from this package.
These source changes must not be attributed to the compact `399ffd79`
package.

The verified frozen upstream candidate is
[`388b299c429d776b3fc7a9890162cd3d940b9dc3`](https://github.com/DrCum/Foldurora/commit/388b299c429d776b3fc7a9890162cd3d940b9dc3)
on pushed `feat/foldurora-upstream-integration`: frozen `f5ae21d5` plus 16
reviewed upstream commits, leaving 25 for staged integration. Provenance, source
review, formatting/diff checks and Linux Clippy passed (34.70s). Full Linux tests
reported 7,902 passed, one inherited terrain precision failure and 50 ignored;
new HUD/cache and retained regressions passed. Its normal binary built in 4m35s.
Bounded software-Vulkan landscape/isolation and HUD visibility checks passed;
landscape/isolation exited normally, but the HUD harness reached its deadline
with Quit confirmation open, so normal HUD Quit acceptance is unproven.
Its exact-source [Windows artifact 13561798593](https://github.com/DrCum/Foldurora/actions/runs/42337795785/artifacts/13561798593)
is verified: 7,874 passed, 0 failed, 50 ignored; all 85 required named regressions
passed exactly once. Parser, formatting, strict Clippy, tests, build and package
succeeded. Native Windows/GPU/grid acceptance remains pending. A separate local builder has received the later camera `6a3009f2` prompt;
its results do not establish an additional local388 build.

This package predates the camera-control/input follow-up reported against local
`f5ae21d5`: the right-side scene is bounded, but the vertical dock is centered and
wheel/ring input can stop responding. A separate camera-control fix is undergoing
integration tests. Shared Chat is also being authored independently; neither is
included in `388b299c`. The exact control sequence preceding the human's failure
is unknown and is not required for the reproduced header hit-region defect.

The current verified Windows package is
`Foldurora-test-388b299c429d-windows-x64.zip`, version
**`0.9.3-beta.20261106.1+155.g388b299c4`**, from successful run
[42337795785](https://github.com/DrCum/Foldurora/actions/runs/42337795785), completed
7 November 2026 at 05:19 UTC (00:19 America/New_York). Artifact test output and
independent raw job logs agree on **7,874 passed, 0 failed, 50 ignored** and all
**85 required named regressions** exactly once. The two masked-HUD regressions
are included. Both manifests, portable archive checksum/CRCs, AMD64/PE32+
executable, executable/install version strings and all seven embedded icon frames
were verified against the exact source ICO.

- Outer artifact SHA-256: `9cc8b2a9e6160cbcabe88b7ff08e86f366e09079ec63849bc9006120b30de4b7`.
- Portable ZIP SHA-256: `f5f0c84ec350f0e3d410ccdd26be8830ffb36a31a0cff4a8ca36fae108e72905`.
- Executable SHA-256: `66bbcb2a0cf1706775565625246f6f8ffa0989d17b3b10f6b6aae2c5c334d6da`.

The retained source-worktree receipt is
`.scratch/windows-ci-42337795785/receipt.json`, with the downloaded package,
raw logs and `icon-resource-verification.json`. This verifies the portable build;
it does not establish native Windows acceptance or resolve the later camera bug.

## Frozen test candidates

| Source | Purpose | Evidence | Remaining acceptance |
| --- | --- | --- | --- |
| `d10473efff52ceaea2d7b22d111319b0efc4f28e` | Combined upstream, viewport and session-host/chat baseline | Local Windows checks: 7,791 passed, 0 failed, 49 ignored; build/package passed. [GitHub run](https://github.com/DrCum/Foldurora/actions/runs/42291668355) passed. | Human confirmed toolbar containment and full-workspace floaters; reported oversized menu, host drag/detach failure and one standalone switch device loss. |
| `8b738ea8b2c700b7538b7e16135a7de19c192d43` | Host movement/recovery and compact dropdown fixes | [Windows run 42304532039](https://github.com/DrCum/Foldurora/actions/runs/42304532039) passed: 7,798 tests passed, 0 failed, 49 ignored; all seven focused regressions passed. Scripts, formatting, Clippy, normal viewer build and packaging passed. Downloaded artifact, manifest, checksums, executable version and Windows x64 identity were verified. Separate native offline passes were reported for a local rebuild below. | Caption-button appearance, unexplained forced-loss client-height change and missing capture/probe bundle limit native verification. Native results belong to the local rebuild, not the CI executable or later candidates. Separate standalone GPU loss remains unresolved. |
| `8cd9a2aa005491b651e5bb8591d94fd6fb891520` | Combined backend/profile checkpoint: host/menu fixes, retained conversation drafts, crash details fallback and five persistent renameable slots | [Windows run 42311181216](https://github.com/DrCum/Foldurora/actions/runs/42311181216) passed: 7,818 tests passed, 0 failed, 49 ignored. Scripts, formatting, strict Clippy, normal build and packaging passed; downloaded package identity, manifest and checksums were verified. | This checkpoint has a known profile-popup margin overlap corrected by `70c1b5cd`. Native host/profile/chat and human grid acceptance remain pending. |
| `70c1b5cda927547b28e5a5ff9713e861771aaee7` | Earlier verified test candidate, `fix/profile-popup-bounds`: bounds the profile popup including its frame and wraps occupied-profile help inline | [Windows run 42315300611](https://github.com/DrCum/Foldurora/actions/runs/42315300611) passed: 7,819 tests passed, 0 failed, 49 ignored; all 28 applicable named regressions passed exactly once. Scripts, formatting, strict Clippy, normal build and packaging passed. Downloaded manifest, checksums and executable/PE identity were verified. Fresh Linux CPU UI and sequential demo-worker evidence is recorded below. | Offline native Windows host/profile/chat, human grid, physical GPU/multi-monitor and performance acceptance remain pending. |
| `399ffd796ccba8e9468bbc9ff7b19eff5017d4bc` | Earlier verified compact package, `feat/compact-host-integration`: compact strip, full-width world, explicit management drawer and popup islands | [Windows run 42327859777](https://github.com/DrCum/Foldurora/actions/runs/42327859777) passed: 7,827 tests passed, 0 failed, 50 ignored; all 36 applicable named regressions passed exactly once. Parser, formatting, strict Clippy, tests, build and packaging passed. Downloaded manifests, checksums and executable/PE identity were verified. Linux and visual evidence is recorded below. | Native popup-island composition/hit testing, world input/handoff/recovery, profiles/drafts, physical GPU/multi-monitor, performance and human grid acceptance remain pending. Earlier local `8b738ea8` native results do not transfer to this binary. |
| `f5ae21d535da3a338bace51ffb03af7beeb040c6` | Earlier verified Windows download: frozen photo/branding source, including compact host and narrow scene/output/preview corrections | [Windows run 42334134158](https://github.com/DrCum/Foldurora/actions/runs/42334134158) passed: 7,842/0/50, all 51 named regressions once, all checks/build/package successful. Manifest, hashes, CRCs, executable identity and seven PE icon frames verified. Linux: 7,870/1 inherited terrain/50; normal build and bounded software-Vulkan exports/icon checks passed. | Restricted 480-pixel control overlap/clipping remains. Native Windows host/photo/export/icon, physical GPU, performance and human grid acceptance remain pending. A separate local build completed; the human reported the camera-control/input bugs described above. |
| `8750450f0161e03fe6ef48194aa58b28d49a2cb9` | Preserved 15-import upstream baseline | Linux Clippy passed (1m29s); full tests: 7,900 passed, one inherited terrain failure, 50 ignored. Its completed Windows test step reports 7,872/0/50 and all 83 named tests once before expected same-branch cancellation. | Viewer build was cancelled and packaging skipped; no usable `8750450f` package. Superseded by `388b299c`, not a failed-test result or validation of the final source. |
| `388b299c429d776b3fc7a9890162cd3d940b9dc3` | Final frozen, pushed 16-import source, `feat/foldurora-upstream-integration`; 25 upstream commits deferred | Source/provenance proof, formatting/diff checks and Linux Clippy passed (34.70s). Full Linux tests: 7,902 passed, one inherited terrain precision failure, 50 ignored; new HUD/cache and retained regressions passed. Normal build passed (4m35s). [Windows artifact 13561798593](https://github.com/DrCum/Foldurora/actions/runs/42337795785/artifacts/13561798593) is verified: 7,874/0/50; all 85 required named tests once, all checks/build/package successful. Manifests, hashes, CRCs, executable identity and seven icon frames match. | Exact-binary software-Vulkan landscape/isolation and HUD visibility passed. Landscape/isolation exited normally; a 240-second guard interrupted HUD Quit confirmation, so normal HUD Quit remains unproven. Restricted controls still crowd/clip. Native host/profile/photo/icon/grid acceptance remains pending; this package predates the separate camera-control/input fix. Optional local-build delivery/start are unconfirmed; earlier source receipts do not transfer. |

Keep these sources and their packages separate. No candidate is merged into main, installed
or released by this work. Both the verified `70c1b5cd` package checkpoint and
compact `399ffd79` source remain frozen while follow-ups proceed separately.
This ledger is updated on
`docs/foldurora-progress-20261106`; its documentation changes do not alter the
runtime candidates or their validation results.

The verified host/menu [Windows artifact 13548583765](https://github.com/DrCum/Foldurora/actions/runs/42304532039/artifacts/13548583765)
contains `Foldurora-test-8b738ea8b2c7-windows-x64.zip`. Its manifest names the exact
`8b738ea8` source, successful checks/build/package and pending human, Windows GPU,
multi-monitor and grid validation. The executable and PE version strings agree on
`0.9.3-beta.20261106.1+110.g8b738ea8b`; its machine identity is AMD64/PE32+.

- Downloaded artifact SHA-256: `899ca31cfe316081bdf39483166afa1c2331ad0a2b2c8756362be73114199325`.
- Inner portable ZIP SHA-256: `1b1b7f46ee8810d4b83de7c7bee05210d7e8971c2b2ad830b2da3f78dc46fb7c`.
- Executable SHA-256: `533a3b92217288bf7cac2e366585623c95e9e52fce88511e1a94b5dafe1380bd`.

The retained receipt is the candidate worktree's
`.scratch/windows-ci-42304532039/receipt.json`, alongside the downloaded manifest,
artifact and raw logs. An earlier CLI download was blocked; the later connector
download and content verification completed successfully. Native behavior is not
established by those checks.

The verified backend/profile [Windows artifact 13550927031](https://github.com/DrCum/Foldurora/actions/runs/42311181216/artifacts/13550927031)
contains `Foldurora-test-8cd9a2aa0054-windows-x64.zip`, built for
`x86_64-pc-windows-msvc` in the development profile with `xr-static-loader`.
The manifest names exact source `8cd9a2aa`, successful checks/build/package and
pending human, GPU/multi-monitor, grid and performance acceptance. Executable
and PE version strings agree on `0.9.3-beta.20261106.1+118.g8cd9a2aa0`, with
AMD64/PE32+ identity. All seven host/menu and 20 applicable profile/draft/diagnostic
regressions passed; the Unix-only linked-path regression is absent as expected.

- Downloaded artifact SHA-256: `aba9afbcebdcda0a0f3be6407a2f30a4ec43aa9297a6febd4684f6a0efd3dbcd`.
- Inner portable ZIP SHA-256: `5bbf8afa07904521eda9ea7598f37c2b9b55764d984527a64efae39909fa90ba`.
- Executable SHA-256: `086e0724ac515cf9cc050eeee4c36c6c4cae49c71de1593c8490ee1d4028fe5b`.

Its retained receipt is `.scratch/windows-ci-42311181216/receipt.json` in the
candidate worktree. The known popup-margin follow-up is not included in this
verified package; use the separate `70c1b5cd` candidate for the corrected UI.

The earlier verified [Windows artifact 13552619352](https://github.com/DrCum/Foldurora/actions/runs/42315300611/artifacts/13552619352)
contains `Foldurora-test-70c1b5cda927-windows-x64.zip`. Its manifest records exact
source `70c1b5cda927547b28e5a5ff9713e861771aaee7`, successful checks/build/package,
`x86_64-pc-windows-msvc`, development profile and `xr-static-loader`. The raw
32 test summaries total **7,819 passed, 0 failed, 49 ignored**, with all seven
host/menu, 20 applicable profile/draft/diagnostic and one popup-bounds regression
passing exactly once. The Unix-only linked-path test is absent as expected.
The executable and PE strings match **`0.9.3-beta.20261106.1+120.g70c1b5cda`**,
with AMD64/PE32+ identity. The artifact was created at 00:57:37 UTC on
7 November 2026; that is still 6 November in America/New_York.

- Downloaded artifact SHA-256: `99cfb245f9a7240ad580a1f070bc8b0130a18ed9d4f15dee180e514ed60be507`.
- Inner portable ZIP SHA-256: `0e4b22c799570bf26bf0c4f9365c98f2778f3479877bb85651ca5a34ea5ace27`.
- Executable SHA-256: `a391546c3adccbd256ca64064c7a0fdf22d41b05f8a15f51091cea46609cdac8`.

The retained receipt is `.scratch/windows-ci-42315300611/receipt.json` in the
candidate worktree. The manifest still marks human, Windows GPU/multi-monitor,
grid and performance acceptance pending. The package is a development test
candidate, not a release or native runtime acceptance result.

The earlier compact [Windows artifact 13558425977](https://github.com/DrCum/Foldurora/actions/runs/42327859777/artifacts/13558425977)
contains `Foldurora-test-399ffd796ccb-windows-x64.zip`, version
**`0.9.3-beta.20261106.1+128.g399ffd796`**, exact source
`399ffd796ccba8e9468bbc9ff7b19eff5017d4bc`. Run 42327859777 completed successfully
at 03:10 UTC on 7 November 2026. Its manifest reports parser, formatting, strict
Clippy, tests, build and package success, with Rust 1.99.1, Windows x64,
development profile and `xr-static-loader`.

The raw artifact tests and independent job logs agree across 32 summaries:
**7,827 passed, 0 failed, 50 ignored**. All **36 applicable named regressions**
passed exactly once: seven host/menu, 20 profile/draft/diagnostic, one compact
popup-bounds, three overlay and five compact UI tests. The Unix-only linked-path
test is absent as expected; the CPU visual fixture is intentionally ignored.
Outer/inner manifests, install/executable version resources, AMD64/PE32+ identity,
recorded ZIP checksum and all 20 inner ZIP-entry CRCs were verified.

- Downloaded artifact SHA-256: `614a3cf8cab1e12d84ec626590acb53b39796dbfa35802ba0317c50e7a394a69`.
- Inner portable ZIP SHA-256: `f21e3a4c4cdb1637ff449258df2eca055c7ff872b3a23fc6fb9a470091705910`.
- Executable SHA-256: `82a802917a0e5324f23a5e692b1763e44f875f72a197a532397ff5c96d5bc039`.

The retained receipt is `.scratch/windows-ci-42327859777/receipt.json` in the
candidate worktree; the lead's independent check is `root-verification.json`
beside it. This verifies the compact test package; native Windows,
GPU/multi-monitor, grid and performance acceptance remain pending. It does not
contain the separate photo/branding follow-up described below.

The earlier verified [Windows artifact 13560192057](https://github.com/DrCum/Foldurora/actions/runs/42334134158/artifacts/13560192057)
contains `Foldurora-test-f5ae21d535da-windows-x64.zip`, version
**`0.9.3-beta.20261106.1+136.gf5ae21d53`**, exact source
`f5ae21d535da3a338bace51ffb03af7beeb040c6`. Parser, formatting, strict Clippy,
tests, build and package passed. Artifact and independent raw job logs agree on
**7,842 passed, 0 failed, 50 ignored** across 32 summaries; all **51 named
regressions passed exactly once**. Manifest target is Windows x64, development
profile, Rust 1.99.1 and `xr-static-loader`.

Outer/inner manifests, recorded checksum, all 20 ZIP-entry CRCs, AMD64/PE32+
identity and install/executable version resources were verified. All seven
embedded RGBA32 icon payloads and group metadata match committed
`foldurora.ico` at 16, 24, 32, 48, 64, 128 and 256 pixels. This verifies packaged
resources, not their native Windows appearance.

- Downloaded artifact SHA-256: `cc9320665a51b328e7df04aab6a8462e1b405035c2d6f4da06c2ca69591a1130`.
- Inner portable ZIP SHA-256: `e7b8abc454c969b65875871fef6941929c35cd71c87fe180a2bfd231649b646b`.
- Executable SHA-256: `1a5ca8ee90683cea0340b1ea645109349d7b2ac0aec19d8cc6e86d57b0a6550b`.

Retained receipts are `.scratch/windows-ci-42334134158/receipt.json`,
`root-verification.json` and `icon-resource-verification.json` in the candidate
worktree. Native, GPU/multi-monitor, performance and human grid acceptance remain
pending. This is the current verified download, separate from final upstream
source `388b299c` and its unfinished validation.

## Reported native Windows evidence — local 8b rebuild

The attached native-results report applies to a separate local rebuild of exact
source `8b738ea8b2c700b7538b7e16135a7de19c192d43`, version
`0.9.3-beta.20261106.1+110.g8b738ea8b`, Rust 1.99.1, development profile and
`xr-static-loader`. It reports the same 7,798/0/49 test result and seven focused
regressions. Its binary/package hashes differ from the CI artifact above:

- Local package ZIP SHA-256: `b10f42484cf4a1a632867a4780bad09fcf20bdb6ef6bb2b340e9cd9c65e34f05`.
- Local executable SHA-256: `12616c5bc9bded71a38fa0271f6aa8d91a9bb793730ad345785951175aa6783d`.

Reported human/offline passes cover continuous native title-bar dragging away
and back, subsequent aligned A/B switching, resize and held movement/camera input
release; explicit detach to responsive taskbar/Alt+Tab-selectable full worlds;
compact menus; minimize followed by reactivation; normal confirmed host closure
with Detach sessions and close; and normal worker Quit with actual process exits.
Six synthetic demo workers produced no reported device-loss/recovery matches.

After forced controller loss, both workers were eventually visible and usable.
The delayed sample was 55.73 seconds after the immediate probe; actual recovery
latency was not measured. One client's delayed size was 960×568 rather than its
initial 960×640, unexplained despite the human reporting full usable worlds.
Caption-button appearance remains unresolved; functional minimize does not
establish visual correctness. The report names relative screenshot/JSON evidence
files, but that bundle was not attached for independent inspection.

These checks used low graphics and an intentional 15 FPS cap. They do not
establish normal performance, mixed-DPI behavior, live-grid/voice/chat routing,
the separate standalone GPU-loss cause, `70c1b5cd` native profiles/drafts or the
new compact UI. User raw files, local paths and private runtime data are not
republished here.

## Implemented in the combined source

The combined source provides five stable, renameable character slots.
Each slot keeps a private settings/cache location
across normal restarts from the same executable location, with exclusive profile
ownership and fresh worker/session authority at each launch. New profiles start
with defaults; earlier generated profiles are preserved. This is implemented
source, with native Windows worker/profile acceptance still pending.

It also provides separate drafts for Nearby and existing resident IMs,
retained while navigating and closing/reopening the same character chat popout.
Drafts stay in memory under the immutable worker/account/grid/session/lease source.
Permission or source loss purges affected text and editor undo state. Local queue
rejection retains text; admitted/uncertain sends are never automatically replayed.
The selected conversation, draft and receipt survive ordinary popout Close/reopen;
a failed native window creation preserves cached UI for another explicit attempt.
This does not add a shared sender selector, group chat or draft persistence across
host restarts.

A small crash-window fix also shows and copies, when no stack is available,
the sanitized diagnostic summary already generated as
`public-report.md`. It adds no collection, transmission or GPU-recovery change.

The `70c1b5cd` UI correction keeps the profile popup's full frame within the roster,
including narrow windows and long labels. Occupied-profile guidance now wraps
inline beside the disabled Open action. The earlier popup's outer margin crossed
the world cutout and could be clipped by the native Windows region; backend
profile ownership and session authority are unchanged by this correction.

Compact checkpoint `399ffd79` then replaces the permanent large roster with one
38-point control strip and a world area using the full client width. Explicit
management reveals the drawer. Tabs condense for narrow windows, and only the
exact acknowledged hosted worker is highlighted. Profile selection and typed
source-bound actions remain separate from acknowledgment. Live popup/tooltip
islands are composed over the acknowledged world cutout; their native Windows
behavior still needs acceptance. This does not implement shared Chat, Attention
or preview monitors.

## Combined validation evidence

The earlier checked Rust source is `5c741b6103328fb32813aca3a462ea930a9c441b`, including the
source-reviewed five-slot implementation from
`d08441453aa756b7b5668a4d5f21edf20e036d04`. Its Linux full-workspace run reported
**7,846 passed, 1 failed, 49 ignored**. The failure is the unchanged software-Vulkan
terrain-ramp precision case (`0.59764713 != 0.5984283`); this is not an all-green
workspace result. All **21 new profile/draft/diagnostic regressions passed**.
Formatting and workspace/all-target Clippy passed with the existing Linux
`-A dead-code` allowance. The actual normal viewer binary built in 4m31s and
reports `0.0.0+1491.g5c741b610`.

The EN/FR/JA CPU chat fixture generated **42 captures**. The six changed
draft-restoration screens were inspected at normal and narrow/150% sizes. These
drive the actual chat UI and software painter with synthetic data; they do not
open a native chat window or prove Windows behavior. Retained evidence is under
`.scratch/chat-drafts-{en,fr,ja}/` in the candidate worktree.

The actual CPU crash-helper window was also opened with a synthetic device-loss
record, and `.scratch/crash-details-fallback.png` was inspected. The fallback
summary displayed successfully. A separate egui clipboard regression verifies
the copy payload; native Windows clipboard behavior and a real GPU crash remain
untested by this fixture.

The actual native Linux CPU controller/profile UI smoke passed in EN/FR/JA on
that same `5c741b61` binary. All five numbered slots were visible, localized
Rename dialogs fit and Japanese glyphs rendered. A short ASCII label was typed
and saved through the UI in each locale. Confirmed normal closure followed by a
restart from the same executable location and private XDG roots retained each
label in both the UI and its owned registry. All six host exits were successful;
the driver and Xvfb wrapper finished. No Open login action, workers, world
renderer, grid or ordinary profile data were used. The receipt is
`.scratch/host-profile-visual/RESULTS.md`, with six inspected captures per locale
under `.scratch/host-profile-visual/run-ieeu1k9Y/{en,fr,ja}/`.

Xvfb's stale front buffer required two Expose flushes after an action before
captures reflected the current UI. An earlier apparent Rename failure was
withdrawn after correcting that observation protocol; no viewer defect or fix
is claimed from that false alarm. This smoke does not establish Windows hosted
world, taskbar, GPU coexistence or human grid behavior.

Backend checkpoint `8cd9a2aa` differs from that checked binary only in six
plain-language profile-name help/error message values across EN/FR/JA. Its
exact-source Windows package is verified above. The preceding
[`5c741b61` run 42310599389](https://github.com/DrCum/Foldurora/actions/runs/42310599389)
was automatically cancelled when the text-only follow-up was pushed. It supplies
no completed validation result and is not a failed feature-validation run.
The candidate worktree's `.scratch/profiles-validation-8cd9a2aa.json` records this
source/evidence distinction.

Latest `70c1b5cd` Linux full-workspace validation reported **7,847 passed, 1 failed,
49 ignored**, including the new regression that measures the actual popup Area
under normal/narrow widths, zoom, long labels and removal of stale guidance.
Formatting and workspace/all-target Clippy passed with the existing Linux
`-A dead-code` allowance. The normal viewer built in 2m46s, reporting
`0.0.0+1494.g70c1b5cda`; its SHA-256 is
`ab4a01e7f149fc8559d37d54b3bd040dc6c70941a978273184e609e43b1c13ff`.

The one Linux failure remains the unchanged terrain-ramp test. The exact
`0.59764713` result is consistent with 8-bit UNORM sampling precision rather
than the ideal-float reference `0.5984283`. The test and renderer are unchanged
across these candidates. The exact driver cause and any production visual impact
are unproven; no tolerance or renderer change was made and the workspace result
is not all green. The Windows test can return successfully without a usable
adapter, so its automated result alone is not physical GPU acceptance.

Fresh `70c1b5cd` native Linux CPU UI checks passed in EN/FR/JA, including current
plain-language help, all five slots, actual Rename/Save and normal restart
persistence. The popup frame stayed inside the roster at 1240×800 and 760×600,
including a 64-character ASCII label. A faint one-level RGB shadow extended two
columns beyond the frame; this is not a zero-outside-pixel claim. A simulated
private slot-5 lock showed wrapped help and disabled Open, then normal
availability after release; this case launched no worker. All seven hosts exited
normally and all owned driver/display processes finished. Receipts are
`.scratch/host-profile-visual/RESULTS-70c1b5cd.md` and
`run-NdHjIHbH/validated-summary.json` beneath that evidence root.

A separate real application IO smoke used sequential offline demo workers on
`70c1b5cd`, with private XDG roots, low graphics, 960×640 windows, a 15 FPS cap
and software Vulkan. Slot 1's actual Clock seconds preference changed from on to
off, saved to its private JSON, and remained off after confirmed Quit, actual PID
exit and reopening the same slot. Its cache-key file retained the same path and
metadata; only metadata was inspected. Slot 2 then retained its separate default
on setting and distinct cache path/key-file inode while slot 1 stayed off.
Slots 3–5 gained no settings files. Explicit Linux Detach left the viewer usable
and the profile occupied until its actual process exit. All three workers and
both controllers exited normally; no forced cleanup or device-loss marker was
needed. The receipt is
`.scratch/host-profile-visual/RESULTS-WORKER-70c1b5cd.md`, with
`worker-run-tKsGOMYz/validated-worker-summary.json` beneath that evidence root.
This establishes slot-1 retention and sequential slot-1/2 separation, not five
simultaneous sessions, sealed-cache content reuse, Windows embedding/taskbar,
physical GPU coexistence, audio/voice or real accounts. No grid, credentials or
ordinary profile data were used.

The latest actual crash-helper capture, `.scratch/crash-details-70c1b5cd.png`,
also displayed the no-stack summary from a synthetic report. It does not
reproduce a real graphics reset. The earlier 42 chat captures remain source
`5c741b61`; that chat component is unchanged through `70c1b5cd`, but those images
are not captures of the latest complete UI.

### Compact checkpoint 399ffd79

Frozen source `399ffd796ccba8e9468bbc9ff7b19eff5017d4bc` passed Linux formatting,
diff checks and workspace/all-target Clippy in 32.51s with the existing
`-A dead-code` allowance. Full workspace tests reported **7,855 passed, 1 failed,
50 ignored**. The one failure is the inherited terrain sampler-precision case
described above; it remains unresolved and the workspace is not all green.
All eight new regressions and all 6,580 viewer tests passed. The normal binary
built in 3m25s, reporting `0.0.0+1502.g399ffd796`, SHA-256
`48c43f104a987dccd0d8c204c8149735211fe819995aca7a4258e1176f507fac`.

The final locale-isolated EN/FR/JA fixture generated and checked all **21 captures**
under `.scratch/compact-host-399ffd79-8eD0V4`. It drives production `HostUi`, the
CPU painter and embedded icons with five fake workers; widget clicks return
typed intents without native dispatch. It opens no renderer, native embedding,
profile/account files or grid. Final checks inspect icon glyph pixels, translated
management-action horizontal bounds, strip/world geometry and actual popup/modal
outer areas, including long labels and narrow/150% views. Six reviewed final
PNGs are published below; earlier failed visual sets stay private.

The normal executable also passed an actual owned **EN-only native Linux CPU
controller** smoke at 1240×800: compact default, Manage open/hide, bounded
five-slot selector, real synthetic Rename/Save, confirmed Close and same-executable/
private-path restart with retained labels. Both hosts exited normally with code
0; the driver and Xvfb wrapper completed. No Open/login, workers, world renderer,
GPU or grid were used. Fourteen screenshots remain private. The bounded receipt
is `.scratch/host-profile-visual/compact-run-HauDSCAq/RESULTS-COMPACT-399ffd79.md`,
with `validated-compact-summary.json` beside it. This is not Windows embedding,
popup hit-testing, world input, FR/JA native, mixed-DPI or performance acceptance.

Exact-source [Windows run 42327859777](https://github.com/DrCum/Foldurora/actions/runs/42327859777)
passed, and its downloaded package and raw receipt are verified above. This
automated result does not establish native Windows popup hit testing, embedded
world behavior or profile/draft acceptance.

### Frozen photo/branding checkpoint f5ae21d5

The frozen, pushed `f5ae21d535da3a338bace51ffb03af7beeb040c6` source combines supplied
purple-folder artwork (`8e9e8789`), photo-guide and overlay bounds
(`f7898573`/`092c8fdf`), aligned live-world/capture scope (`dc0a4063`) and valid
narrow scene/output aspects (`477a212c`). The rounded-preview correction
(`d995b4dd`) keeps the selected full-photo framing independently of preview target
rounding, including narrow targets; fixture corrections are included at `f5ae21d5`.
Clean exports frame the bounded live world at the chosen output aspect; Interface
output preserves the complete workspace with its bounded scene, rejecting
unsupported aspect/size combinations and canceling on canvas/UI-scale changes.
These changes passed source review, corrected Linux tests, a normal build and
the bounded software-Vulkan checks below. Native Windows photo/export and
polished narrow-control acceptance remain pending.

The first full-workspace run at earlier source
`e640d4fc8524f55c6eb4568470050fb5d07f51b6` reported **7,866 passed, 4 failed,
50 ignored**. One failure is the inherited terrain case; the other three
were fixture assumptions about default UHD versus Window resolution, egui's
pending zoom and AF bracket shape representation. Those fixtures are corrected
and all 15 new regressions pass at `f5ae21d5`. That earlier
source had passed review and Linux Clippy in 34.93s.

Final `f5ae21d5` formatting, diff checks and Linux workspace/all-target Clippy
passed (34.48s, with the existing `-A dead-code` allowance). The corrected full
workspace run reported **7,870 passed, 1 failed, 50 ignored**, with all 15 new
photo/branding/UI regressions passing. The sole failure remains the inherited
terrain precision case; the workspace is not all green. The normal binary built
in **4m32s**, version `0.0.0+1510.gf5ae21d53`, SHA-256
`0a251831eeb7a8da36a279c0fe4b20ae54e75584542d598af6a23968efb2a4c3`.
Exact-source
[Windows run 42334134158](https://github.com/DrCum/Foldurora/actions/runs/42334134158)
and its package are now verified above. The latest optional local build-only
prompt now targets final `388b299c`; builder delivery/start remain unknown. The
first owned software-Vulkan demo stopped before photo mode after a six-second
startup stall in lavapipe; no capture or export was produced. That failed attempt
is preserved privately as the OBK fixture.

One approved warm task-local-cache/INFO retry, with Cargo idle, completed the two
mandatory actual 960×640-window cases normally. Restricted landscape kept its
scene/guides in the right-hand world viewport and saved a 640×360 clean PNG.
With unrestricted controls, the scene/guides stayed right-bound and saved a
360×640 portrait PNG. Inspected green-panel boundaries match the centered
guide/export transform within two landscape output pixels and one portrait
pixel, allowing blur/rasterization tolerance. The actual native 64×64 window
icon is the supplied purple folder with cyan/yellow details and transparency.

A separately approved small Interface case completed normally in about 143s,
saving the complete 960×640 workspace to 480×320. Inventory, World Map and
Preferences remain; the visible scene boundary maps x480→x240, workspace
UI/shadow (264,68)→(132,34), and the Inventory frame (13,70)→(6,35). Photo
controls are intentionally omitted from that export. Other windows obscure
much of the scene; visible boundaries and retained workspace geometry support
this bounded check. All successful applications/drivers/displays exited normally
with code 0, and all owned process groups are gone.

**Open UI limitation:** restricted 480-pixel viewfinder controls/help overlap
heavily and clip, and narrow toolbar crowding remains visible. The successful
scope/composition checks do not establish full narrow-UI usability. The earlier
cold stall is not explained by the later pass; no physical GPU-reset cause or
general performance result is inferred. No real grid, ordinary profiles,
credential-store or bootstrap/cache-key contents were used. The bounded receipt
is `.scratch/photo-branding-smoke/RESULTS-F5AE21D5.md`; raw logs/authentication/test
data remain private. These results belong to `f5ae21d5`, not the newer upstream
source. The earlier verified `399ffd79` package remains preserved separately.

### Final selective upstream source checkpoint 388b299c

Official upstream is pinned at `576dd54a707677bf5b93dd696142cc17fbdef270`.
The inventory contains 58 commits since `ae2fd1016ec83304719fa131312e2ac69c8f37a6`:
17 were already imported. The pushed source
[`388b299c`](https://github.com/DrCum/Foldurora/tree/388b299c429d776b3fc7a9890162cd3d940b9dc3)
adds **16 individually reviewed `-x` imports**, leaving **25 deferred**. The
[exact provenance and staged risk plan](https://github.com/DrCum/Foldurora/blob/388b299c429d776b3fc7a9890162cd3d940b9dc3/docs/FOLDURORA.md#selective-upstream-catch-up--7-november-2026-utc)
records every imported and remaining commit.

The batch covers session/login details, progressive UI images, RLVa Trash,
Favorites, numeric sorting, atomic teleport history, adapter retry, photo-isolation
draw metadata, explicit IM-toast focus, mesh retry, masked-HUD mip-alpha
restoration and small UI corrections.
It intentionally adopts upstream's inventory-hover master preference defaulting
off; missing/new settings require opting in to hover cards. Imported creators'
permissions guidance has an explicit fork qualification for five unavailable
helper targets; it does not claim the deferred rights/cache implementation exists.

Independent proof at `.scratch/upstream-16-root-patch-proof.json` in the root
worktree confirms every import's non-`TASKS.md` patch matches its original.
The three extra commits contain only tracking/instructions/guide/workflow changes;
no production source was manually resolved. Source review and provenance are
complete. Final formatting/diff checks and Linux Clippy passed in 34.70s. Full
Linux tests reported **7,902 passed, 1 inherited terrain failure, 50 ignored**,
with new HUD/cache and retained regressions passing. The receipt is
`.scratch/upstream-388-workspace-receipt.json`. The normal viewer built in
**4m35s**, version `0.0.0+1529.g388b299c4`, SHA-256
`9e3b71cce97e3f4ad5a1dba77f26617f0d26bc01c2b06c3ae246126a56acdcbf`;
its receipt is `.scratch/upstream-388-binary-receipt.json`. Exact-binary
software-Vulkan results from the owned
`.scratch/upstream-render-smoke/batch-8YOrwLUf` fixture are recorded below.
Physical GPU/native, Windows package and grid acceptance remain pending. The first 17
imports and this selective batch are
not a merge of the entire upstream branch. No new PR, merge to main or release
is claimed.

Preserved baseline `8750450f` passed Linux Clippy in 1m29s and full workspace
tests with **7,900 passed, 1 inherited terrain failure, 50 ignored**. The final
source adds the dependency-unblocked masked-HUD patch, so it needs its own checks.
The baseline's [Windows run 42335589961](https://github.com/DrCum/Foldurora/actions/runs/42335589961)
completed its test step before cancellation: its raw receipt reports
**7,872 passed, 0 failed, 50 ignored**, with all 83 named tests once. Advancing
the same branch superseded that run at 04:17:31 UTC; viewer build was cancelled
and package skipped. This was not a failed-test run and produced no usable
baseline package. The raw cancellation receipt is
`.scratch/windows-ci-42335589961/cancelled-run-receipt.json`; no obsolete artifact
was downloaded. Final [run 42337795785](https://github.com/DrCum/Foldurora/actions/runs/42337795785)
started at 04:17:32 UTC and remains a separate unfinished receipt.

### Bounded renderer/photo/HUD checks — source 388b299c

The exact normal binary above passed owned offline Linux software-Vulkan
landscape framing/export and selected-subject isolation checks. The 960×640
workspace kept the scene in its right half; the matching clean export is
640×360, with sampled panel edges within 1.34 output pixels of their expected
positions. Real Remove/All actions produced 480×320 exports with 4,943 visible
pixels for the selected subject, zero for the empty choice, and all 153,600 for
restored All. The empty image is fully transparent; restored All is opaque RGB.
This establishes the zero-visible-subject CPU behavior; GPU dispatch/draw-call
counts and debug-profile assertions were not measured. Both cases exited normally
with status 0.

HUD visibility passed for synthetic mask cutoffs 128/200/255 at exact HUD zoom
0.5 and 1.0 after the real World → HUDs → Normal size action. Panel dimensions
increased by 2.00–2.03×, allowing for rasterized edges. The independent world
render scale remained 50%.

**HUD shutdown limitation:** the 240-second harness guard sent SIGTERM with the
final Quit confirmation open, before its confirmation command processed. Normal
HUD Quit acceptance is **not established**. Harness/Xvfb wrapper status was 1;
the viewer's raw wait status was not retained, so no numeric viewer exit status is
inferred. All known owned app/driver processes and process groups are absent; no
GPU stall/loss or panic marker was detected. No retry was run. Landscape/isolation
normal exits do not establish HUD shutdown acceptance. The bounded receipt is
`.scratch/upstream-render-smoke/batch-8YOrwLUf/VALIDATION-388-RECEIPT.json`.

Restricted 480-pixel photo controls/help still overlap and clip. Task-local caches,
software Vulkan and the intentional 15 FPS cap do not establish cold-start,
physical GPU, performance or Windows embedding/input/taskbar acceptance. No grid,
ordinary profiles or credentials were used.

The [nine-image gallery and sanitized provenance](https://github.com/DrCum/Foldurora/blob/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/README.md)
contains unchanged, reviewed synthetic PNGs only. Earlier capture history is
preserved; raw logs, private JSON/settings, bootstrap data and user reports remain
private.

| Check | PNG |
| --- | --- |
| Landscape workspace, frame 680 | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/01-landscape-frame-680.png) |
| Matching clean landscape export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/02-landscape-clean-export.png) |
| Selected-subject isolation UI | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/03-selected-isolation-ui.png) |
| Selected transparent export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/04-selected-transparent-export.png) |
| Empty-subject UI | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/05-empty-subjects-ui.png) |
| Empty transparent export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/06-empty-transparent-export.png) |
| Restored-All export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/07-restored-all-export.png) |
| HUD zoom 50% | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/08-hud-zoom-50.png) |
| HUD zoom Normal / 100% | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/c7c7f46b8f57893f736118863a6dd2688dd19906/foldurora/20261107-388b299c/09-hud-zoom-100.png) |

### Deferred FSR and recovery plan — source 388b299c

This is an implementation plan, not risky code included in the current batch.
The reviewed renderer/recovery functions match the preserved `8750450f` source.

- Keep native configuration for swapchain, egui, blur and placement. Define
  unscaled `display_size = output_viewport().size()` and
  `internal_size = fsr::render_size(display_size, ratio)`. Current `world_size`
  already means internal targets; do not scale it twice. Preserve snapshot/RLVa
  upscale bypasses. Composite at the viewport origin even at equal sizes.
- Reconcile `renderer.rs::{rebuild_targets, make_taa, render_view}`, the paired
  `post_size`/`post_depth` resources introduced by FSR, and the Rust/WGSL ABI.
  Preserve `PhotoScene` and its `capture_aspect`, Interface hints/cancellation
  and the RLVa divisor across sizing and export paths.
- Explicitly reset temporal history on device, surface, render/display size,
  AA-quality, viewport, camera-cut, snapshot and standby/resume transitions.
  Current `motion_cut` does not imply `taa.reset`; setting one cannot substitute
  for resetting the other.
- Recovery `f7d31f56` can be integrated independently first. Preserve
  `recover_from_gpu_loss` invalidating host display before `WindowPlan`: the
  WorkerBridge must revoke/purge, release input, restore the old window and stop
  IO before replacement. Add synthetic Active/Preparing/Standby lifecycle tests
  proving restore-before-plan, stale-proof rejection and failed recreation staying
  detached; then accept native captions, taskbar, focus and placement. Preserve
  f7's explicit `!renderer.vr_engaged()` no-recovery guard (absent from the current
  opt-in branch) and the existing repeated-loss Recover→RecoverSafe→Close policy.

## Selected visual evidence

### Photo/export and native icon — f5ae21d5

The [seven-image gallery and sanitized provenance](https://github.com/DrCum/Foldurora/blob/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/README.md)
contains only unchanged synthetic PNG copies. Earlier captures/history remain
intact; no private settings files, raw logs/JSON, user reports or screenshots
are republished.

| Image | Raw PNG |
| --- | --- |
| Restricted landscape, frame 680 | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/01-restricted-landscape-frame-680.png) |
| Matching clean landscape export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/02-restricted-landscape-clean-export.png) |
| Unrestricted controls, portrait, frame 680 | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/03-unrestricted-portrait-frame-680.png) |
| Matching clean portrait export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/04-unrestricted-portrait-clean-export.png) |
| Native Linux 64-pixel window icon | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/05-native-window-icon-64.png) |
| Full Interface workspace, frame 680 | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/06-interface-workspace-frame-680.png) |
| Matching half-scale Interface export | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/34883cadfddfe4d1d557259dda03a43bbc6b443b/foldurora/20261107-f5ae21d5/07-interface-workspace-export.png) |

### Compact 399ffd79

The [compact capture README and provenance](https://github.com/DrCum/Foldurora/blob/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/README.md)
records the fixture and limits. Exactly six final synthetic PNGs were added,
preserving all prior capture history and publishing no private JSON or user files.

| Image | Raw PNG |
| --- | --- |
| EN wide, five fake workers | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/01-en-wide-five-workers.png) |
| EN management drawer | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/02-en-management-open.png) |
| EN narrow profile popup at 150% | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/03-en-narrow-150pct-profile-dropdown.png) |
| EN Close confirmation dialog | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/04-en-close-confirmation.png) |
| JA management drawer | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/05-ja-management-open.png) |
| FR Rename dialog | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/9557bba4a0adb792f1658df062e688a24740efdc/foldurora/20261107-399ffd79/06-fr-profile-rename-dialog.png) |

### Earlier 70c1b5cd and chat evidence

Seven reviewed synthetic PNGs are retained on the dedicated `captures` branch,
preserving its existing images and history. The
[capture README and provenance](https://github.com/DrCum/Foldurora/blob/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/README.md)
records source versions, fixture methods and limits. These raw links are pinned
to that capture commit; they do not depend on the cloud `.scratch` directories.

| Image | Raw PNG |
| --- | --- |
| `70c1b5cd`: Japanese five-slot selector | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/01-ja-selector-open.png) |
| `70c1b5cd`: narrow popup with a 64-character label | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/02-en-narrow-long-label-popup.png) |
| `70c1b5cd`: narrow occupied slot, simulated fixture lock | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/03-en-narrow-simulated-occupied-slot5.png) |
| `70c1b5cd`: real demo slot 1 after preference restart | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/04-slot1-reopened-clock-preference.png) |
| `70c1b5cd`: real sequential demo slot 2 default | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/05-slot2-clock-preference-default.png) |
| `70c1b5cd`: actual helper with a synthetic crash report | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/06-crash-details-synthetic.png) |
| Earlier `5c741b61`: restored Nearby draft, unchanged chat component | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/07-chat-nearby-draft-restored-5c741b61.png) |

## Port order and dependencies

| Feature | Current gap | Next concrete work |
| --- | --- | --- |
| Compact host redesign — first | Implemented at frozen `399ffd79`; Windows package verified with 7,827/0/50 and 36 named passes. Linux regressions, 21 synthetic captures and owned EN CPU-controller smoke passed. | Accept native Windows popup islands, compact controls, management, handoff/input and recovery on the exact new build. |
| Shared chat — second | Source-bound popouts exist; the shared source/conversation selector is not implemented. | Reuse Nearby/resident-IM transport and one canonical `ChatUi` per exact source, with native identities/restrictions. Explicit character selection may update displayed Send as; unrelated world switching must not retarget the sender. |
| Attention — third | Explicit review/mark-read/dismiss behavior is incomplete. | Add a separate typed protocol with native observed-watermark metadata. Attention selection alone must not acknowledge read or activate the world, but may update displayed Send as. |
| Monitors with chat — fourth | Reduced-target caller/scene/UI and aggregate admission are unfinished. | Complete actual small render targets and latest-only delivery under a shared acknowledged/pending budget, alongside source-bound chat. Models alone are not a working monitor. |
| Five stable, renameable character slots | Included in the verified compact `399ffd79` Windows package. Earlier `70c1b5cd` Linux UI rename/restart and sequential slot-1/2 IO passed; those runtime results keep their original source attribution. Earlier host/menu packages used per-launch profiles. | Check native Windows five-slot persistence and labels, exclusive ownership through detach/host death, and fresh session authority on reopen. |
| Per-character/per-conversation drafts | Memory-only navigation and Close/reopen retention is implemented and passed synthetic checks. | Native popout focus/reopen and human grid routing/restrictions remain to be checked. |
| Named workspaces | Viewport and native floater geometry exist; selective named arrangements, previews, favorites and HUD restore are incomplete. | Map Folderstorm's restore groups onto Aurora's typed settings/actions after stable character storage is established. |
| Fade and Birdseye switches | Native activation exists; Folderstorm's visual transitions are not ported. | Preserve fenced GPU/input handoff; design each transition around native camera/render actions after host acceptance. |
| Assistant and other tools | Aurora has native inventory/appearance/camera actions; Folderstorm's adapter and some tools remain. | Port bounded typed actions after the supporting account/workspace models, rather than generic input injection. |
| Profile import/migration | Stable new slots exist; migration of old generated profiles or ordinary viewer preferences is not implemented. | Decide explicit import/copy behavior later; keep credentials, writable caches and live-session authority separate. Moving installations does not silently migrate profiles. |
| Foldurora branding — separate follow-up | Verified `f5ae21d5` Windows package has seven icon frames matching the ICO; actual Linux 64-pixel icon inspection passed. Native Windows appearance remains pending. It is not included in `399ffd79`. | Accept native Windows icon/taskbar appearance; bounded `388b299c` rendering checks do not establish this. |
| Photo/camera viewport — separate follow-up | Verified `f5ae21d5` package includes the source that passed bounded software-Vulkan clean/Interface exports. Restricted 480-pixel controls still overlap/clip; native Windows photo/export acceptance is pending. The older scene expansion remains in `399ffd79`. | Fix/validate narrow control usability and native Windows photo/export behavior. Integrated `388b299c` passed the bounded landscape/isolation checks recorded above. |
| Selective upstream catch-up — separate integration | Pushed `388b299c` adds 16 reviewed upstream commits to `f5ae21d5`, with exact provenance proof and 25 deferred. Linux lint/tests and normal build passed except the inherited terrain precision test; bounded software-Vulkan rendering checks passed with HUD normal Quit unproven after the harness deadline; Windows package proof is pending. | Validate exact-source runtime/package behavior; preserve earlier evidence and the deferred dependency groups. |

Folderstorm can later acknowledge a visible, following, at-tail transcript after
a configured delay (default 60 seconds with visible-read acknowledgment enabled).
That automatic acknowledgment is a separate unported behavior; no current parity
is claimed for it.

After the compact host redesign, the selected next milestone is a shared CPU
chat source/conversation selector using the existing Nearby/resident-IM transport
and one canonical `ChatUi` per exact source. Attention follows as a separate typed
protocol milestone. See the pinned Folderstorm
[host selection behavior](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/tools/session-host/main.cpp#L1508)
and [worker unread handling](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/fssessionworker.cpp#L482).
`ChatLine.event` is export-stream order, not a native unread cursor. Future Mark
Read needs native observed-watermark metadata, never a cursor inferred from
retained host lines.

Stable profile labels are display names, not account identity. The actual logged-in
account must remain visible beside a custom label. Chat drafts must never be keyed
by a slot label or carried into a newly authorized session merely because the slot
is the same. A detached worker retains its profile lock until actual process exit.
The stable namespace remains tied to the canonical executable path; persistence
tests must restart the host from the same location. Another portable folder is a
separate installation for these profiles.

## Human Windows retest

The current verified download is frozen source `f5ae21d5`, including compact host,
photo-viewport changes and folder artwork. Keep the earlier `399ffd79` package
separate. The latest optional local-builder prompt is build-only for final
`388b299c`; delivery/start are unconfirmed, and it is not the verified package
recipe below. New native testing
must cover compact dropdowns/tooltips over the actual world, keyboard/narrow/enlarged-UI
access and Manage open/hide. Earlier
local `8b738ea8` native results do not establish these outcomes on the compact build.

Use the verified [current Windows package](https://github.com/DrCum/Foldurora/actions/runs/42334134158/artifacts/13560192057),
source `f5ae21d5`, version `0.9.3-beta.20261106.1+136.gf5ae21d53`. Extract the
inner `Foldurora-test-f5ae21d535da-windows-x64.zip` to its own folder, then open
PowerShell in the folder containing `aurora-viewer.exe` and run:

```powershell
.\aurora-viewer.exe --session-host --language en --title "Test Foldurora f5ae21d5"
```

Keep this executable in the same folder through every close/reopen test; moving
the portable folder intentionally creates a separate profile group. Use a fresh
PowerShell window without demo/test overrides. Any real-grid login and account
selection are performed by you, with no automated login.

1. With two sessions open, switch A → B → A. Drag the host's actual Windows title
   bar away, release, then drag it back and release. Switch B → A again and resize.
   Check that the intended world, host controls, mouse and keyboard remain usable.
   Briefly check movement/camera input is released when switching. Open the
   viewer's searchable main menu and a submenu: both should stay compact.
2. Detach one viewer. Close the host using its confirmed **Detach sessions and
   close** action. Both viewers should be visible, responsive ordinary windows
   with working captions/taskbar access and correct client contents. Check again
   after a short wait. If a viewer freezes, disappears or reports device loss,
   stop at that checkpoint and retain the report rather than repeatedly retrying.
   Normally Quit the detached workers, confirming their actual process exit.
3. Relaunch the controller with the same command. Check its selector has five
   profiles, rename two and open their viewers manually. In one profile change a
   harmless setting such as **Clock seconds**; keep the other profile distinct.
   Normally Quit the first worker, confirm, and wait for its actual process exit
   before reopening the same slot. Its setting should persist and the other
   profile should keep its own value. Normally close and restart the controller
   from the same executable path; labels and settings should remain. A detached
   viewer must keep its profile **In use** until that viewer actually exits.
4. If you already have Nearby and a resident IM available, type different unsent
   drafts, navigate between them, then Close/reopen that character's chat popout.
   Each draft should return to its own conversation and character; switching the
   active world must not change the sending character. Draft retention is memory
   only and does not extend across a controller restart.

Report the exact displayed version, which steps passed or failed, and a screenshot
of the relevant state. Human acceptance remains pending until these results are
recorded; automated package checks alone do not establish the native behavior.

## Local Windows agent next steps

The latest optional instruction supplied to the user targets final `388b299c`
for build/check/package only; delivery/start remain unconfirmed. It includes no
GUI/native testing. The separate offline native-acceptance recipe below targets
the verified `f5ae21d5` package when that work is requested.

The local builder can use its Computer Use capability and existing desktop
permission for offline acceptance. This cloud session has no callable
send-to-thread/handoff tool; the prepared request remains undelivered. The
following recipe is durable even if its `.scratch` copy is unavailable. If the
desktop or existing permission is unavailable, report that boundary without
touching unrelated windows.

1. Prefer current verified source `f5ae21d535da3a338bace51ffb03af7beeb040c6` from
   [Windows run 42334134158](https://github.com/DrCum/Foldurora/actions/runs/42334134158)
   and its verified artifact linked above. Verify manifest checks, source/version
   suffix and SHA-256 before testing. Preserve the separate `8b738ea8` focused package and
   `8cd9a2aa` backend/profile checkpoint; do not mix their receipts. Keep the
   same executable location throughout persistence tests. If building locally,
   use its own default-dev worktree, normal build-slot rules, `scripts/check.ps1`,
   `cargo build --locked -p aurora-viewer --features xr-static-loader` and
   `scripts/package.ps1`. No cleanup scripts, installation, release or source fixes.
2. Use fresh owned test data and a fresh PowerShell process with `AURORA_DEMO=1`,
   `AURORA_GRAPHICS_PRESET=low`, `AURORA_WINDOW_SIZE=960,640` and
   `AURORA_FPS_LIMIT=15`. Leave login/exit/capture/profile/forced-loss overrides
   and `AURORA_LOG_NAME` unset; do not use `AURORA_UPDATE_TEST_ROOT`. Before
   starting, check only whether
   `%APPDATA%\Aurora\AuroraViewer\config\demo\login_accounts.json` exists.
   If this would change unrelated prior data, stop and report it; do not read,
   rename or delete that data. Use only a fixture whose ownership is established.
   Launch `--session-host --language en` with a source-tagged
   `Test host native f5ae21d5 [AUTOMATION - DO NOT TOUCH]` title and the repository's
   automation border helper. Track the owned executable PIDs/HWNDs and the host's
   actual child workers; worker titles are not inherited. Never select saved
   accounts, enter credentials or submit a grid login.
3. Open two generic demo workers and identify them by owned process/session,
   since visible synthetic names can match. Switch A/B/A. Drag the host's actual
   native title bar with real down/move/up, first a short distance, then away and
   back; release promptly. Capture settled positions, switch again and resize.
   Verify input/cutout alignment, release of held movement/camera input across a
   switch, usable host chrome and compact searchable menus/submenus. Physically
   release all input after the check. Stop on device loss or a persistently
   hidden/unreachable worker; preserve the failing checkpoint rather than retrying.
4. Detach A, then normally close the host with its Detach sessions and close
   action. Both workers must remain visible, responsive ordinary windows with
   native captions/taskbar access and correct client contents immediately and
   after lease fallback. Normally Quit only these owned workers, confirm prompts
   and verify actual PID exit. Forced host loss is a separate case; an
   unconfirmed Close is not exit evidence.
5. Rename two of the five profiles to synthetic labels. Change a harmless real
   preference in slot 1, normally Quit, wait for actual worker exit and reopen
   that slot from the same executable path; its setting must persist with fresh
   session authority. Slot 2 must retain its separate setting/cache namespace.
   Restart the controller and confirm labels persist. A second controller at
   that location must refuse concurrent registry ownership. A detached worker
   must keep its profile occupied through host closure/replacement until the
   worker's actual confirmed exit, then make it available. Inspect only owned
   settings and cache-file metadata, never key bytes or cache contents.
6. Where demo supplies Nearby and an existing resident IM, verify independent
   drafts across navigation and popout Close/reopen, without changing sender when
   switching worlds. Record any missing native-IM fixture as a limit. Check
   crash-details fallback only with a separate synthetic report, without inducing
   a GPU hang. An existing sanitized `d10473ef` `public-report.md` may be read
   for build/GPU/driver/window/loss fields; do not reopen that old real report's
   helper, because automatic sending may be enabled. Keep raw logs/bootstrap
   authentication files private. Return exact source/version, scoped PID mapping,
   screenshots and pass/fail per case. Real-grid routing, restrictions, unread
   behavior, voice and held PTT require the human.

## Validation and input needed later

- The local Windows builder can perform offline native acceptance using its
  Computer Use capability. This cloud session currently has no callable
  send-to-thread/handoff tool, so its prepared request has not been delivered.
  The request is retained as `.scratch/WINDOWS-NATIVE-RETEST-REQUEST.txt` in the
  candidate worktree. Preparation is not delivery or a completed retest. GitHub
  builds remain available; build success does not establish native behavior.
- Only the human performs real-grid login and verifies actual Nearby/IM routing,
  restrictions, unread semantics, voice and held movement/PTT behavior.
- If the standalone graphics loss recurs, retain the report folder and identify
  which viewer/process was active and which switch was requested. The crash label
  detects graphics-device loss; it does not establish the operating-system cause.
  The full sanitized report is now received for `d10473eff`, version
  `0.9.3-beta.20261106.1+107.gd10473eff`, on an RTX 3060/NVIDIA 610.88. It records
  unfinished submission frame 5747 after 1.002 seconds and last finished frame
  5745. Its “hang” label comes from the source's
  [500 ms submission heuristic](https://github.com/DrCum/Foldurora/blob/d10473efff52ceaea2d7b22d111319b0efc4f28e/crates/aurora-render/src/device_lost.rs#L280),
  not independent confirmation of an operating-system reset cause. The coarse
  loss fields are `unknown / other / callback`. Sampled VRAM 2,573/11,347 MB and
  texture pressure 0/7 show no recorded exhaustion. Focused window plus one other
  peer and zero foreground peers is internally consistent. No Characters row
  identity or switch target is recorded, so the observed status mismatch remains
  unresolved. Enumerated Vulkan layers do not prove an overlay was active, and
  no driver culprit is established. The user report itself is not republished.
- No immediate preference blocks new stable slots starting from defaults. Importing
  an old generated profile, copying ordinary viewer preferences, or sharing slots
  across moved installations needs an explicit later choice; no migration is planned
  as an automatic side effect.

Preserve failing checkpoints and exact-source receipts. Do not use a retry,
unconfirmed Close, or a green build alone as acceptance of host recovery.
