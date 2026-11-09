use clap::Parser;
use std::io::Read;
use std::sync::Arc;
use taur_core::{Store, resolve_home};
use taur_server::cli::{Cli, Cmd};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Cmd::Hook { name } = &cli.cmd {
        std::process::exit(hook(name));
    }
    if let Cmd::GitServe { user } = &cli.cmd {
        let store = Store::open(&resolve_home(cli.home.clone())).unwrap_or_else(|e| {
            eprintln!("taur: {e}");
            std::process::exit(1);
        });
        std::process::exit(taur_server::ssh::exec_for_sshd(&store, user));
    }
    if let Err(e) = run(cli).await {
        eprintln!("taur: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open(&resolve_home(cli.home))?;
    match cli.cmd {
        Cmd::Serve(args) => {
            let cfg = args.validate()?;
            store.configure_mail(cfg.smtp_url.as_deref(), &cfg.mail_from, &cfg.base_url)?;
            store.set_ssh_clone_base(cfg.ssh_clone_base.clone());
            if cfg.smtp_url.is_some() {
                eprintln!("taur: sending mail via SMTP");
            } else {
                eprintln!(
                    "taur: no SMTP configured; writing mail to {}",
                    store.home().join("outbox").display()
                );
            }
            for w in taur_server::cli::bootstrap_admins(&store, &cfg.admins) {
                eprintln!("taur: warning: {w}");
            }
            store.configure_rate_limits(&cfg.rate);
            log_limits(&cfg.rate);
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown_tx.send(true);
            });
            let store = Arc::new(store);
            let s2 = store.clone();
            tokio::spawn(async move {
                // Popularity decays daily; refresh it hourly.
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    tick.tick().await;
                    if let Err(e) = s2.refresh_popularity() {
                        eprintln!("taur: popularity refresh failed: {e}");
                    }
                }
            });
            let sgc = store.clone();
            tokio::spawn(async move {
                // The first interval tick is immediate; skip it so startup does
                // not maintain repositories while the process is still opening.
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
                tick.tick().await;
                loop {
                    tick.tick().await;
                    let s = sgc.clone();
                    let _ =
                        tokio::task::spawn_blocking(move || match taur_core::repo::maintain_repos(
                            &s.repos_dir(),
                            std::time::Duration::from_secs(14 * 24 * 60 * 60),
                        ) {
                            Ok(r) => eprintln!(
                                "taur: repository maintenance collected {} and skipped {}",
                                r.collected, r.skipped
                            ),
                            Err(e) => eprintln!("taur: repository maintenance failed: {e}"),
                        })
                        .await;
                }
            });
            if let Some(addr) = cfg.ssh_bind {
                let hook_bin = std::env::current_exe()?;
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                eprintln!("taur: ssh on {}", listener.local_addr()?);
                tokio::spawn(taur_server::ssh::serve(listener, store.clone(), hook_bin));
            }
            if cfg.tls == taur_server::cli::TlsMode::Acme {
                let acme = cfg.acme.clone().expect("acme config");
                let cache = acme
                    .cache
                    .clone()
                    .unwrap_or_else(|| store.home().join("acme"));
                if acme.staging {
                    eprintln!(
                        "taur: ACME staging directory {}; certificates from it are not publicly trusted",
                        acme.directory
                    );
                } else {
                    eprintln!("taur: ACME directory {}", acme.directory);
                }
                let tls = taur_server::acme::bootstrap(
                    taur_server::acme::AcmeSettings {
                        domains: cfg.domains.clone(),
                        directory: acme.directory.clone(),
                        staging: acme.staging,
                        contact: acme.contact.clone(),
                        accept_terms: true,
                        cache,
                        http_bind: acme.http_bind,
                        renew_before: acme.renew_before,
                    },
                    taur_server::acme::InstantAcme,
                    shutdown_rx.clone(),
                )
                .await?;
                if let Some(addr) = cfg.grpc_bind {
                    let listener = tokio::net::TcpListener::bind(&addr).await?;
                    eprintln!(
                        "taur: grpc TLS on {} (same certificate as HTTPS)",
                        listener.local_addr()?
                    );
                    let s3 = store.clone();
                    let grpc_cfg = tls.grpc.clone();
                    tokio::spawn(async move {
                        if let Err(e) = taur_server::grpc::serve_tls(listener, s3, grpc_cfg).await {
                            eprintln!("taur: grpc server stopped: {e}");
                        }
                    });
                }
                let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
                eprintln!(
                    "taur: HTTPS on {} for {}",
                    listener.local_addr()?,
                    cfg.domains.join(", ")
                );
                eprintln!("taur: public URL {}", cfg.base_url);
                taur_server::acme::serve_https(
                    listener,
                    taur_server::app(store),
                    tls.https,
                    shutdown_rx,
                )
                .await?;
            } else {
                if let Some(addr) = cfg.grpc_bind {
                    let listener = tokio::net::TcpListener::bind(&addr).await?;
                    eprintln!(
                        "taur: grpc plaintext HTTP/2 on {} (not TLS)",
                        listener.local_addr()?
                    );
                    let s3 = store.clone();
                    tokio::spawn(async move {
                        if let Err(e) = taur_server::grpc::serve(listener, s3).await {
                            eprintln!("taur: grpc server stopped: {e}");
                        }
                    });
                }
                let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
                eprintln!("taur: plaintext HTTP on http://{}", listener.local_addr()?);
                eprintln!("taur: public URL {}", cfg.base_url);
                let mut shutdown_rx = shutdown_rx;
                axum::serve(
                    listener,
                    taur_server::app(store)
                        .into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await?;
            }
        }
        Cmd::Gc { grace_secs } => {
            let report = taur_core::repo::maintain_repos(
                &store.repos_dir(),
                std::time::Duration::from_secs(grace_secs),
            )?;
            println!(
                "collected {} repositories, skipped {} busy",
                report.collected, report.skipped
            );
        }
        Cmd::Import {
            dir,
            maintainer,
            registry,
        } => {
            let info = store.import_into(&dir, maintainer.as_deref(), registry.as_deref())?;
            println!("imported {} {}", info.pkgbase, info.version());
        }
        Cmd::Registry(cmd) => registry_cmd(&store, cmd)?,
        Cmd::Mirror(cmd) => mirror_cmd(&store, cmd)?,
        Cmd::Moderator { name, revoke } => {
            let id = store.user_id(&name)?;
            store.grant_moderator(id, !revoke)?;
            println!(
                "{name} is {}a moderator",
                if revoke { "no longer " } else { "" }
            );
        }
        Cmd::Admin { name, revoke } => {
            let id = store.user_id(&name)?;
            store.set_admin(id, !revoke)?;
            println!(
                "{name} is {}a site admin",
                if revoke { "no longer " } else { "" }
            );
        }
        Cmd::VerifyUser { name } => {
            let user = store
                .user_by_name(&name)?
                .ok_or_else(|| taur_core::Error::NotFound(format!("user {name}")))?;
            store.mark_verified(user.id)?;
            println!("verified {name}");
        }
        Cmd::Hook { .. } | Cmd::GitServe { .. } => unreachable!(),
        Cmd::SshAuth { key_type, key } => {
            match taur_server::ssh::authorized_keys_line(&store, &key_type, &key) {
                Ok(Some(line)) => print!("{line}"),
                Ok(None) => {}
                Err(e) => {
                    eprintln!("taur: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

fn log_limits(policy: &taur_core::RatePolicy) {
    let parts: Vec<String> = policy
        .classes
        .iter()
        .map(|c| {
            if c.max == 0 {
                format!("{} off", c.class.name())
            } else {
                format!("{} {}/{}s", c.class.name(), c.max, c.window.as_secs())
            }
        })
        .collect();
    eprintln!("taur: rate limits: {}", parts.join(", "));
    if policy.trusted_proxies.is_empty() {
        eprintln!("taur: X-Forwarded-For is ignored; clients are keyed by the TCP peer");
    } else {
        let nets: Vec<String> = policy
            .trusted_proxies
            .iter()
            .map(|n| n.to_string())
            .collect();
        eprintln!(
            "taur: trusted proxies for X-Forwarded-For: {}",
            nets.join(", ")
        );
        eprintln!(
            "taur: the proxy must append the client address. Without that header, every client behind the proxy shares one bucket"
        );
    }
}

fn mirror_cmd(
    store: &taur_core::Store,
    cmd: taur_server::cli::MirrorCmd,
) -> Result<(), Box<dyn std::error::Error>> {
    use taur_server::cli::MirrorCmd;
    match cmd {
        MirrorCmd::Add { name, url } => {
            store.add_mirror(&name, &url)?;
            println!(
                "mirror {name} reads recipes from {url}/{{pkgbase}}.git (provenance mirror; not a pacman repository and not an operator attestation). Nothing has been fetched."
            );
        }
        MirrorCmd::List => {
            for row in store.list_mirrors()? {
                println!(
                    "{} url={} registry={} provenance={}",
                    row.name, row.base_url, row.registry, row.provenance
                );
            }
        }
        MirrorCmd::Select { name, pkgbases } => {
            if pkgbases.is_empty() {
                return Err("name at least one package base".into());
            }
            for pkg in &pkgbases {
                store.select_mirror_package(&name, pkg)?;
                println!("selected {pkg} on {name}");
            }
        }
        MirrorCmd::Unselect { name, pkgbase } => {
            store.unselect_mirror_package(&name, &pkgbase)?;
            println!("{pkgbase} will not be updated; the last copy is kept");
        }
        MirrorCmd::Sync { name } => {
            let rows = store.sync_mirror(&name)?;
            if rows.is_empty() {
                println!("mirror {name} has no selected packages");
            }
            let mut failed = false;
            for row in rows {
                if row.status == "unselected" {
                    continue;
                }
                println!(
                    "{} {} revision={} stale={} {}",
                    row.pkgbase,
                    row.status,
                    row.revision.unwrap_or_else(|| "-".into()),
                    row.stale,
                    row.detail
                );
                if matches!(
                    row.status.as_str(),
                    "failed" | "blocked" | "upstream-missing"
                ) {
                    failed = true;
                }
            }
            if failed {
                return Err("one or more packages were not updated".into());
            }
        }
        MirrorCmd::Status { name } => {
            for row in store.mirror_status(&name)? {
                println!(
                    "{} status={} revision={} stale={} {}",
                    row.pkgbase,
                    row.status,
                    row.revision.unwrap_or_else(|| "-".into()),
                    row.stale,
                    row.detail
                );
            }
        }
        MirrorCmd::Remove { name } => {
            store.remove_mirror(&name)?;
            println!("removed mirror {name}");
        }
        MirrorCmd::Drop { name, pkgbase } => {
            store.drop_mirrored_package(&name, &pkgbase)?;
            println!("dropped {pkgbase} from mirror {name}");
        }
    }
    Ok(())
}

fn registry_cmd(
    store: &taur_core::Store,
    cmd: taur_server::cli::RegistryCmd,
) -> Result<(), Box<dyn std::error::Error>> {
    use taur_core::registry::{self, PROVENANCE_OPERATOR};
    use taur_server::cli::RegistryCmd;
    match cmd {
        RegistryCmd::Create { name } => {
            store.insert_registry(&name, "enforce")?;
            println!(
                "created enforcing package registry {name} (provenance {PROVENANCE_OPERATOR})"
            );
        }
        RegistryCmd::List => {
            for row in store.list_registries()? {
                println!(
                    "{} enforcement={} provenance={}",
                    row.name, row.enforcement, row.provenance
                );
            }
        }
        RegistryCmd::AddKey {
            registry,
            label,
            public_key,
        } => {
            let fp = store.insert_registry_key(
                &registry,
                &label,
                &registry::parse_hex_key(&std::fs::read_to_string(&public_key)?, 32)?,
            )?;
            println!("trusted operator key {fp} on {registry}");
        }
        RegistryCmd::BindIdentity {
            registry,
            nick,
            email,
        } => {
            store.insert_identity(&registry, &nick, &email)?;
            println!(
                "operator identity {nick} recorded on {registry} (not an AUR or Arch verification)"
            );
        }
        RegistryCmd::Expect {
            registry,
            pkgbase,
            repository,
            version,
            signature,
            key_fingerprint,
            nick,
        } => {
            let signature = registry::parse_hex_key(&std::fs::read_to_string(&signature)?, 64)?;
            store.insert_expectation(
                &registry,
                &pkgbase,
                &repository,
                &version,
                &signature,
                &key_fingerprint,
                &nick,
            )?;
            println!("operator expectation for {pkgbase} recorded on {registry}");
        }
        RegistryCmd::Sign {
            secret_key,
            registry,
            repository,
            pkgbase,
            version,
            srcinfo,
            pkgbuild,
        } => {
            let seed = registry::parse_hex_key(&std::fs::read_to_string(&secret_key)?, 32)?;
            let seed: [u8; 32] = seed
                .as_slice()
                .try_into()
                .map_err(|_| "secret key must be 32 bytes")?;
            let sig = registry::sign_recipe(
                &seed,
                &registry,
                &repository,
                &pkgbase,
                &version,
                &std::fs::read(&srcinfo)?,
                &std::fs::read(&pkgbuild)?,
            );
            println!("{}", hex_encode(&sig));
        }
        RegistryCmd::Keygen { dir } => {
            std::fs::create_dir_all(&dir)?;
            let (signing, verifying) = registry::generate_key();
            let secret = dir.join("seed.hex");
            let public = dir.join("public.hex");
            std::fs::write(&secret, hex_encode(signing.as_bytes()))?;
            taur_core::set_mode(&secret, 0o600)?;
            std::fs::write(&public, hex_encode(verifying.as_bytes()))?;
            println!(
                "wrote {} and {} (fingerprint {})",
                secret.display(),
                public.display(),
                registry::fingerprint(verifying.as_bytes())
            );
        }
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// `taur hook pre-receive|post-receive`, run by git inside the bare repo.
fn hook(name: &str) -> i32 {
    let result = (|| -> taur_core::Result<()> {
        let home = std::env::var_os("TAUR_HOME")
            .ok_or_else(|| taur_core::Error::Invalid("TAUR_HOME not set".into()))?;
        let pusher = std::env::var("TAUR_PUSHER")
            .map_err(|_| taur_core::Error::Invalid("pushes must be authenticated".into()))?;
        let repo = std::env::current_dir()?;
        let pkgbase = repo
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".git"))
            .ok_or_else(|| taur_core::Error::Invalid("not a TAUR repository".into()))?
            .to_string();
        let store = Store::open(std::path::Path::new(&home))?;
        if let (Ok(from), Ok(base)) = (
            std::env::var("TAUR_MAIL_FROM"),
            std::env::var("TAUR_BASE_URL"),
        ) {
            let smtp = std::env::var("TAUR_SMTP_URL").ok();
            store.configure_mail(smtp.as_deref(), &from, &base)?;
        }
        match name {
            "pre-receive" => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input)?;
                let updates = taur_core::push::parse_updates(&input);
                store.pre_receive(&repo, &pkgbase, &pusher, &updates)
            }
            "post-receive" => store.post_receive(&repo, &pkgbase, &pusher),
            _ => Err(taur_core::Error::Invalid(format!("unknown hook {name}"))),
        }
    })();
    match result {
        Ok(()) => 0,
        Err(taur_core::Error::Invalid(m)) => {
            eprintln!("error: {m}");
            1
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
