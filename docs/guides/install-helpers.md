---
title: Search and install with yay or paru
summary: Point a helper at this server and install the way you would from the AUR.
audience: user
order: 10
---

TAUR speaks the AUR RPC and git layout, so yay and paru can use it as an AUR URL.

## Search on the site

Open **Packages**, type at least two characters, and pick a field (name, description, maintainer, depends, and the other RPC fields). A package page shows the version, dependencies, votes, and the git clone URL.

## Install with a helper

Set the helper's AUR URL to this server, for example `https://YOUR-SERVER`. Then:

```sh
yay -S hello
paru -S hello
```

`yay -Ss hello` and `paru -Ss hello` search the same RPC the site uses. The package page is the URL the helper shows as the AUR page.

The helper clones the git repository, reads `.SRCINFO`, and builds locally. TAUR does not ship built packages.
