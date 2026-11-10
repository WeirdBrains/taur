# TAUR project site

Static pages for `https://taur.weirdbrains.com`. This branch is only the project site.
The package repository, RPC, git, and API are `https://packages.taur.weirdbrains.com`,
served by the `taur` binary from the `main` branch.

## Preview

From this directory:

```sh
python -m http.server 8090 --bind 127.0.0.1
```

Open `http://127.0.0.1:8090/`. Canonical links in the HTML use `https://taur.weirdbrains.com`.
Serving the folder locally does not make that name resolve.

## Deploy

`CNAME` is `taur.weirdbrains.com`, which is the name GitHub Pages would publish if
this branch is the Pages source. `Caddyfile` is an alternate vhost for the same
files. Neither file was applied to a live server from this checkout. The package
service hostname is not this site.

## Mark

`assets/logo.svg` is an original angular tau. It is not the Arch Linux logo and it is
not an Arch Linux endorsement. `assets/logo-dark.svg` is the same geometry for a dark
background. `assets/icon.svg` is the favicon tile. `assets/favicon.ico` and the root `/favicon.ico` are the same mark in ICO form (16, 32, and 48 px), so browsers that request `/favicon.ico` get it. `assets/wordmark.svg` sets the word
"taur" beside the mark. No Arch artwork was traced.
