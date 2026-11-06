//! Organizations, membership roles and package ownership checks.

use crate::{Action, Error, Result, Role, Store, accounts::validate_username, now};
use rusqlite::{OptionalExtension, params};

/// Result of adopt. `Request` means a site admin still has to accept it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    Maintainer,
    Request(i64),
}

impl Store {
    /// Create an organization; `actor` becomes its Owner.
    pub fn create_org(&self, actor: i64, name: &str) -> Result<i64> {
        validate_username(name)?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let taken: Option<i64> = tx
            .query_row("SELECT id FROM orgs WHERE name = ?1", [name], |r| r.get(0))
            .optional()?;
        if taken.is_some() {
            return Err(Error::Conflict(format!("organization {name} exists")));
        }
        tx.execute(
            "INSERT INTO orgs (name, created_at) VALUES (?1, ?2)",
            params![name, now()],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO org_members (org_id, user_id, role) VALUES (?1, ?2, 'owner')",
            params![id, actor],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn role_in(&self, org: i64, user: i64) -> Result<Option<Role>> {
        let role: Option<String> = self
            .lock()
            .query_row(
                "SELECT role FROM org_members WHERE org_id = ?1 AND user_id = ?2",
                params![org, user],
                |r| r.get(0),
            )
            .optional()?;
        Ok(role.and_then(|r| Role::parse(&r)))
    }

    fn require(&self, org: i64, actor: i64, action: Action) -> Result<Role> {
        match self.role_in(org, actor)? {
            Some(r) if r.allows(action) => Ok(r),
            _ => Err(Error::Forbidden),
        }
    }

    fn owner_count(&self, org: i64) -> Result<i64> {
        Ok(self.lock().query_row(
            "SELECT COUNT(*) FROM org_members WHERE org_id = ?1 AND role = 'owner'",
            [org],
            |r| r.get(0),
        )?)
    }

    /// Add a member or change their role. Admins cannot grant Owner or touch Owners;
    /// the last Owner cannot be demoted.
    pub fn set_member(&self, actor: i64, org: i64, user: i64, role: Role) -> Result<()> {
        let actor_role = self.require(org, actor, Action::ManageMembers)?;
        let current = self.role_in(org, user)?;
        if actor_role != Role::Owner && (role == Role::Owner || current == Some(Role::Owner)) {
            return Err(Error::Forbidden);
        }
        if current == Some(Role::Owner) && role != Role::Owner && self.owner_count(org)? <= 1 {
            return Err(Error::Conflict(
                "an organization needs at least one owner".into(),
            ));
        }
        self.lock().execute(
            "INSERT INTO org_members (org_id, user_id, role) VALUES (?1, ?2, ?3) \
             ON CONFLICT (org_id, user_id) DO UPDATE SET role = excluded.role",
            params![org, user, role.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_member(&self, actor: i64, org: i64, user: i64) -> Result<()> {
        let actor_role = self.require(org, actor, Action::ManageMembers)?;
        let current = self.role_in(org, user)?;
        if current == Some(Role::Owner) {
            if actor_role != Role::Owner {
                return Err(Error::Forbidden);
            }
            if self.owner_count(org)? <= 1 {
                return Err(Error::Conflict(
                    "an organization needs at least one owner".into(),
                ));
            }
        }
        self.lock().execute(
            "DELETE FROM org_members WHERE org_id = ?1 AND user_id = ?2",
            params![org, user],
        )?;
        Ok(())
    }

    /// Transfer a package base to an organization. The actor must be able to manage the
    /// package now and hold ManagePackages in the target org.
    pub fn set_pkgbase_org(&self, actor: i64, pkgbase: i64, org: Option<i64>) -> Result<()> {
        if !self.can_manage_pkgbase(actor, pkgbase)? {
            return Err(Error::Forbidden);
        }
        if let Some(o) = org {
            self.require(o, actor, Action::ManagePackages)?;
        }
        self.lock().execute(
            "UPDATE pkgbases SET org_id = ?1 WHERE id = ?2",
            params![org, pkgbase],
        )?;
        Ok(())
    }

    /// Maintainer, co-maintainer, or a member of the owning org with ManagePackages
    /// (every role has it).
    pub fn can_manage_pkgbase(&self, user: i64, pkgbase: i64) -> Result<bool> {
        let conn = self.lock();
        let row: Option<(Option<i64>, Option<i64>)> = conn
            .query_row(
                "SELECT maintainer_id, org_id FROM pkgbases WHERE id = ?1",
                [pkgbase],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((maintainer, org)) = row else {
            return Err(Error::NotFound("package base".into()));
        };
        if maintainer == Some(user) {
            return Ok(true);
        }
        let admin: bool = conn
            .query_row("SELECT is_admin FROM users WHERE id = ?1", [user], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false);
        if admin {
            return Ok(true);
        }
        let co: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM co_maintainers WHERE pkgbase_id = ?1 AND user_id = ?2",
                params![pkgbase, user],
                |r| r.get(0),
            )
            .optional()?;
        if co.is_some() {
            return Ok(true);
        }
        drop(conn);
        Ok(match org {
            Some(o) => self
                .role_in(o, user)?
                .is_some_and(|r| r.allows(Action::ManagePackages)),
            None => false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OrgMembership {
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Member {
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OrgDetail {
    pub id: i64,
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OwnedPackage {
    pub pkgbase: String,
    pub maintainer: Option<String>,
    pub org: Option<String>,
}

impl Store {
    pub fn org_id(&self, name: &str) -> Result<i64> {
        self.lock()
            .query_row("SELECT id FROM orgs WHERE name = ?1", [name], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("organization {name}")))
    }

    pub fn user_id(&self, name: &str) -> Result<i64> {
        self.lock()
            .query_row("SELECT id FROM users WHERE name = ?1", [name], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("user {name}")))
    }

    /// Organizations `user` belongs to, with their role.
    pub fn orgs_for(&self, user: i64) -> Result<Vec<OrgMembership>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT o.name, m.role FROM orgs o JOIN org_members m ON m.org_id = o.id \
             WHERE m.user_id = ?1 ORDER BY o.name",
        )?;
        let rows = st.query_map([user], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (name, role) = row?;
            if let Some(role) = Role::parse(&role) {
                out.push(OrgMembership { name, role });
            }
        }
        Ok(out)
    }

    /// Organization details; only members may view them.
    pub fn org_detail(&self, actor: i64, name: &str) -> Result<OrgDetail> {
        let id = self.org_id(name)?;
        if self.role_in(id, actor)?.is_none() {
            return Err(Error::Forbidden);
        }
        let conn = self.lock();
        let (org_name, members) = {
            let org_name: String =
                conn.query_row("SELECT name FROM orgs WHERE id = ?1", [id], |r| r.get(0))?;
            let mut st = conn.prepare(
                "SELECT u.name, m.role FROM org_members m JOIN users u ON u.id = m.user_id \
                 WHERE m.org_id = ?1 ORDER BY u.name",
            )?;
            let rows = st.query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut members = Vec::new();
            for row in rows {
                let (name, role) = row?;
                if let Some(role) = Role::parse(&role) {
                    members.push(Member { name, role });
                }
            }
            (org_name, members)
        };
        Ok(OrgDetail {
            id,
            name: org_name,
            members,
        })
    }

    /// Delete an organization (Owner only). Its packages keep their maintainer and
    /// lose the org link.
    pub fn delete_org(&self, actor: i64, org: i64) -> Result<()> {
        self.require(org, actor, Action::DeleteOrg)?;
        self.lock()
            .execute("DELETE FROM orgs WHERE id = ?1", [org])?;
        Ok(())
    }

    /// Package bases `user` maintains, co-maintains, or can manage through an org.
    pub fn packages_for(&self, user: i64) -> Result<Vec<OwnedPackage>> {
        let conn = self.lock();
        let mut st = conn.prepare(
            "SELECT b.name, m.name, o.name FROM pkgbases b \
             LEFT JOIN users m ON m.id = b.maintainer_id LEFT JOIN orgs o ON o.id = b.org_id \
             WHERE b.maintainer_id = ?1 \
             OR b.org_id IN (SELECT org_id FROM org_members WHERE user_id = ?1) \
             OR EXISTS (SELECT 1 FROM co_maintainers c WHERE c.pkgbase_id = b.id AND c.user_id = ?1) \
             ORDER BY b.name",
        )?;
        let rows = st.query_map([user], |r| {
            Ok(OwnedPackage {
                pkgbase: r.get(0)?,
                maintainer: r.get(1)?,
                org: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Give up a relationship with the package base.
    ///
    /// Matches aurweb's website, with TAUR's org link left in place:
    /// the maintainer's disown promotes the lowest `user_id` co-maintainer
    /// (aurweb promotes the lowest `Priority`); a co-maintainer drops only
    /// themselves; a site admin who is neither orphans the base and clears
    /// co-maintainers; an org member who can manage the package clears the
    /// personal maintainer and keeps the org.
    pub fn disown(&self, actor: i64, pkgbase: i64) -> Result<()> {
        let maintainer: Option<i64> = self
            .lock()
            .query_row(
                "SELECT maintainer_id FROM pkgbases WHERE id = ?1",
                [pkgbase],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("package base".into()))?;
        let comaints: Vec<i64> = {
            let conn = self.lock();
            let mut st = conn.prepare(
                "SELECT user_id FROM co_maintainers WHERE pkgbase_id = ?1 ORDER BY user_id",
            )?;
            let rows = st.query_map([pkgbase], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let is_maintainer = maintainer == Some(actor);
        let is_comaintainer = comaints.contains(&actor);
        let is_admin = self.is_admin(actor)?;
        if is_maintainer {
            let next = comaints.into_iter().find(|id| *id != actor);
            self.lock().execute(
                "UPDATE pkgbases SET maintainer_id = ?1 WHERE id = ?2",
                params![next, pkgbase],
            )?;
            if let Some(id) = next {
                self.lock().execute(
                    "DELETE FROM co_maintainers WHERE pkgbase_id = ?1 AND user_id = ?2",
                    params![pkgbase, id],
                )?;
            }
            return Ok(());
        }
        if is_comaintainer && !is_admin {
            self.lock().execute(
                "DELETE FROM co_maintainers WHERE pkgbase_id = ?1 AND user_id = ?2",
                params![pkgbase, actor],
            )?;
            return Ok(());
        }
        if is_admin {
            let conn = self.lock();
            conn.execute(
                "UPDATE pkgbases SET maintainer_id = NULL WHERE id = ?1",
                [pkgbase],
            )?;
            conn.execute(
                "DELETE FROM co_maintainers WHERE pkgbase_id = ?1",
                [pkgbase],
            )?;
            return Ok(());
        }
        if self.can_manage_pkgbase(actor, pkgbase)? {
            self.lock().execute(
                "UPDATE pkgbases SET maintainer_id = NULL WHERE id = ?1",
                [pkgbase],
            )?;
            return Ok(());
        }
        Err(Error::Forbidden)
    }

    /// Website adopt. A site admin takes an orphan immediately (aurweb Package
    /// Maintainer). An org member who can manage an org-owned orphan does too
    /// (TAUR). Anyone else files an `adoption` request and does not become
    /// maintainer until a site admin accepts it.
    pub fn adopt(&self, actor: i64, pkgbase: i64) -> Result<Adopted> {
        self.adopt_inner(actor, pkgbase, false)
    }

    /// SSH `adopt`, matching aurweb `git-serve`: an orphan always files a
    /// request, including when the account is a site admin. An org-owned
    /// orphan is still taken immediately by a member who can manage it.
    pub fn adopt_ssh(&self, actor: i64, pkgbase: i64) -> Result<Adopted> {
        self.adopt_inner(actor, pkgbase, true)
    }

    fn adopt_inner(&self, actor: i64, pkgbase: i64, ssh: bool) -> Result<Adopted> {
        let (maintainer, org): (Option<i64>, Option<i64>) = self
            .lock()
            .query_row(
                "SELECT maintainer_id, org_id FROM pkgbases WHERE id = ?1",
                [pkgbase],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("package base".into()))?;
        if maintainer.is_some() {
            return Err(Error::Conflict("package base is not orphaned".into()));
        }
        let admin = self.is_admin(actor)?;
        if org.is_some() {
            if !self.can_manage_pkgbase(actor, pkgbase)? {
                return Err(Error::Forbidden);
            }
            self.assign_maintainer(actor, pkgbase)?;
            return Ok(Adopted::Maintainer);
        }
        if admin && !ssh {
            self.assign_maintainer(actor, pkgbase)?;
            return Ok(Adopted::Maintainer);
        }
        let pending: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM requests WHERE pkgbase_id = ?1 AND kind = 'adoption' AND status = 'pending'",
            [pkgbase],
            |r| r.get(0),
        )?;
        if pending > 0 {
            return Err(Error::Conflict(
                "there is already a pending adoption request".into(),
            ));
        }
        let name: String =
            self.lock()
                .query_row("SELECT name FROM pkgbases WHERE id = ?1", [pkgbase], |r| {
                    r.get(0)
                })?;
        let req = self.file_request(actor, &name, "adoption", None, "Autogenerated by TAUR.")?;
        Ok(Adopted::Request(req.id))
    }

    fn assign_maintainer(&self, actor: i64, pkgbase: i64) -> Result<()> {
        self.lock().execute(
            "UPDATE pkgbases SET maintainer_id = ?1 WHERE id = ?2",
            params![actor, pkgbase],
        )?;
        Ok(())
    }

    /// Move a package base out of any org to `user` as maintainer.
    pub fn transfer_to_user(&self, actor: i64, pkgbase: i64, user: i64) -> Result<()> {
        if !self.can_manage_pkgbase(actor, pkgbase)? {
            return Err(Error::Forbidden);
        }
        self.lock().execute(
            "UPDATE pkgbases SET org_id = NULL, maintainer_id = ?1 WHERE id = ?2",
            params![user, pkgbase],
        )?;
        Ok(())
    }
}
