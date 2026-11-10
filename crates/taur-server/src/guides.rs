//! Task guides shown in the web UI. The Markdown in `docs/guides/` is the source
//! and is compiled into the binary.

use taur_core::accounts::User;

pub struct Guide {
    pub slug: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub audience: &'static str,
    pub order: u32,
    pub body: &'static str,
}

const RAW: &[(&str, &str)] = &[
    ("publish", include_str!("../../../docs/guides/publish.md")),
    (
        "install-helpers",
        include_str!("../../../docs/guides/install-helpers.md"),
    ),
    ("account", include_str!("../../../docs/guides/account.md")),
    ("ssh-keys", include_str!("../../../docs/guides/ssh-keys.md")),
    (
        "organizations",
        include_str!("../../../docs/guides/organizations.md"),
    ),
    (
        "community",
        include_str!("../../../docs/guides/community.md"),
    ),
    ("requests", include_str!("../../../docs/guides/requests.md")),
    (
        "cli-and-api",
        include_str!("../../../docs/guides/cli-and-api.md"),
    ),
    (
        "moderation",
        include_str!("../../../docs/guides/moderation.md"),
    ),
    (
        "administration",
        include_str!("../../../docs/guides/administration.md"),
    ),
    (
        "operating",
        include_str!("../../../docs/guides/operating.md"),
    ),
];

fn parse(slug: &'static str, raw: &'static str) -> Guide {
    let rest = raw
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("guide {slug} is missing front matter"));
    let (fm, body) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("guide {slug} front matter does not end"));
    let mut title = "";
    let mut summary = "";
    let mut audience = "";
    let mut order = 1000u32;
    for line in fm.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "title" => title = value,
            "summary" => summary = value,
            "audience" => audience = value,
            "order" => {
                order = value
                    .parse()
                    .unwrap_or_else(|_| panic!("guide {slug} has a bad order"))
            }
            _ => {}
        }
    }
    if !matches!(audience, "user" | "moderator" | "admin" | "owner") {
        panic!("guide {slug} has audience {audience:?}");
    }
    Guide {
        slug,
        title,
        summary,
        audience,
        order,
        body: body.trim_start(),
    }
}

fn guides() -> Vec<Guide> {
    let mut all: Vec<Guide> = RAW.iter().map(|(slug, raw)| parse(slug, raw)).collect();
    all.sort_by_key(|g| g.order);
    all
}

/// 0 user, 1 moderator, 2 site admin, 3 site owner.
pub fn rank(user: Option<&User>) -> u8 {
    match user {
        Some(u) if u.is_owner => 3,
        Some(u) if u.is_admin => 2,
        Some(u) if u.is_moderator => 1,
        _ => 0,
    }
}

fn allowed(audience: &str, rank: u8) -> bool {
    let need = match audience {
        "user" => 0,
        "moderator" => 1,
        "admin" => 2,
        "owner" => 3,
        _ => 4,
    };
    rank >= need
}

pub fn list(rank: u8) -> Vec<Guide> {
    guides()
        .into_iter()
        .filter(|g| allowed(g.audience, rank))
        .collect()
}

pub fn get(slug: &str, rank: u8) -> Option<Guide> {
    guides()
        .into_iter()
        .find(|g| g.slug == slug && allowed(g.audience, rank))
}
