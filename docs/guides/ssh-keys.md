---
title: SSH keys
summary: Add the key git will use to push.
audience: user
order: 30
---

Pushes over SSH are accepted only for a key stored on your account.

```sh
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519
```

On your account page, paste the single line from `~/.ssh/id_ed25519.pub` (`ssh-ed25519 AAAA... comment`). The private key stays on your machine.

Remove a key from that page when you retire it. A removed key can no longer push. The server never stores the private key.

`taurctl key add FILE` and `taurctl key remove ID` do the same thing.
