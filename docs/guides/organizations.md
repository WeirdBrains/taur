---
title: Organizations
summary: Share a package base with other maintainers.
audience: user
order: 40
---

An organization is a group of accounts. It is not the site-owner role.

## Create one

Open **Organizations** and create a name. You become its Owner.

## Roles inside the organization

- **Owner** manages members, can delete the organization, and cannot be removed by an Admin. The last Owner has to transfer ownership or delete the organization first.
- **Admin** adds and removes members, except Owners.
- **Contributor** can manage the organization's packages.

## Move a package

On Upload, the owner list is your account (the default) and every organization where you can manage packages (Owner, Admin, or Contributor). Type in the filter at the top of the list to narrow it. `taurctl pkg upload ARCHIVE --org ORG` does the same from the CLI, using the organization's exact name; omit `--org` to publish under your account. A new package is created under the owner you pick. An update does not move it.

On a package you maintain, transfer it to the organization or to another user. Members can then push and edit co-maintainers according to their role. Disowning leaves the package orphaned so someone else can adopt it.
