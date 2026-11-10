---
title: Site admin guide
summary: Verify people, send reset links, grant moderators, and review the audit.
audience: admin
order: 90
---

Site admin is not an organization Admin, and it is not site owner. Owners grant this role.

## People

**Admin** searches accounts. Email addresses are visible to you and hidden from moderators.

On a user page you can mark the address verified, resend the verification mail, email a password-reset link, grant or remove moderator, and sign every session out. You cannot grant or remove site admin or site owner. You cannot remove your own admin flag if you are the owner.

## Audit

**Audit** lists rejections, acceptances, and reversals.

- **Accept** lets that same rejected upload or registration be tried again. It does not create the account or store the recipe.
- **Reverse** clears a package or user suspension.
- **Revoke** withdraws an acceptance. The original rule applies again.

A rejected package request stays closed.

## Mirrors

**Mirrors** copies selected recipes from an AUR-style git source. Add the base URL, select package bases, then sync. Nothing is fetched until sync. Drop a copied package before removing the source. A mirror is not an operator signature and not a pacman repository.

Registries, operator keys, and identity bindings are site-owner actions.
