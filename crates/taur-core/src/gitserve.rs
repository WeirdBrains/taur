//! aurweb-style `git-serve` commands shared by the embedded SSH server and
//! `taur git-serve` (OpenSSH `command=` forced command).
//!
//! Read-only commands (`help`, `list-repos`, `git-upload-pack`) work for any
//! account. Everything else requires a verified email. Push permission matches
//! `pre_receive`: maintainer, co-maintainer, an org member who can manage the
//! package, or a site admin. A new package base can be created by any verified user.

use crate::srcinfo::validate_name;
use crate::{Error, Result, Store};

/// Git transport the caller should exec (`git upload-pack` / `git receive-pack`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOp {
    pub verb: &'static str,
    pub pkgbase: String,
}

/// Result of one SSH command. When `git` is set, stdout/stderr are empty and the
/// caller runs git; otherwise print the buffers and exit with `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub git: Option<GitOp>,
}

impl Outcome {
    fn text(stdout: impl Into<String>, stderr: impl Into<String>, code: i32) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
            code,
            git: None,
        }
    }

    fn ok(stdout: impl Into<String>) -> Self {
        Self::text(stdout, "", 0)
    }

    fn err(stderr: impl Into<String>) -> Self {
        Self::text("", stderr, 1)
    }
}

const HELP: &str = "\
Commands:
    adopt <pkgbase>                       Adopt an orphaned package base.
    disown <pkgbase>                      Disown a package base (it becomes orphaned).
    flag <pkgbase> <comment>              Flag a package base out-of-date.
    help                                  Show this help message and exit.
    list-repos                            List package bases you maintain.
    set-comaintainers <pkgbase> [user…]   Replace the co-maintainer list.
    set-keywords <pkgbase> [keyword…]     Replace package base keywords.
    unflag <pkgbase>                      Remove an out-of-date flag.
    unvote <pkgbase>                      Remove your vote.
    vote <pkgbase>                        Vote for a package base.
    git-receive-pack                      Internal command used with Git.
    git-upload-pack                       Internal command used with Git.
";

/// Split a POSIX-ish command line (`SSH_ORIGINAL_COMMAND`). Supports single and
/// double quotes and backslash escapes. Errors on an unclosed quote.
pub fn split_command(input: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_arg = false;
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match quote {
            Some('\'') if c == '\'' => quote = None,
            Some('\'') => cur.push(c),
            Some('"') if c == '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            Some('"') if c == '"' => quote = None,
            Some(_) => cur.push(c),
            None if c == '\\' => {
                in_arg = true;
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            None if c == '\'' || c == '"' => {
                in_arg = true;
                quote = Some(c);
            }
            None if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut cur));
                    in_arg = false;
                }
            }
            None => {
                in_arg = true;
                cur.push(c);
            }
        }
    }
    if quote.is_some() {
        return Err(Error::Invalid("unclosed quote".into()));
    }
    if in_arg {
        args.push(cur);
    }
    Ok(args)
}

fn die(action: &str, msg: &str) -> Outcome {
    Outcome::err(format!(
        "{action}: {msg}\nTry `help` for a list of commands.\n"
    ))
}

fn fail(action: &str, err: Error) -> Outcome {
    let msg = match err {
        Error::Forbidden => "permission denied".to_string(),
        Error::Unverified => {
            "your account email is not verified. Verify it before using this command".into()
        }
        Error::NotFound(m) | Error::Conflict(m) | Error::Invalid(m) => m,
        other => other.to_string(),
    };
    Outcome::err(format!("{action}: {msg}\n"))
}

/// `exact` is the required argument count (the package base plus nothing else).
fn pkgbase_name(args: &[String]) -> std::result::Result<String, &'static str> {
    if args.is_empty() {
        return Err("missing repository name");
    }
    if args.len() > 1 {
        return Err("too many arguments");
    }
    if validate_name(&args[0]).is_err() {
        return Err("invalid repository name");
    }
    Ok(args[0].clone())
}

/// Run one git-serve command for `user` (the account that owns the offered key).
/// `command` is `SSH_ORIGINAL_COMMAND` (empty when the client opened a shell).
pub fn serve(store: &Store, user: &str, command: &str) -> Outcome {
    let command = command.trim();
    if command.is_empty() {
        return Outcome::err(format!(
            "Welcome to TAUR, {user}! Interactive shell is disabled.\nTry `help` for a list of commands.\n"
        ));
    }
    let argv = match split_command(command) {
        Ok(v) => v,
        Err(e) => return fail("git-serve", e),
    };
    if argv.is_empty() {
        return die("git-serve", "invalid command");
    }
    let mut action = argv[0].clone();
    let mut rest = argv[1..].to_vec();
    if action == "git" {
        let Some(sub) = rest.first() else {
            return die("git", "missing argument");
        };
        if sub != "upload-pack" && sub != "receive-pack" {
            return die("git", "invalid command");
        }
        action = format!("git-{sub}");
        rest.remove(0);
    }
    dispatch(store, user, &action, &rest)
}

fn dispatch(store: &Store, user: &str, action: &str, args: &[String]) -> Outcome {
    let account = match store.user_by_name(user) {
        Ok(Some(u)) => u,
        Ok(None) => return fail(action, Error::NotFound(format!("user {user}"))),
        Err(e) => return fail(action, e),
    };
    let readonly = matches!(action, "help" | "list-repos" | "git-upload-pack");
    if !readonly && let Err(e) = store.require_verified(&account) {
        return fail(action, e);
    }
    match action {
        "help" => {
            if !args.is_empty() {
                return die(action, "too many arguments");
            }
            Outcome::text("", HELP, 0)
        }
        "list-repos" => {
            if !args.is_empty() {
                return die(action, "too many arguments");
            }
            match maintained(store, account.id) {
                Ok(names) => Outcome::ok(
                    names
                        .into_iter()
                        .map(|n| format!(" {n}\n"))
                        .collect::<String>(),
                ),
                Err(e) => fail(action, e),
            }
        }
        "git-upload-pack" | "git-receive-pack" => match git_op(store, &account, action, args) {
            Ok(op) => Outcome {
                stdout: String::new(),
                stderr: String::new(),
                code: 0,
                git: Some(op),
            },
            Err(e) => fail(action, e),
        },
        "adopt" => one_pkg(action, args, |name| {
            let id = require_pkgbase_id(store, name)?;
            store.adopt(account.id, id)?;
            Ok(format!("adopted {name}\n"))
        }),
        "disown" => one_pkg(action, args, |name| {
            let id = require_pkgbase_id(store, name)?;
            store.disown(account.id, id)?;
            Ok(format!("disowned {name}\n"))
        }),
        "vote" => one_pkg(action, args, |name| {
            if store.has_voted(account.id, name)? {
                return Err(Error::Conflict(format!(
                    "you have already voted for {name}"
                )));
            }
            store.vote(account.id, name)?;
            Ok(format!("voted for {name}\n"))
        }),
        "unvote" => one_pkg(action, args, |name| {
            if !store.has_voted(account.id, name)? {
                return Err(Error::Conflict(format!("you have not voted for {name}")));
            }
            store.unvote(account.id, name)?;
            Ok(format!("removed vote from {name}\n"))
        }),
        "flag" => {
            if args.len() < 2 {
                return die(
                    action,
                    if args.is_empty() {
                        "missing repository name"
                    } else {
                        "missing comment"
                    },
                );
            }
            if args.len() > 2 {
                return die(action, "too many arguments");
            }
            if let Err(e) = validate_name(&args[0]) {
                return fail(action, e);
            }
            if args[1].chars().count() < 3 {
                return fail(
                    action,
                    Error::Invalid("comment must be at least 3 characters".into()),
                );
            }
            match store.flag_out_of_date(account.id, &args[0], &args[1]) {
                Ok(()) => Outcome::ok(format!("flagged {}\n", args[0])),
                Err(e) => fail(action, e),
            }
        }
        "unflag" => one_pkg(action, args, |name| {
            store.unflag(account.id, name)?;
            Ok(format!("unflagged {name}\n"))
        }),
        "set-keywords" => keywords_or_coms(store, &account, action, args, true),
        "set-comaintainers" => keywords_or_coms(store, &account, action, args, false),
        _ => die(action, "invalid command"),
    }
}

fn one_pkg(action: &str, args: &[String], f: impl FnOnce(&str) -> Result<String>) -> Outcome {
    let name = match pkgbase_name(args) {
        Ok(n) => n,
        Err("invalid repository name") => {
            return fail(
                action,
                Error::Invalid(format!("invalid package name {:?}", args[0])),
            );
        }
        Err(msg) => return die(action, msg),
    };
    match f(&name) {
        Ok(msg) => Outcome::ok(msg),
        Err(e) => fail(action, e),
    }
}

fn keywords_or_coms(
    store: &Store,
    account: &crate::accounts::User,
    action: &str,
    args: &[String],
    keywords: bool,
) -> Outcome {
    if args.is_empty() {
        return die(action, "missing repository name");
    }
    let name = &args[0];
    if let Err(e) = validate_name(name) {
        return fail(action, e);
    }
    let id = match store.pkgbase_id(name) {
        Ok(Some(id)) => id,
        Ok(None) => return fail(action, Error::NotFound(format!("package base {name}"))),
        Err(e) => return fail(action, e),
    };
    match store.can_manage_pkgbase(account.id, id) {
        Ok(true) => {}
        Ok(false) => return fail(action, Error::Forbidden),
        Err(e) => return fail(action, e),
    }
    let values: Vec<String> = args[1..]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();
    let result = if keywords {
        store.set_keywords(name, &values)
    } else {
        store.set_comaintainers(account.id, name, &values)
    };
    match result {
        Ok(()) => Outcome::ok(format!("updated {name}\n")),
        Err(e) => fail(action, e),
    }
}

fn git_op(
    store: &Store,
    account: &crate::accounts::User,
    action: &str,
    args: &[String],
) -> Result<GitOp> {
    if args.len() != 1 {
        return Err(Error::Invalid(if args.is_empty() {
            "missing path".into()
        } else {
            "too many arguments".into()
        }));
    }
    let path = args[0].trim().trim_end_matches('/');
    let path = path.strip_prefix('/').unwrap_or(path);
    let name = path.strip_suffix(".git").unwrap_or(path);
    validate_name(name)?;
    let verb = if action == "git-upload-pack" {
        "upload-pack"
    } else {
        if let Some(id) = store.pkgbase_id(name)?
            && !store.can_manage_pkgbase(account.id, id)?
        {
            return Err(Error::Invalid(format!(
                "permission denied: {} cannot push to {name}",
                account.name
            )));
        }
        "receive-pack"
    };
    Ok(GitOp {
        verb,
        pkgbase: name.to_string(),
    })
}

fn require_pkgbase_id(store: &Store, name: &str) -> Result<i64> {
    store
        .pkgbase_id(name)?
        .ok_or_else(|| Error::NotFound(format!("package base {name}")))
}

fn maintained(store: &Store, user: i64) -> Result<Vec<String>> {
    let conn = store.lock();
    let mut st =
        conn.prepare("SELECT name FROM pkgbases WHERE maintainer_id = ?1 ORDER BY name")?;
    let rows = st.query_map([user], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quotes() {
        assert_eq!(
            split_command("git-upload-pack '/foo.git'").unwrap(),
            vec!["git-upload-pack", "/foo.git"]
        );
        assert_eq!(
            split_command("flag bar 'too old'").unwrap(),
            vec!["flag", "bar", "too old"]
        );
        assert!(split_command("flag 'nope").is_err());
    }
}
