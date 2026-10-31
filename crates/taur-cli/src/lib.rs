//! `taurctl`: a thin command-line client over the TAUR HTTP API.

pub mod client;
pub mod config;

use clap::{Args, Parser, Subcommand};
use client::Client;
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum CliError {
    NotLoggedIn,
    Api(u16, String),
    Transport(String),
    Config(String),
    Input(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::NotLoggedIn => write!(f, "not logged in (run `taurctl login`)"),
            CliError::Api(code, m) => write!(f, "server said ({code}): {m}"),
            CliError::Transport(m) => write!(f, "cannot reach server: {m}"),
            CliError::Config(m) => write!(f, "config error: {m}"),
            CliError::Input(m) => write!(f, "{m}"),
        }
    }
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::NotLoggedIn | CliError::Api(401, _) => 3,
            _ => 1,
        }
    }
}

/// Manage a TAUR server: accounts, packages, organizations and roles.
#[derive(Parser, Debug)]
#[command(name = "taurctl", version)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Args, Debug)]
pub struct Global {
    /// Server URL (overrides TAUR_SERVER and the config file).
    #[arg(long, global = true, env = "TAUR_SERVER")]
    pub server: Option<String>,
    /// Config directory (default: $TAURCTL_CONFIG_DIR, else $XDG_CONFIG_HOME/taurctl).
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,
    /// Print raw JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Don't ask for confirmation.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    /// Read the password from the first line of stdin instead of prompting.
    #[arg(long, global = true)]
    pub password_stdin: bool,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Show or change CLI settings.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Create an account.
    Register {
        name: String,
        #[arg(long)]
        email: String,
    },
    /// Sign in and store a session token (0600) in the config dir.
    Login { name: String },
    /// End the session and delete the stored token.
    Logout,
    /// Show the signed-in account.
    Whoami,
    /// Delete your account (packages you maintain become orphans).
    Unregister,
    /// Confirm your email address with the token from the verification mail.
    Verify { token: String },
    /// Send the verification mail again.
    ResendVerification,
    /// Reset a forgotten password.
    #[command(subcommand)]
    ResetPassword(ResetCmd),
    /// Search packages (AUR RPC).
    Search {
        term: String,
        /// name, name-desc, maintainer, depends, makedepends, optdepends, checkdepends,
        /// submitter, provides, conflicts, replaces, keywords, groups, comaintainers.
        #[arg(long, default_value = "name-desc")]
        by: String,
    },
    /// Show package details (AUR RPC).
    Info {
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Manage organizations and member roles.
    #[command(subcommand)]
    Org(OrgCmd),
    /// Manage package ownership.
    #[command(subcommand)]
    Pkg(PkgCmd),
    /// Manage SSH public keys used for git pushes over SSH.
    #[command(subcommand)]
    Key(KeyCmd),
}

#[derive(Subcommand, Debug)]
pub enum ResetCmd {
    /// Email a reset link to the account's address.
    Request { email: String },
    /// Set a new password with the token from the reset mail (prompts for the password).
    Confirm {
        #[arg(long)]
        token: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum KeyCmd {
    /// List your SSH keys.
    List,
    /// Add a public key from a file (e.g. ~/.ssh/id_ed25519.pub) or `-` for stdin.
    Add { file: PathBuf },
    /// Remove a key by id.
    Remove { id: i64 },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Print the effective settings.
    Show,
    /// Save the default server URL.
    SetServer { url: String },
}

#[derive(Subcommand, Debug)]
pub enum OrgCmd {
    /// Organizations you belong to.
    List,
    /// Create an organization (you become Owner).
    Create { name: String },
    /// Show members and roles.
    Show { name: String },
    /// Delete an organization (Owner only).
    Delete { name: String },
    /// Add a member (Owner/Admin).
    AddMember {
        org: String,
        user: String,
        #[arg(long, default_value = "contributor")]
        role: String,
    },
    /// Change a member's role (owner, admin, contributor).
    SetRole {
        org: String,
        user: String,
        role: String,
    },
    /// Remove a member.
    RemoveMember { org: String, user: String },
}

#[derive(Subcommand, Debug)]
pub enum PkgCmd {
    /// Package bases you can manage.
    List,
    /// Adopt an orphaned package base.
    Adopt { pkgbase: String },
    /// Give up maintainership (package becomes orphaned).
    Disown { pkgbase: String },
    /// Transfer a package base to an organization or a user.
    Transfer {
        pkgbase: String,
        #[arg(long, conflicts_with = "user", required_unless_present = "user")]
        org: Option<String>,
        #[arg(long)]
        user: Option<String>,
    },
}

pub fn run(cli: Cli) -> i32 {
    match execute(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("taurctl: {e}");
            e.exit_code()
        }
    }
}

fn password(g: &Global, prompt: &str) -> Result<String, CliError> {
    let p = if g.password_stdin {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|e| CliError::Input(e.to_string()))?;
        line.trim_end_matches(['\n', '\r']).to_string()
    } else {
        rpassword::prompt_password(prompt).map_err(|e| CliError::Input(e.to_string()))?
    };
    if p.is_empty() {
        return Err(CliError::Input("empty password".into()));
    }
    Ok(p)
}

fn confirm(g: &Global, question: &str) -> Result<(), CliError> {
    if g.yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || g.password_stdin {
        return Err(CliError::Input(format!("{question}: pass -y to confirm")));
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| CliError::Input(e.to_string()))?;
    if matches!(line.trim(), "y" | "Y" | "yes") {
        Ok(())
    } else {
        Err(CliError::Input("aborted".into()))
    }
}

fn server(g: &Global, dir: &Path) -> Result<String, CliError> {
    Ok(match &g.server {
        Some(s) => s.clone(),
        None => config::load(dir)?
            .server
            .unwrap_or_else(|| config::DEFAULT_SERVER.into()),
    })
}

fn print(g: &Global, v: &Value, human: impl FnOnce(&Value)) {
    if g.json {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    } else {
        human(v);
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(s).collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    }
}

fn done(g: &Global, msg: String) {
    if g.json {
        println!("{}", serde_json::json!({ "ok": true, "message": msg }));
    } else {
        println!("{msg}");
    }
}

fn execute(cli: Cli) -> Result<(), CliError> {
    let g = &cli.global;
    let dir = config::resolve_dir(g.config_dir.clone());
    if let Cmd::Config(c) = &cli.cmd {
        match c {
            ConfigCmd::Show => {
                let v = serde_json::json!({
                    "server": server(g, &dir)?,
                    "config_dir": dir.display().to_string(),
                    "logged_in": config::read_token(&dir).is_some(),
                });
                print(g, &v, |v| {
                    for k in ["server", "config_dir", "logged_in"] {
                        println!("{k}: {}", s(&v[k]));
                    }
                });
            }
            ConfigCmd::SetServer { url } => {
                let mut cfg = config::load(&dir)?;
                cfg.server = Some(url.clone());
                config::save(&dir, &cfg)?;
                done(g, format!("server set to {url}"));
            }
        }
        return Ok(());
    }
    let c = Client::new(&server(g, &dir)?, config::read_token(&dir));
    match &cli.cmd {
        Cmd::Config(_) => unreachable!(),
        Cmd::Register { name, email } => {
            let p = password(g, "New password: ")?;
            if !g.password_stdin
                && rpassword::prompt_password("Repeat password: ")
                    .ok()
                    .as_deref()
                    != Some(p.as_str())
            {
                return Err(CliError::Input("passwords do not match".into()));
            }
            let v = c.register(name, email, &p)?;
            print(g, &v, |v| println!("registered {}", s(&v["name"])));
        }
        Cmd::Login { name } => {
            let p = password(g, "Password: ")?;
            let token = c.login(name, &p)?;
            config::write_token(&dir, &token)?;
            done(g, format!("logged in as {name}"));
        }
        Cmd::Logout => {
            let r = c.logout();
            config::clear_token(&dir);
            match r {
                Ok(()) | Err(CliError::Api(401, _)) => done(g, "logged out".into()),
                Err(e) => return Err(e),
            }
        }
        Cmd::Whoami => {
            let v = c.whoami()?;
            print(g, &v, |v| {
                let state = if v["verified"] == true {
                    "verified"
                } else {
                    "unverified"
                };
                println!("{} <{}> ({state})", s(&v["name"]), s(&v["email"]))
            });
        }
        Cmd::Unregister => {
            let me = c.whoami()?;
            let p = password(g, "Password: ")?;
            confirm(
                g,
                &format!("Delete account {} permanently?", s(&me["name"])),
            )?;
            c.unregister(&p)?;
            config::clear_token(&dir);
            done(g, format!("account {} deleted", s(&me["name"])));
        }
        Cmd::Verify { token } => {
            let v = c.verify(token)?;
            print(g, &v, |v| println!("verified {}", s(&v["name"])));
        }
        Cmd::ResendVerification => {
            c.resend_verification()?;
            done(g, "verification mail sent".into());
        }
        Cmd::ResetPassword(ResetCmd::Request { email }) => {
            c.reset_request(email)?;
            done(
                g,
                format!("if {email} has an account, a reset link is on its way"),
            );
        }
        Cmd::ResetPassword(ResetCmd::Confirm { token }) => {
            let p = password(g, "New password: ")?;
            if !g.password_stdin
                && rpassword::prompt_password("Repeat password: ")
                    .ok()
                    .as_deref()
                    != Some(p.as_str())
            {
                return Err(CliError::Input("passwords do not match".into()));
            }
            c.reset_confirm(token, &p)?;
            config::clear_token(&dir);
            done(g, "password changed; all sessions were signed out".into());
        }
        Cmd::Search { term, by } => {
            let v = c.search(term, by)?;
            print(g, &v, |v| {
                for p in v["results"].as_array().into_iter().flatten() {
                    println!(
                        "{} {}  [{}]\n    {}",
                        s(&p["Name"]),
                        s(&p["Version"]),
                        s(&p["Maintainer"]),
                        s(&p["Description"])
                    );
                }
            });
        }
        Cmd::Info { packages } => {
            let v = c.info(packages)?;
            print(g, &v, |v| {
                for p in v["results"].as_array().into_iter().flatten() {
                    for k in [
                        "Name",
                        "PackageBase",
                        "Version",
                        "Description",
                        "URL",
                        "License",
                        "Maintainer",
                        "CoMaintainers",
                        "Depends",
                        "MakeDepends",
                        "OptDepends",
                        "Provides",
                        "Conflicts",
                        "Keywords",
                        "NumVotes",
                        "OutOfDate",
                        "URLPath",
                    ] {
                        println!("{k:<14}: {}", s(&p[k]));
                    }
                    println!();
                }
            });
        }
        Cmd::Org(o) => match o {
            OrgCmd::List => {
                let v = c.orgs()?;
                print(g, &v, |v| {
                    for o in v.as_array().into_iter().flatten() {
                        println!("{}  {}", s(&o["name"]), s(&o["role"]));
                    }
                });
            }
            OrgCmd::Create { name } => {
                let v = c.org_create(name)?;
                print(g, &v, |v| {
                    println!("created organization {}", s(&v["name"]))
                });
            }
            OrgCmd::Show { name } => {
                let v = c.org_show(name)?;
                print(g, &v, |v| {
                    println!("{}", s(&v["name"]));
                    for m in v["members"].as_array().into_iter().flatten() {
                        println!("  {}  {}", s(&m["name"]), s(&m["role"]));
                    }
                });
            }
            OrgCmd::Delete { name } => {
                confirm(g, &format!("Delete organization {name}?"))?;
                c.org_delete(name)?;
                done(g, format!("deleted organization {name}"));
            }
            OrgCmd::AddMember { org, user, role } | OrgCmd::SetRole { org, user, role } => {
                let role = role.to_ascii_lowercase();
                c.set_member(org, user, &role)?;
                done(g, format!("{user} is {role} of {org}"));
            }
            OrgCmd::RemoveMember { org, user } => {
                c.remove_member(org, user)?;
                done(g, format!("removed {user} from {org}"));
            }
        },
        Cmd::Key(k) => match k {
            KeyCmd::List => {
                let v = c.keys()?;
                print(g, &v, |v| {
                    for k in v.as_array().into_iter().flatten() {
                        println!(
                            "{}  {}  {}  {}",
                            s(&k["id"]),
                            s(&k["fingerprint"]),
                            s(&k["algorithm"]),
                            s(&k["comment"])
                        );
                    }
                });
            }
            KeyCmd::Add { file } => {
                let text = if file.as_os_str() == "-" {
                    let mut t = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut t)
                        .map_err(|e| CliError::Input(e.to_string()))?;
                    t
                } else {
                    std::fs::read_to_string(file)
                        .map_err(|e| CliError::Input(format!("{}: {e}", file.display())))?
                };
                let v = c.key_add(text.trim())?;
                print(g, &v, |v| {
                    println!("added key {} ({})", s(&v["id"]), s(&v["fingerprint"]))
                });
            }
            KeyCmd::Remove { id } => {
                c.key_remove(*id)?;
                done(g, format!("removed key {id}"));
            }
        },
        Cmd::Pkg(p) => match p {
            PkgCmd::List => {
                let v = c.my_packages()?;
                print(g, &v, |v| {
                    for p in v.as_array().into_iter().flatten() {
                        println!(
                            "{}  maintainer={} org={}",
                            s(&p["pkgbase"]),
                            s(&p["maintainer"]),
                            s(&p["org"])
                        );
                    }
                });
            }
            PkgCmd::Adopt { pkgbase } => {
                c.pkg_action(pkgbase, "adopt", None)?;
                done(g, format!("adopted {pkgbase}"));
            }
            PkgCmd::Disown { pkgbase } => {
                confirm(g, &format!("Disown {pkgbase}?"))?;
                c.pkg_action(pkgbase, "disown", None)?;
                done(g, format!("disowned {pkgbase}"));
            }
            PkgCmd::Transfer { pkgbase, org, user } => {
                let (body, to) = match (org, user) {
                    (Some(o), _) => (serde_json::json!({"org": o}), format!("organization {o}")),
                    (None, Some(u)) => (serde_json::json!({"user": u}), format!("user {u}")),
                    _ => return Err(CliError::Input("give --org or --user".into())),
                };
                c.pkg_action(pkgbase, "owner", Some(body))?;
                done(g, format!("transferred {pkgbase} to {to}"));
            }
        },
    }
    Ok(())
}

/// Render man pages for `taurctl` and every subcommand: `(file name, roff)`.
pub fn man_pages() -> Vec<(String, Vec<u8>)> {
    fn walk(cmd: clap::Command, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        let name = if prefix.is_empty() {
            cmd.get_name().to_string()
        } else {
            format!("{prefix}-{}", cmd.get_name())
        };
        let cmd = cmd
            .name(&*name.clone().leak())
            .disable_help_subcommand(true);
        let mut buf = Vec::new();
        let _ = clap_mangen::Man::new(cmd.clone()).render(&mut buf);
        out.push((format!("{name}.1"), buf));
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            walk(sub.clone(), &name, out);
        }
    }
    let mut out = Vec::new();
    let mut cmd = <Cli as clap::CommandFactory>::command();
    cmd.build();
    walk(cmd, "", &mut out);
    out
}
