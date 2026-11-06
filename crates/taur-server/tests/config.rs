//! `taur serve` settings: env parsing, flag precedence, validation, admin bootstrap.
//! One test per binary section because it mutates the process environment.

use clap::Parser;
use taur_server::cli::{Cli, Cmd, ENV_VARS, ServeArgs, bootstrap_admins};

const VARS: &[&str] = &[
    "TAUR_HOME",
    "TAUR_BIND",
    "TAUR_BASE_URL",
    "TAUR_SSH_BIND",
    "TAUR_SSH_PUBLIC_HOST",
    "TAUR_SSH_PUBLIC_PORT",
    "TAUR_GRPC_BIND",
    "TAUR_SMTP_URL",
    "TAUR_MAIL_FROM",
    "TAUR_ADMINS",
];

fn serve(args: &[&str]) -> Result<ServeArgs, clap::Error> {
    let mut v = vec!["taur", "serve"];
    v.extend_from_slice(args);
    match Cli::try_parse_from(v)?.cmd {
        Cmd::Serve(a) => Ok(a),
        _ => unreachable!(),
    }
}

fn set(k: &str, v: &str) {
    // SAFETY: this test binary runs a single test that touches the environment.
    unsafe { std::env::set_var(k, v) }
}

fn clear() {
    for k in VARS {
        unsafe { std::env::remove_var(k) }
    }
}

#[test]
fn env_flags_validation_and_admins() {
    clear();
    // defaults
    let c = serve(&[]).unwrap().validate().unwrap();
    assert_eq!(c.bind.to_string(), "127.0.0.1:8080");
    assert_eq!(c.base_url, "https://packages.taur.weirdbrains.com");
    assert_eq!(c.mail_from, "TAUR <noreply@taur.weirdbrains.com>");
    assert!(c.ssh_bind.is_none() && c.grpc_bind.is_none() && c.smtp_url.is_none());
    assert!(c.admins.is_empty() && c.ssh_clone_base.is_none());

    // everything from the environment
    set("TAUR_BIND", "0.0.0.0:9000");
    set("TAUR_BASE_URL", "https://pkgs.example.org/");
    set("TAUR_SSH_BIND", "0.0.0.0:2222");
    set("TAUR_GRPC_BIND", "127.0.0.1:50051");
    set("TAUR_SMTP_URL", "smtps://u:secret@smtp.example.org");
    set("TAUR_MAIL_FROM", "noreply@example.org");
    set("TAUR_ADMINS", "alice, bob,alice");
    let c = serve(&[]).unwrap().validate().unwrap();
    assert_eq!(c.bind.to_string(), "0.0.0.0:9000");
    assert_eq!(c.base_url, "https://pkgs.example.org");
    assert_eq!(c.ssh_bind.unwrap().port(), 2222);
    assert_eq!(
        c.ssh_clone_base.as_deref(),
        Some("ssh://taur@pkgs.example.org:2222")
    );
    assert_eq!(c.grpc_bind.unwrap().port(), 50051);
    assert!(c.smtp_url.is_some());
    assert_eq!(c.mail_from, "noreply@example.org");
    assert_eq!(c.admins, vec!["alice", "bob"]);
    set("TAUR_SSH_PUBLIC_HOST", "git.example.org");
    set("TAUR_SSH_PUBLIC_PORT", "22");
    let c = serve(&[]).unwrap().validate().unwrap();
    assert_eq!(
        c.ssh_clone_base.as_deref(),
        Some("ssh://taur@git.example.org")
    );

    // flags override the environment
    let c = serve(&[
        "--bind",
        "127.0.0.1:7000",
        "--base-url",
        "http://localhost:7000",
        "--admins",
        "carol",
    ])
    .unwrap()
    .validate()
    .unwrap();
    assert_eq!(c.bind.port(), 7000);
    assert_eq!(c.base_url, "http://localhost:7000");
    assert_eq!(c.admins, vec!["carol"]);
    let home = tempfile::tempdir().unwrap();
    set("TAUR_HOME", home.path().to_str().unwrap());
    assert_eq!(
        Cli::try_parse_from(["taur", "serve"])
            .unwrap()
            .home
            .as_deref(),
        Some(home.path())
    );
    assert_eq!(
        Cli::try_parse_from(["taur", "--home", "/x", "serve"])
            .unwrap()
            .home
            .unwrap()
            .to_str(),
        Some("/x")
    );

    // validation errors name the variable and never echo the SMTP URL
    let bad = |k: &str, v: &str| {
        set(k, v);
        let e = match serve(&[]) {
            Ok(a) => a.validate().unwrap_err(),
            Err(e) => e.to_string(),
        };
        clear();
        e
    };
    assert!(bad("TAUR_BIND", "localhost").contains("TAUR_BIND"));
    assert!(bad("TAUR_SSH_BIND", "2222").contains("TAUR_SSH_BIND"));
    assert!(bad("TAUR_GRPC_BIND", "x:y").contains("TAUR_GRPC_BIND"));
    assert!(bad("TAUR_BASE_URL", "taur.example.org").contains("TAUR_BASE_URL"));
    assert!(bad("TAUR_BASE_URL", "https://x.org/sub").contains("TAUR_BASE_URL"));
    let e = bad("TAUR_SMTP_URL", "http://u:topsecret@x");
    assert!(
        e.contains("TAUR_SMTP_URL") && !e.contains("topsecret"),
        "{e}"
    );
    assert!(bad("TAUR_MAIL_FROM", "nobody").contains("TAUR_MAIL_FROM"));
    assert!(bad("TAUR_ADMINS", "al ice").contains("TAUR_ADMINS"));
    assert!(bad("TAUR_SSH_PUBLIC_PORT", "99999").contains("ssh-public-port"));

    // admin bootstrap: grant-only, idempotent, warns about unknown users
    let d = tempfile::tempdir().unwrap();
    let store = taur_core::Store::open(d.path()).unwrap();
    let a = store.register("alice", "alice@x", "longenough").unwrap().id;
    let names = vec!["alice".to_string(), "ghost".to_string()];
    let w = bootstrap_admins(&store, &names);
    assert_eq!(w.len(), 1);
    assert!(w[0].contains("ghost"));
    assert!(store.is_admin(a).unwrap());
    assert_eq!(bootstrap_admins(&store, &names[..1]).len(), 0);
    assert!(store.is_admin(a).unwrap());
    bootstrap_admins(&store, &[]);
    assert!(
        store.is_admin(a).unwrap(),
        "removing a name must not revoke"
    );

    // every variable is documented
    let doc = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/configuration.md"
    ))
    .unwrap();
    let example = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/taur.env.example"
    ))
    .unwrap();
    for (n, _, _) in ENV_VARS {
        assert!(doc.contains(n), "{n} missing from docs/configuration.md");
        assert!(
            example.contains(n),
            "{n} missing from packaging/taur.env.example"
        );
    }
    // .env.example has an uncommented entry for every variable, incl. taurctl's.
    let dotenv =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env.example"))
            .unwrap();
    for n in ENV_VARS
        .iter()
        .map(|v| v.0)
        .chain(["TAUR_SERVER", "TAURCTL_CONFIG_DIR"])
    {
        assert!(
            dotenv.lines().any(|l| l.starts_with(&format!("{n}="))),
            "{n} missing from .env.example"
        );
        assert!(doc.contains(n), "{n} missing from docs/configuration.md");
    }
}
