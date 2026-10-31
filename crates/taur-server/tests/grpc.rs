//! gRPC API against a real tonic server on a temp home.

use std::path::PathBuf;
use std::sync::Arc;
use taur_core::Store;
use taur_server::grpc::pb::*;
use tonic::{Code, Request};

fn authed<T>(msg: T, token: &str) -> Request<T> {
    let mut r = Request::new(msg);
    r.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    r
}

#[tokio::test]
async fn grpc_api() {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(home.path()).unwrap());
    let fx = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/hello-taur");
    store.import_dir(&fx, None).unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(taur_server::grpc::serve(l, store.clone()));
    let ch = tonic::transport::Endpoint::from_shared(url)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut pkgs = packages_client::PackagesClient::new(ch.clone());
    let mut acct = accounts_client::AccountsClient::new(ch.clone());
    let mut comm = community_client::CommunityClient::new(ch.clone());
    let mut orgs = orgs_client::OrgsClient::new(ch.clone());
    let mut manage = manage_client::ManageClient::new(ch.clone());
    let mut reqs = requests_client::RequestsClient::new(ch.clone());
    let mut keys = ssh_keys_client::SshKeysClient::new(ch);

    // public
    let found = pkgs
        .search(SearchRequest {
            query: "hello".into(),
            by: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(found.packages.len(), 2);
    let info = pkgs
        .info(InfoRequest {
            names: vec!["hello-taur".into()],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.packages[0].pkgbase, "hello-taur");
    assert!(info.packages[0].relations.contains_key("license"));
    let e = pkgs
        .search(SearchRequest {
            query: "x".into(),
            by: "nope".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);

    // accounts
    acct.register(RegisterRequest {
        name: "alice".into(),
        email: "alice@x".into(),
        password: "longenough".into(),
    })
    .await
    .unwrap();
    let t =
        taur_core::mail::FileMailer::find_token(&home.path().join("outbox"), "alice@x", "verify")
            .unwrap();
    assert!(
        acct.verify(TokenRequest { token: t })
            .await
            .unwrap()
            .into_inner()
            .verified
    );
    let tok = acct
        .login(LoginRequest {
            name: "alice".into(),
            password: "longenough".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .token;
    assert_eq!(
        acct.whoami(authed(Empty {}, &tok))
            .await
            .unwrap()
            .into_inner()
            .name,
        "alice"
    );
    assert_eq!(
        acct.whoami(Empty {}).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    assert_eq!(
        acct.whoami(authed(Empty {}, "bogus"))
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );

    // community
    comm.vote(authed(
        Toggle {
            pkgbase: "hello-taur".into(),
            on: true,
        },
        &tok,
    ))
    .await
    .unwrap();
    assert!(
        comm.vote_status(authed(
            PkgbaseRequest {
                pkgbase: "hello-taur".into()
            },
            &tok
        ))
        .await
        .unwrap()
        .into_inner()
        .value
    );
    comm.notify(authed(
        Toggle {
            pkgbase: "hello-taur".into(),
            on: true,
        },
        &tok,
    ))
    .await
    .unwrap();
    assert!(
        comm.notify_status(authed(
            PkgbaseRequest {
                pkgbase: "hello-taur".into()
            },
            &tok
        ))
        .await
        .unwrap()
        .into_inner()
        .value
    );
    let c = comm
        .add_comment(authed(
            AddCommentRequest {
                pkgbase: "hello-taur".into(),
                content: "hi".into(),
            },
            &tok,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        comm.pin_comment(authed(
            PinRequest {
                id: c.id,
                pinned: true
            },
            &tok
        ))
        .await
        .unwrap_err()
        .code(),
        Code::PermissionDenied
    );
    assert_eq!(
        pkgs.comments(PkgbaseRequest {
            pkgbase: "hello-taur".into()
        })
        .await
        .unwrap()
        .into_inner()
        .comments[0]
            .content,
        "hi"
    );
    comm.flag_out_of_date(authed(
        FlagRequest {
            pkgbase: "hello-taur".into(),
            comment: "old".into(),
        },
        &tok,
    ))
    .await
    .unwrap();
    assert_eq!(
        pkgs.get_flag(PkgbaseRequest {
            pkgbase: "hello-taur".into()
        })
        .await
        .unwrap()
        .into_inner()
        .flagger
        .as_deref(),
        Some("alice")
    );

    // management: adopt the orphan, move it to an org
    manage
        .adopt(authed(
            PkgbaseRequest {
                pkgbase: "hello-taur".into(),
            },
            &tok,
        ))
        .await
        .unwrap();
    orgs.create(authed(
        OrgRequest {
            org: "weirdbrains".into(),
        },
        &tok,
    ))
    .await
    .unwrap();
    manage
        .transfer(authed(
            TransferRequest {
                pkgbase: "hello-taur".into(),
                to: Some(transfer_request::To::Org("weirdbrains".into())),
            },
            &tok,
        ))
        .await
        .unwrap();
    let mine = manage
        .my_packages(authed(Empty {}, &tok))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(mine.packages[0].org.as_deref(), Some("weirdbrains"));
    assert_eq!(
        orgs.list(authed(Empty {}, &tok))
            .await
            .unwrap()
            .into_inner()
            .orgs[0]
            .role,
        "owner"
    );
    assert_eq!(
        orgs.set_member(authed(
            SetMemberRequest {
                org: "weirdbrains".into(),
                user: "alice".into(),
                role: "boss".into()
            },
            &tok
        ))
        .await
        .unwrap_err()
        .code(),
        Code::InvalidArgument
    );

    // requests
    let r = reqs
        .file(authed(
            FileRequestRequest {
                pkgbase: "hello-taur".into(),
                kind: "deletion".into(),
                merge_into: None,
                comment: "dup".into(),
            },
            &tok,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r.status, "pending");
    assert_eq!(
        reqs.list(authed(ListRequestsRequest { all: false }, &tok))
            .await
            .unwrap()
            .into_inner()
            .requests
            .len(),
        1
    );
    reqs.reject(authed(
        CloseRequestRequest {
            id: r.id,
            comment: String::new(),
        },
        &tok,
    ))
    .await
    .unwrap();
    assert_ne!(
        reqs.get(authed(IdRequest { id: r.id }, &tok))
            .await
            .unwrap()
            .into_inner()
            .status,
        "pending"
    );

    // ssh keys
    let k = keys
        .add(authed(AddSshKeyRequest { key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl test".into() }, &tok))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        keys.list(authed(Empty {}, &tok))
            .await
            .unwrap()
            .into_inner()
            .keys
            .len(),
        1
    );
    keys.remove(authed(IdRequest { id: k.id }, &tok))
        .await
        .unwrap();

    acct.logout(authed(Empty {}, &tok)).await.unwrap();
    assert_eq!(
        acct.whoami(authed(Empty {}, &tok))
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
}
