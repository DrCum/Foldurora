# Workspace Inventory folder restoration slice

On top of a6119664, optional main-Inventory folder references are captured in a
workspace's existing account/grid-private catalogue. Capture defaults off and
stores a loaded folder UUID only. No asset, contents, label or filesystem path is
stored. Disabling capture also removes the last-arrangement folder reference;
existing named records keep their original choices.

Capture and final Switch share the native folder chooser's actual main-owner,
RLVa showinv, Inventory owner, current archive VisibilityRead and checked
ordinary-parent-path rules. The helper never creates an Include exemption.
Unknown/stale/hidden owners, library/trash paths and missing folders refuse
navigation. Current graph/viewport/source and native layout gates remain required.
No fetch, item move, wear action or new Inventory window is generated.

Preview leaves folder navigation unchanged. Switch, startup restore and Previous
apply the folder only after successful geometry placement under a fresh native
permission read. Previous only includes folders if its switch affected that
group. Received Items uses the existing native tab/reveal route. Unobserved
folders do not falsely mark a record modified. Last arrangement retains an earlier
authorized same-account folder reference while its owner is closed/unobserved.

Intentional current limits: main Inventory only, selected folder or selected
item's loaded parent, no expanded-tree/extra-window/library restoration. Inventory
must be visible in an eligible ordinary view when capture/navigation happens.
A closed/restricted/unavailable folder is skipped with a localized status; the
viewer does not open it or queue a hidden future navigation. This differs from
Folderstorm's broader panel restoration and remains a documented limitation.

Three focused actual-source model/controller tests pass in the component harness;
real viewer all-target compilation/lint and13 localization cases are checked
separately. Native visibility/callback behavior and full Windows tests remain
pending. Reuse the current related-group checkpoint cadence: do not request a
separate full package for every small folder/context change. Source authority,
archive masks and private-store safeguards are unchanged.
