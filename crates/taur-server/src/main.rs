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
    if let Err(e) = run(cli).await {
        eprintln!("taur: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open(&resolve_home(cli.home))?;
    match cli.cmd {
        Cmd::Serve {
            bind,
            ssh_bind,
            base_url,
            smtp_url,
            mail_from,
        } => {
            store.set_base_url(&base_url);
            match smtp_url {
                Some(url) => {
                    store.set_mailer(Arc::new(taur_core::mail::SmtpMailer::new(
                        &url, &mail_from,
                    )?));
                    eprintln!("taur: sending mail via SMTP");
                }
                None => {
                    let outbox = store.home().join("outbox");
                    store.set_mailer(Arc::new(taur_core::mail::FileMailer::new(
                        outbox.clone(),
                        &mail_from,
                    )));
                    eprintln!(
                        "taur: no SMTP configured; writing mail to {}",
                        outbox.display()
                    );
                }
            }
            let store = Arc::new(store);
            if let Some(addr) = ssh_bind {
                let hook_bin = std::env::current_exe()?;
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                eprintln!("taur: ssh on {}", listener.local_addr()?);
                tokio::spawn(taur_server::ssh::serve(listener, store.clone(), hook_bin));
            }
            let listener = tokio::net::TcpListener::bind(&bind).await?;
            eprintln!("taur: listening on http://{}", listener.local_addr()?);
            axum::serve(listener, taur_server::app(store)).await?;
        }
        Cmd::Import { dir, maintainer } => {
            let info = store.import_dir(&dir, maintainer.as_deref())?;
            println!("imported {} {}", info.pkgbase, info.version());
        }
        Cmd::VerifyUser { name } => {
            let user = store
                .user_by_name(&name)?
                .ok_or_else(|| taur_core::Error::NotFound(format!("user {name}")))?;
            store.mark_verified(user.id)?;
            println!("verified {name}");
        }
        Cmd::Hook { .. } => unreachable!(),
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
