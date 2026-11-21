# Handoff

Running log for [TAUR](https://github.com/WeirdBrains/taur), the AUR-compatible package server from [Weird Brains](https://weirdbrains.com). Newest entry first.

## 2026-11-20: punk redesign released as v1.13.1

- **Changed:** a punk web redesign: near-black base, hot pink, acid green and hazard yellow, hard-shadow cards, the TAUR logo in the header and a home-page hero with a copy-ready `yay` command. Released as [v1.13.1](https://github.com/WeirdBrains/taur/releases/tag/v1.13.1) and published to TAUR with `taurctl`.
- **Commits:** [de3398b](https://github.com/WeirdBrains/taur/commit/de3398b) (redesign), [8e87b7f](https://github.com/WeirdBrains/taur/commit/8e87b7f) (keep headings in their written case), [47da4d3](https://github.com/WeirdBrains/taur/commit/47da4d3) (release v1.13.1), [119706b](https://github.com/WeirdBrains/taur/commit/119706b) (record the release tarball hash in the public Arch recipe).
- **Checks:** builds and tests passed locally; every CI check passed on v1.13.1 and all packages are attached to the release.
- **Not tested:** the live site, because the package server is still on 1.12.0.
- **Left:** the v1.13.0 tag failed the CI headless-browser gate (the all-caps headings broke it) and has no release or packages; it stays as a failed tag. The production server upgrade to 1.13.1 is left to m3talsmith.
- **Next:** after the upgrade, check that <https://pkg.taur.weirdbrains.com> shows the new look.
