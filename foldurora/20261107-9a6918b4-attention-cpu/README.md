# Attention production CPU UI — 7 November 2026 UTC

These six original PNGs are copied byte for byte from the production Attention
body rendered by egui and the software CPU painter with synthetic snapshots.
They have not been edited, resized, combined or recaptured. All profile labels,
account/grid identities, conversation titles, policies and unread observations
are synthetic. No transcript or draft copy is held by the Attention body.

Exact source: [`9a6918b4e7334e4da319449b56e985107e6b5cd5`](https://github.com/DrCum/Foldurora/commit/9a6918b4e7334e4da319449b56e985107e6b5cd5),
pushed on `feat/attention-integration`.
Test ELF SHA-256:
`a5f017ee4d2834c3cbbf6b6e4794007c16ca3c98bdf4025ae70db23dc043cbfc`
(770,055,472 bytes). The test executable is distinct from a normal viewer build.
The fixture is `session_host::attention_ui::visual::attention_cpu_visual_fixture`.

The exact-source Linux run passed all **203 required checks** exactly once and
all **54 CPU captures**: 18 each in English, French and Japanese. The full Linux
workspace result was **8,020 passed, one inherited terrain precision failure,
53 ignored**. Clippy passed with the existing Linux `dead_code` allowance.
The original [root validation receipt](validation-receipt.json) records the
commands and these separate outcomes. The three unchanged full-run manifests
([EN](manifests/en.json), [FR](manifests/fr.json), [JA](manifests/ja.json)) retain
all 54 captures' required-text, clipping and privacy results. This gallery
publishes only six of those images; the other manifest image names are inventory
entries, not additional published PNGs. [SHA256SUMS](SHA256SUMS) and
[manifest.json](manifest.json) identify every copied file and its original name.

The first Attention slice includes five stable saved-profile filters, Nearby
and existing Direct conversations (Direct on, Nearby off by default), a local
Dismiss reminder action and passive Open shared chat/Previous/Next navigation.
Opening chat does not mark unread messages as read or activate a World. Saved
checkmarks follow published preferences; the save-failure scene retains the
previous settings. The simulated load failure hides reminders and disables
Attention actions while the separate host/shared chat remains usable.

Normal scenes use 540×580 logical pixels at 100%; narrow scenes use 380×580 at
150% (570×870 physical pixels); the short scene uses 380×420 at 100%. Expanded
filters scroll within a bounded viewport. The narrow bottom scene follows real
egui wheel events in the CPU fixture. The 380×340 native minimum is covered by
an actual-egui pointer/layout regression, not by these 420-pixel-high images.
A scene can intentionally clip offscreen list/filter rows; full-run checks
require each intended field in the appropriate top/bottom or scrolled scene.

This is CPU UI evidence, not native-window/OS-input, Windows, GPU, real IME,
authentication, preference-file IO, Shared Chat/World submission or grid
acceptance. No real profiles, broker, worker, login or live network are used by
this fixture. Native Review, monitors and chat protocol V3 are outside this MVP.
The separately source-reviewed native Attention fixture and external driver have
not been compiled or run. The Windows candidate run is tracked separately; no
Windows result or normal-viewer binary hash is inferred from these images.

## English selected reminder, filters collapsed — 540×580

![English selected reminder, filters collapsed](en/normal-01-selected.png)

## English expanded profile and category filters — 540×580

![English expanded profile and category filters](en/normal-07-expanded-filters.png)

## French expanded filters, scrolled to the bottom — 380×580 at 150%

![French expanded filters, scrolled to the bottom](fr/narrow-150pct-08-expanded-filters-bottom.png)

## Japanese short expanded-filter window — 380×420

![Japanese short expanded-filter window](ja/short-02-expanded-filters.png)

## English simulated save failure retains published settings — 540×580

![English simulated save failure retains published settings](en/normal-03-save-failed.png)

## Japanese simulated load failure disables reminders — 380×580 at 150%

![Japanese simulated load failure disables reminders](ja/narrow-150pct-06-load-failed.png)
