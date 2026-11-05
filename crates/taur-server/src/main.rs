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
            if let Some(addr) = cfg.ssh_bind {
                let hook_bin = std::env::current_exe()?;
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                eprintln!("taur: ssh on {}", listener.local_addr()?);
                tokio::spawn(taur_server::ssh::serve(listener, store.clone(), hook_bin));
            }
            if let Some(addr) = cfg.grpc_bind {
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                eprintln!("taur: grpc on {}", listener.local_addr()?);
                let s3 = store.clone();
                tokio::spawn(async move {
                    if let Err(e) = taur_server::grpc::serve(listener, s3).await {
                        eprintln!("taur: grpc server stopped: {e}");
                    }
                });
            }
            let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
            eprintln!("taur: listening on http://{}", listener.local_addr()?);
            axum::serve(listener, taur_server::app(store)).await?;
        }
        Cmd::Import { dir, maintainer } => {
            let info = store.import_dir(&dir, maintainer.as_deref())?;
            println!("imported {} {}", info.pkgbase, info.version());
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
