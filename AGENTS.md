# Foldurora agent policy

Foldurora ports Folderstorm behavior onto Aurora's Rust viewer. Use
**DrCum/Foldurora** explicitly for GitHub operations. The user's current
instructions override inherited Aurora process requirements and this policy.

## Scope and ownership

- Start from [the current handoff](docs/FOLDURORA_CURRENT_HANDOFF.md). Read
  relevant architecture, task and technical-reference sections on demand;
  repeated whole-document reading and upstream project-board administration
  are not required.
- Keep at most two implementation tasks active. Source-only reviewers are
  useful for concrete risks; further review passes must address findings or
  changed risks. Use the user's preferred models; no Astra agents.
- Work in owned branches/worktrees. Preserve other agents' changes, existing
  build artifacts and unfinished checkpoints. Keep each validation candidate
  frozen, including its full source SHA, while checks/builds run.
- Stay within the current goal and necessary dependencies. While awaiting
  builds or human testing, continue useful work within that goal rather than
  starting unrelated features or upstream integrations.

## Product safeguards

- Consult Folderstorm source for ported behavior. Preserve Aurora protocol
  correctness and existing compatibility; record source references and
  intentional behavior differences.
- Preserve creator permissions, RLVa restrictions, profile/account isolation,
  credential protections and licensing. Process changes never weaken these
  safeguards. Shared permission helpers apply to every UI and alternate path.
- Never log into a grid or handle the user's credentials. Grid login and
  interactive grid acceptance belong to the user. Offline demos and tests
  must not bypass production safeguards.
- Keep code and repository documentation in English. Use existing shared UI
  conventions and localized EN/FR/JA strings. Specialized guidance is in
  [the engineering reference](docs/FOLDURORA_ENGINEERING_REFERENCE.md).

## Cloud and disk

- This cloud environment has a fixed **32 GB** disk. Use it primarily for
  source investigation, implementation, review and focused Linux checks.
- Allow only one heavy cloud build/test process at a time. Reuse the existing
  `ci` profile and owned compatible Cargo graph. Avoid multiple active cloud
  profiles or a dependency build/cache for every source worktree.
- Check free space before expensive work. Reclaim only idle, reproducible
  build caches you own. Preserve source, uncommitted work, profiles, logs,
  screenshots and evidence needed for unresolved failures.
- Keep the known-good build, current candidate and binaries required for open
  reproductions. Verify preservation in existing local/project artifact
  storage before removing older cloud copies. Do not build another archival
  or receipt framework.

## Checks and Windows builds

- During development, test changed behavior with focused tests. Prefer
  workspace-filtered Cargo commands where feature resolution requires them,
  for example `cargo test --profile ci --workspace --locked <filter>`.
- Run the appropriate full suite on the frozen combined candidate. Repeat
  full validation only when subsequent changes, failures or unresolved risks
  justify it. Use normal Linux CI for ownership tests this sandbox cannot
  execute correctly; report the limitation without security exceptions or
  silent skips.
- Windows builds normally use the existing **Foldurora Windows Builder** on
  **Cumputer**, thread `01a1af77-a465-71c3-be08-cd9d1b927f83`, through the user's
  Dot **Slick**. Project build/test communication through that relay is
  authorized. Verify delivery; a prepared request is not a started build.
  If unavailable, provide one complete request for the user to forward.
- Each request pins repository, branch and full SHA; commands/checks,
  features and packaging; specific offline/native tests and expectations;
  and required returns: exact source, commands/outcomes, totals, failure logs,
  executable version, package path and SHA-256.
- Reuse an owned local build worktree and compatible persistent caches. Do
  not routinely clean builds, upgrade tooling, change dependencies or create
  fresh dependency caches. Return build-agent source changes to the lead for
  integration and renewed validation.

## Publication, acceptance and handoff

- Respect explicit publication holds in the current handoff. In particular,
  the denied `670c026b` public-source upload is not approved by later work
  requests; do not retry it, include it in another public payload or change
  branch/tool/route to bypass the restriction.
- Draft PRs may precede human acceptance when pending checks are explicit.
  Do not merge, install or publish a release without user authorization.
  No global PR housekeeping, upstream board administration or Aurora-specific
  naming workflow is required.
- Distinguish implemented, compiled, automatically tested and human-accepted
  behavior. A successful build does not prove Windows/GPU reliability.
- Maintain one concise current handoff with exact checkpoints, validation,
  remaining failures and next actions. Reuse existing scripts and evidence.
  Finish a focused goal with a frozen candidate, verified local package and
  targeted human checklist; if blocked, identify the exact blocker and the
  smallest action needed to remove it.
