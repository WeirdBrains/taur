//! Organization roles and what each may do.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Contributor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Create, update, push to and adopt/disown packages owned by the org.
    ManagePackages,
    /// Add, remove and change roles of members.
    ManageMembers,
    /// Rename or reconfigure the organization.
    ManageOrg,
    /// Delete the organization.
    DeleteOrg,
}

impl Role {
    pub fn allows(self, action: Action) -> bool {
        match self {
            Role::Owner => true,
            Role::Admin => matches!(action, Action::ManagePackages | Action::ManageMembers),
            Role::Contributor => matches!(action, Action::ManagePackages),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Contributor => "contributor",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "contributor" => Some(Role::Contributor),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_matrix() {
        use Action::*;
        for a in [ManagePackages, ManageMembers, ManageOrg, DeleteOrg] {
            assert!(Role::Owner.allows(a));
        }
        assert!(Role::Admin.allows(ManagePackages) && Role::Admin.allows(ManageMembers));
        assert!(!Role::Admin.allows(ManageOrg) && !Role::Admin.allows(DeleteOrg));
        assert!(Role::Contributor.allows(ManagePackages));
        assert!(!Role::Contributor.allows(ManageMembers));
        assert_eq!(Role::parse("Admin"), Some(Role::Admin));
        assert_eq!(Role::parse("x"), None);
    }
}
