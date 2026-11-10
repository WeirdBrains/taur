//! Install a published TAUR native package with the system package manager.
//!
//! This is a local operator command. It does not talk to a TAUR server except
//! for an optional post-restart health check. Package, configuration, database,
//! and upload/git data stay where the native package already keeps them. A
//! package rollback cannot undo a database schema migration.

use crate::CliError;
use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// GitHub repository that publishes release assets.
pub const GITHUB_REPO: &str = "WeirdBrains/taur";
const API_BASE: &str = "https://api.github.com";
const HEALTH_TRIES: u32 = 10;
const HEALTH_WAIT: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Deb,
    Rpm,
    Arch,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Family::Deb => "Debian/Ubuntu (.deb)",
            Family::Rpm => "Fedora/RHEL (.rpm)",
            Family::Arch => "Arch (.pkg.tar.zst)",
        }
    }

    pub fn asset(self, version: &str) -> String {
        match self {
            Family::Deb => format!("taur_{version}-1_amd64.deb"),
            Family::Rpm => format!("taur-{version}-1.x86_64.rpm"),
            Family::Arch => format!("taur-{version}-1-x86_64.pkg.tar.zst"),
        }
    }

    fn installer(self, pkg: &Path) -> Result<(String, Vec<String>), CliError> {
        let pkg = pkg
            .to_str()
            .ok_or_else(|| CliError::Input("package path is not valid UTF-8".into()))?
            .to_string();
        match self {
            Family::Deb => Ok(("apt-get".into(), vec!["install".into(), "-y".into(), pkg])),
            Family::Rpm => Ok(("dnf".into(), vec!["install".into(), "-y".into(), pkg])),
            Family::Arch => Ok((
                "pacman".into(),
                vec!["-U".into(), "--noconfirm".into(), pkg],
            )),
        }
    }

    fn query_installed(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Family::Deb => ("dpkg-query", &["-W", "-f", "${Version}\n", "taur"]),
            Family::Rpm => ("rpm", &["-q", "--qf", "%{VERSION}", "taur"]),
            Family::Arch => ("pacman", &["-Q", "taur"]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Platform {
    pub family: Family,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub version: Option<String>,
    pub restart: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub json: bool,
    pub health_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub tag: String,
    pub package: String,
    pub action: String,
    pub restarted: bool,
    pub rolled_back: bool,
}

impl Outcome {
    fn json(&self) -> Value {
        json!({
            "ok": true,
            "tag": self.tag,
            "package": self.package,
            "action": self.action,
            "restarted": self.restarted,
            "rolled_back": self.rolled_back,
        })
    }
}

/// Side effects used by `run_with`. Tests supply a fake implementation.
pub trait Env {
    fn os(&self) -> &str;
    fn arch(&self) -> &str;
    fn os_release(&self) -> Option<String>;
    fn has_command(&self, name: &str) -> bool;
    fn is_root(&self) -> bool;
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOut, CliError>;
    fn get(&self, url: &str) -> Result<Vec<u8>, CliError>;
    fn log(&self, msg: &str);
    fn confirm(&self, question: &str) -> Result<(), CliError>;
    fn sleep(&self, d: Duration);
    fn work_dir(&self) -> Result<PathBuf, CliError>;
    fn env_file(&self) -> PathBuf {
        PathBuf::from("/etc/taur/taur.env")
    }
    fn api_base(&self) -> String {
        API_BASE.to_string()
    }
}

#[derive(Clone, Debug)]
pub struct CmdOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOut {
    fn ok(&self) -> bool {
        self.code == 0
    }
}

/// Detect the package family for this machine. Errors instead of picking a
/// package for the wrong architecture or an unsupported OS.
pub fn detect(io: &dyn Env) -> Result<Platform, CliError> {
    let os = io.os();
    let arch = io.arch();
    if os != "linux" || arch != "x86_64" {
        return Err(CliError::Input(format!(
            "taurctl upgrade supports Linux x86_64 only (this is {os}/{arch}); published assets are amd64/x86_64 .deb, .rpm, and .pkg.tar.zst"
        )));
    }
    let (id, like) = parse_os_release(io.os_release().as_deref().unwrap_or(""));
    let debian = matches_id(&id, &like, &["debian", "ubuntu"]);
    let rhel = matches_id(
        &id,
        &like,
        &["fedora", "rhel", "centos", "rocky", "almalinux"],
    );
    let arch_os = matches_id(&id, &like, &["arch", "manjaro", "endeavouros"]);
    let family = if debian && (io.has_command("apt-get") || io.has_command("dpkg")) {
        Family::Deb
    } else if rhel && (io.has_command("dnf") || io.has_command("rpm")) {
        Family::Rpm
    } else if arch_os && io.has_command("pacman") {
        Family::Arch
    } else if io.has_command("apt-get") || io.has_command("dpkg") {
        Family::Deb
    } else if io.has_command("dnf") || io.has_command("rpm") {
        Family::Rpm
    } else if io.has_command("pacman") {
        Family::Arch
    } else {
        return Err(CliError::Input(
            "no supported package manager found (need apt-get, dnf, or pacman)".into(),
        ));
    };
    if family == Family::Deb && !io.has_command("apt-get") {
        return Err(CliError::Input(
            "Debian/Ubuntu upgrade needs apt-get (dpkg alone is not used)".into(),
        ));
    }
    if family == Family::Rpm && !io.has_command("dnf") {
        return Err(CliError::Input(
            "Fedora/RHEL upgrade needs dnf (rpm alone is not used)".into(),
        ));
    }
    Ok(Platform { family })
}

pub fn run(opts: Options) -> Result<Outcome, CliError> {
    run_with(opts, &mut Real)
}

pub fn run_with(opts: Options, io: &mut dyn Env) -> Result<Outcome, CliError> {
    let platform = detect(io)?;
    let requested = opts.version.as_deref().map(normalize_tag);
    let release = fetch_release(io, requested.as_deref())?;
    let version = strip_v(&release.tag);
    let asset = platform.family.asset(version);
    let url = release
        .assets
        .iter()
        .find(|a| a.name == asset)
        .map(|a| a.url.clone())
        .ok_or_else(|| {
            CliError::Input(format!(
                "release {} has no {} package ({}); published Linux x86_64 assets were not used for a different architecture",
                release.tag,
                platform.family.name(),
                asset
            ))
        })?;
    let sums_url = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .map(|a| a.url.clone())
        .ok_or_else(|| {
            CliError::Input(format!(
                "release {} has no SHA256SUMS; refusing to install an unverified package",
                release.tag
            ))
        })?;

    let installed = installed_version(io, platform.family);
    io.log(&format!(
        "release {}  package {}  ({})",
        release.tag,
        asset,
        platform.family.name()
    ));
    if let Some(cur) = &installed {
        io.log(&format!("installed version: {cur}"));
        if cur == version && !opts.dry_run {
            let out = Outcome {
                tag: release.tag.clone(),
                package: asset,
                action: "already-installed".into(),
                restarted: false,
                rolled_back: false,
            };
            emit(&opts, io, &out, &format!("TAUR {cur} is already installed"));
            return Ok(out);
        }
    } else {
        io.log("no installed 'taur' package was found; this will be a fresh install");
    }

    let (prog, args) = platform.family.installer(&PathBuf::from(&asset))?;
    let planned = display_cmd(io, &prog, &args);
    if opts.dry_run {
        io.log(&format!("would install with: {planned}"));
        if opts.restart {
            io.log(&format!(
                "would restart with: {} and then GET {}/api/v1/server",
                display_cmd(io, "systemctl", &["restart".into(), "taur".into()]),
                health_base(&opts, io)
            ));
        } else {
            io.log("would not restart the service (pass --restart)");
        }
        io.log("a package rollback cannot undo a database schema migration");
        let out = Outcome {
            tag: release.tag,
            package: asset,
            action: "dry-run".into(),
            restarted: false,
            rolled_back: false,
        };
        emit(
            &opts,
            io,
            &out,
            "dry run; nothing was downloaded or installed",
        );
        return Ok(out);
    }

    let work = io.work_dir()?;
    std::fs::create_dir_all(&work).map_err(|e| CliError::Input(e.to_string()))?;
    let sums = io.get(&sums_url)?;
    let sums_text = String::from_utf8(sums)
        .map_err(|_| CliError::Input("SHA256SUMS is not valid UTF-8".into()))?;

    let mut previous = None;
    if let Some(cur) = &installed
        && cur != version
    {
        match retain_previous(io, &work, platform.family, cur, &release.tag) {
            Ok(p) => previous = Some(p),
            Err(e) => {
                io.log(&format!(
                    "could not keep the installed package for rollback: {e}"
                ));
                io.log("if the new version migrates the database, reinstalling an older package cannot undo that");
                if !opts.yes {
                    io.confirm("continue without a retained rollback package?")?;
                }
            }
        }
    }

    let pkg_path = work.join(&asset);
    let bytes = io.get(&url)?;
    std::fs::write(&pkg_path, &bytes).map_err(|e| CliError::Input(e.to_string()))?;
    verify_sha256(io, &pkg_path, &asset, &sums_text)?;

    let (prog, args) = platform.family.installer(&pkg_path)?;
    let shown = display_cmd(io, &prog, &args);
    if !opts.yes {
        io.confirm(&format!(
            "install TAUR {} with `{shown}`? Configuration, the database, and git repositories are left in place",
            release.tag
        ))?;
    }
    run_privileged(io, &prog, &args)?;
    io.log(&format!("installed {}", asset));

    let mut restarted = false;
    let mut rolled_back = false;
    if opts.restart {
        if !opts.yes {
            io.confirm(
                "restart the taur service now? if the new version already migrated the database, rolling the package back cannot undo that",
            )?;
        }
        match restart_and_check(io, &opts) {
            Ok(()) => restarted = true,
            Err(e) => {
                io.log(&format!("health check failed: {e}"));
                if let Some(prev) = &previous {
                    io.log(&format!(
                        "reinstalling the previous package {}",
                        prev.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    io.log("reinstalling the previous package cannot undo a database schema migration; restore a pre-upgrade database backup if the service still fails");
                    let (p, a) = platform.family.installer(prev)?;
                    run_privileged(io, &p, &a)?;
                    rolled_back = true;
                    if let Err(e2) = restart_and_check(io, &opts) {
                        return Err(CliError::Input(format!(
                            "rolled back the package but the service is still unhealthy: {e2}"
                        )));
                    }
                    restarted = true;
                    let out = Outcome {
                        tag: release.tag,
                        package: asset,
                        action: "rolled-back".into(),
                        restarted,
                        rolled_back,
                    };
                    emit(
                        &opts,
                        io,
                        &out,
                        "the new package failed its health check; the previous package was reinstalled and the service was restarted",
                    );
                    return Ok(out);
                }
                return Err(CliError::Input(format!(
                    "the new package was installed but the service health check failed, and no previous package was retained for rollback: {e}"
                )));
            }
        }
    } else {
        io.log(
            "the service was not restarted (pass --restart to restart it and run a health check)",
        );
    }

    let out = Outcome {
        tag: release.tag,
        package: asset,
        action: "installed".into(),
        restarted,
        rolled_back,
    };
    let msg = if restarted {
        format!("installed {} and restarted the taur service", out.package)
    } else {
        format!("installed {}", out.package)
    };
    emit(&opts, io, &out, &msg);
    Ok(out)
}

fn emit(opts: &Options, io: &dyn Env, out: &Outcome, human: &str) {
    if opts.json {
        io.log(&serde_json::to_string_pretty(&out.json()).unwrap_or_default());
    } else {
        io.log(human);
    }
}

struct Release {
    tag: String,
    assets: Vec<Asset>,
}

struct Asset {
    name: String,
    url: String,
}

fn fetch_release(io: &dyn Env, tag: Option<&str>) -> Result<Release, CliError> {
    let base = io.api_base();
    let url = match tag {
        Some(t) => format!("{base}/repos/{GITHUB_REPO}/releases/tags/{t}"),
        None => format!("{base}/repos/{GITHUB_REPO}/releases/latest"),
    };
    let body = io.get(&url).map_err(|e| match e {
        CliError::Transport(m) if m.contains("404") => {
            CliError::Input(format!("no GitHub release for {}", tag.unwrap_or("latest")))
        }
        other => other,
    })?;
    parse_release(&body)
}

fn parse_release(body: &[u8]) -> Result<Release, CliError> {
    let v: Value = serde_json::from_slice(body)
        .map_err(|e| CliError::Input(format!("invalid GitHub release JSON: {e}")))?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        return Err(CliError::Input(format!("GitHub API: {msg}")));
    }
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| CliError::Input("GitHub release has no tag_name".into()))?
        .to_string();
    let assets = v
        .get("assets")
        .and_then(|a| a.as_array())
        .ok_or_else(|| CliError::Input("GitHub release has no assets".into()))?
        .iter()
        .filter_map(|a| {
            Some(Asset {
                name: a.get("name")?.as_str()?.to_string(),
                url: a.get("browser_download_url")?.as_str()?.to_string(),
            })
        })
        .collect();
    Ok(Release { tag, assets })
}

fn retain_previous(
    io: &dyn Env,
    work: &Path,
    family: Family,
    current: &str,
    _new_tag: &str,
) -> Result<PathBuf, CliError> {
    let tag = normalize_tag(current);
    let release = fetch_release(io, Some(&tag))?;
    let asset = family.asset(strip_v(&tag));
    let url = release
        .assets
        .iter()
        .find(|a| a.name == asset)
        .map(|a| a.url.clone())
        .ok_or_else(|| {
            CliError::Input(format!("no {asset} on release {tag} to keep for rollback"))
        })?;
    let sums_url = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .map(|a| a.url.clone())
        .ok_or_else(|| CliError::Input(format!("release {tag} has no SHA256SUMS")))?;
    let sums = String::from_utf8(io.get(&sums_url)?)
        .map_err(|_| CliError::Input("SHA256SUMS is not valid UTF-8".into()))?;
    let path = work.join(&asset);
    std::fs::write(&path, io.get(&url)?).map_err(|e| CliError::Input(e.to_string()))?;
    verify_sha256(io, &path, &asset, &sums)?;
    Ok(path)
}

fn verify_sha256(io: &dyn Env, file: &Path, name: &str, sums: &str) -> Result<(), CliError> {
    if !io.has_command("sha256sum") {
        return Err(CliError::Input(
            "sha256sum is required to verify SHA256SUMS".into(),
        ));
    }
    let expected = expected_hash(sums, name)
        .ok_or_else(|| CliError::Input(format!("{name} is not listed in SHA256SUMS")))?;
    let path = file
        .to_str()
        .ok_or_else(|| CliError::Input("package path is not valid UTF-8".into()))?;
    let out = io.run("sha256sum", &[path])?;
    if !out.ok() {
        return Err(CliError::Input(format!(
            "sha256sum failed: {}{}",
            out.stdout, out.stderr
        )));
    }
    let actual = out
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if actual != expected.to_ascii_lowercase() {
        return Err(CliError::Input(format!(
            "checksum mismatch for {name}: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn expected_hash<'a>(sums: &'a str, name: &str) -> Option<&'a str> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        if file == name || Path::new(file).file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(hash);
        }
    }
    None
}

fn installed_version(io: &dyn Env, family: Family) -> Option<String> {
    let (prog, args) = family.query_installed();
    if !io.has_command(prog) {
        return None;
    }
    let out = io.run(prog, args).ok()?;
    if !out.ok() {
        return None;
    }
    parse_installed(family, &out.stdout)
}

pub fn parse_installed(family: Family, stdout: &str) -> Option<String> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    let raw = match family {
        Family::Arch => text.split_whitespace().nth(1).unwrap_or(text),
        Family::Deb | Family::Rpm => text,
    };
    let ver = raw.split('-').next().unwrap_or(raw).trim();
    if ver.is_empty() {
        None
    } else {
        Some(ver.to_string())
    }
}

fn restart_and_check(io: &dyn Env, opts: &Options) -> Result<(), CliError> {
    if !io.has_command("systemctl") {
        return Err(CliError::Input(
            "systemctl not found; start the service by hand and GET /api/v1/server".into(),
        ));
    }
    run_privileged(io, "systemctl", &["restart".into(), "taur".into()])?;
    let base = health_base(opts, io);
    let url = format!("{}/api/v1/server", base.trim_end_matches('/'));
    let mut last = "no response".to_string();
    for i in 0..HEALTH_TRIES {
        if i > 0 {
            io.sleep(HEALTH_WAIT);
        }
        match io.get(&url) {
            Ok(body) => {
                if let Ok(v) = serde_json::from_slice::<Value>(&body)
                    && v.get("version").and_then(|x| x.as_str()).is_some()
                {
                    return Ok(());
                }
                last = "response was not a TAUR /api/v1/server document".into();
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(CliError::Input(format!(
        "GET {url} did not return a healthy TAUR server: {last}"
    )))
}

fn health_base(opts: &Options, io: &dyn Env) -> String {
    if let Some(u) = &opts.health_url {
        return u.trim_end_matches('/').to_string();
    }
    if let Some(u) = bind_to_health_url(&io.env_file()) {
        return u;
    }
    "http://127.0.0.1:8080".into()
}

pub fn bind_to_health_url(env_file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(env_file).ok()?;
    let mut bind = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("TAUR_BIND=") {
            bind = Some(v.trim().trim_matches('"').to_string());
        }
    }
    let bind = bind?;
    let port = bind.rsplit(':').next()?.to_string();
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("http://127.0.0.1:{port}"))
}

fn run_privileged(io: &dyn Env, program: &str, args: &[String]) -> Result<(), CliError> {
    if !io.has_command(program) {
        return Err(CliError::Input(format!("{program} not found")));
    }
    let (prog, argv) = if io.is_root() {
        (
            program.to_string(),
            args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )
    } else if io.has_command("sudo") {
        let mut v = vec![program];
        v.extend(args.iter().map(|s| s.as_str()));
        ("sudo".into(), v)
    } else {
        return Err(CliError::Input(
            "installing a package requires root or sudo".into(),
        ));
    };
    let refs: Vec<&str> = argv.to_vec();
    let out = io.run(&prog, &refs)?;
    if !out.ok() {
        return Err(CliError::Input(format!(
            "{prog} {} failed (exit {}): {}{}",
            refs.join(" "),
            out.code,
            out.stdout,
            out.stderr
        )));
    }
    Ok(())
}

fn display_cmd(io: &dyn Env, program: &str, args: &[String]) -> String {
    let mut parts = Vec::new();
    if !io.is_root() {
        parts.push("sudo".into());
    }
    parts.push(program.to_string());
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

pub fn normalize_tag(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('v') || s.starts_with('V') {
        format!("v{}", &s[1..])
    } else {
        format!("v{s}")
    }
}

fn strip_v(tag: &str) -> &str {
    tag.strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag)
}

fn parse_os_release(text: &str) -> (String, String) {
    let mut id = String::new();
    let mut like = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"').to_ascii_lowercase();
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            like = v.trim_matches('"').to_ascii_lowercase();
        }
    }
    (id, like)
}

fn matches_id(id: &str, like: &str, names: &[&str]) -> bool {
    names
        .iter()
        .any(|n| id == *n || like.split_whitespace().any(|w| w == *n))
}

struct Real;

impl Env for Real {
    fn os(&self) -> &str {
        std::env::consts::OS
    }
    fn arch(&self) -> &str {
        std::env::consts::ARCH
    }
    fn os_release(&self) -> Option<String> {
        std::fs::read_to_string("/etc/os-release").ok()
    }
    fn has_command(&self, name: &str) -> bool {
        let Some(path) = std::env::var_os("PATH") else {
            return false;
        };
        for dir in std::env::split_paths(&path) {
            let p = dir.join(name);
            if p.is_file() {
                return true;
            }
        }
        false
    }
    fn is_root(&self) -> bool {
        match Command::new("id").arg("-u").output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim() == "0",
            _ => false,
        }
    }
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOut, CliError> {
        let o = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| CliError::Input(format!("cannot run {program}: {e}")))?;
        Ok(CmdOut {
            code: o.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        })
    }
    fn get(&self, url: &str) -> Result<Vec<u8>, CliError> {
        let resp = ureq::AgentBuilder::new()
            .user_agent(&format!("taurctl/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .get(url)
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, _) => {
                    CliError::Transport(format!("GET {url} returned {code}"))
                }
                other => CliError::Transport(other.to_string()),
            })?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| CliError::Transport(e.to_string()))?;
        Ok(buf)
    }
    fn log(&self, msg: &str) {
        println!("{msg}");
        let _ = std::io::stdout().flush();
    }
    fn confirm(&self, question: &str) -> Result<(), CliError> {
        if !std::io::stdin().is_terminal() {
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
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
    fn work_dir(&self) -> Result<PathBuf, CliError> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let dir = base
            .join("taurctl")
            .join(format!("upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| CliError::Input(e.to_string()))?;
        Ok(dir)
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn tags_and_asset_names() {
        assert_eq!(normalize_tag("1.5.0"), "v1.5.0");
        assert_eq!(normalize_tag("v1.5.0"), "v1.5.0");
        assert_eq!(Family::Deb.asset("1.5.0"), "taur_1.5.0-1_amd64.deb");
        assert_eq!(Family::Rpm.asset("1.5.0"), "taur-1.5.0-1.x86_64.rpm");
        assert_eq!(
            Family::Arch.asset("1.5.0"),
            "taur-1.5.0-1-x86_64.pkg.tar.zst"
        );
    }

    #[test]
    fn parses_installed_versions() {
        assert_eq!(
            parse_installed(Family::Deb, "1.5.0-1\n").as_deref(),
            Some("1.5.0")
        );
        assert_eq!(
            parse_installed(Family::Rpm, "1.4.0").as_deref(),
            Some("1.4.0")
        );
        assert_eq!(
            parse_installed(Family::Arch, "taur 1.5.0-1").as_deref(),
            Some("1.5.0")
        );
    }

    #[test]
    fn sums_lookup() {
        let sums = "abc  taur_1.5.0-1_amd64.deb\ndef  other\n";
        assert_eq!(expected_hash(sums, "taur_1.5.0-1_amd64.deb"), Some("abc"));
        assert_eq!(expected_hash(sums, "missing"), None);
    }
}
