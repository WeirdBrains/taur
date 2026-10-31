# Organizations and roles

Library-level model (API/UI in phase 4). Users create organizations and become Owner.

| Action | Owner | Admin | Contributor |
|---|---|---|---|
| Manage packages | ✓ | ✓ | ✓ |
| Manage members | ✓ | ✓ | |
| Manage org settings | ✓ | | |
| Delete org | ✓ | | |

Admins cannot grant the Owner role or change/remove Owners. The last Owner cannot be
demoted or removed. A package base can be assigned to an organization by someone who can
manage it and has ManagePackages there; then all members can manage it, alongside the
maintainer and co-maintainers.
