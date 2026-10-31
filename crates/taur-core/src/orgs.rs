//! Organizations, membership roles and package ownership checks.

use crate::{Action, Error, Result, Role, Store, accounts::validate_username, now};
use rusqlite::{OptionalExtension, params};

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
