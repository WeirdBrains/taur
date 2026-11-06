//! gRPC API (tonic), mirroring the JSON API. Auth: `authorization: Bearer <token>` metadata.

use std::sync::Arc;
use taur_core::{Error, LimitClass, Role, SearchBy, Store, accounts::User};
use tonic::{Request, Response, Status};

pub mod pb {
    tonic::include_proto!("taur.v1");
}
use pb::*;

type R<T> = Result<Response<T>, Status>;

fn status(e: Error) -> Status {
    match e {
        Error::Unauthorized => Status::unauthenticated(e.to_string()),
        Error::Forbidden | Error::Unverified => Status::permission_denied(e.to_string()),
        Error::NotFound(m) => Status::not_found(m),
        Error::Conflict(m) => Status::already_exists(m),
        Error::Invalid(m) | Error::Parse(m) => Status::invalid_argument(m),
        Error::RateLimited { retry_after } => {
            let mut st = Status::resource_exhausted("rate limit reached");
            if let Ok(value) = retry_after.to_string().parse() {
                let _ = st.metadata_mut().insert("retry-after", value);
            }
            st
        }
        other => {
            // Error values never contain passwords or tokens.
            eprintln!("taur: grpc internal error: {other}");
            Status::internal("internal error")
        }
    }
}

fn ok<T>(r: taur_core::Result<T>) -> R<T> {
    r.map(Response::new).map_err(status)
}

fn empty(r: taur_core::Result<()>) -> R<Empty> {
    ok(r.map(|_| Empty {}))
}

fn token<T>(req: &Request<T>) -> taur_core::Result<String> {
    req.metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or(Error::Unauthorized)
}

/// Shared service state; one instance implements every service.
#[derive(Clone)]
pub struct Svc {
    store: Arc<Store>,
}

impl Svc {
    fn limit<T>(&self, req: &Request<T>, class: LimitClass) -> Result<(), Status> {
        let peer = req.remote_addr().map(|a| a.ip());
        let forwarded = req
            .metadata()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok());
        self.store
            .enforce_limit(class, peer, forwarded)
            .map_err(status)
    }

    fn user<T>(&self, req: &Request<T>) -> taur_core::Result<User> {
        self.store.user_for_token(&token(req)?)
    }
    fn verified<T>(&self, req: &Request<T>) -> taur_core::Result<User> {
        let u = self.user(req)?;
        self.store.require_verified(&u)?;
        Ok(u)
    }
    fn pb(&self, name: &str) -> taur_core::Result<i64> {
        self.store
            .pkgbase_id(name)?
            .ok_or_else(|| Error::NotFound(format!("package base {name}")))
    }
}

fn package(p: taur_core::PkgInfo) -> Package {
    Package {
        id: p.id,
        url_path: taur_core::rpc::url_path(&p.pkgbase),
        name: p.name,
        pkgbase_id: p.pkgbase_id,
        pkgbase: p.pkgbase,
        version: p.version,
        description: p.description,
        url: p.url,
        num_votes: p.num_votes,
        popularity: p.popularity,
        out_of_date: p.out_of_date,
        maintainer: p.maintainer,
        submitter: p.submitter,
        first_submitted: p.first_submitted,
        last_modified: p.last_modified,
        organization: p.organization,
        relations: p
            .relations
            .into_iter()
            .map(|(k, values)| (k, StringList { values }))
            .collect(),
        keywords: p.keywords,
        comaintainers: p.comaintainers,
    }
}

fn comment(c: taur_core::community::Comment) -> Comment {
    Comment {
        id: c.id,
        author: c.author,
        content: c.content,
        created_at: c.created_at,
        edited_at: c.edited_at,
        pinned: c.pinned,
    }
}

fn user(u: User) -> pb::User {
    pb::User {
        id: u.id,
        name: u.name,
        email: u.email,
        verified: u.verified,
    }
}

fn request(r: taur_core::community::Request) -> PackageRequest {
    PackageRequest {
        id: r.id,
        pkgbase: r.pkgbase,
        requester: r.requester,
        kind: r.kind,
        merge_into: r.merge_into,
        comments: r.comments,
        status: r.status,
        closure_comment: r.closure_comment,
        created_at: r.created_at,
        closed_at: r.closed_at,
    }
}

fn role(s: &str) -> taur_core::Result<Role> {
    Role::parse(s).ok_or_else(|| Error::Invalid("role must be owner, admin or contributor".into()))
}

#[tonic::async_trait]
impl packages_server::Packages for Svc {
    async fn search(&self, req: Request<SearchRequest>) -> R<PackageList> {
        self.limit(&req, LimitClass::Grpc)?;
        let r = req.into_inner();
        ok((|| {
            let by_s = if r.by.is_empty() {
                "name-desc"
            } else {
                r.by.as_str()
            };
            let by = SearchBy::parse(by_s)
                .ok_or_else(|| Error::Invalid(format!("unknown search field {by_s}")))?;
            let found = self.store.search(by, &r.query)?;
            if found.len() > taur_core::rpc::MAX_RESULTS {
                return Err(Error::Invalid("Too many package results.".into()));
            }
            Ok(PackageList {
                packages: found.into_iter().map(package).collect(),
            })
        })())
    }
    async fn info(&self, req: Request<InfoRequest>) -> R<PackageList> {
        self.limit(&req, LimitClass::Grpc)?;
        let names = req.into_inner().names;
        ok(self.store.info(&names).map(|v| PackageList {
            packages: v.into_iter().map(package).collect(),
        }))
    }
    async fn comments(&self, req: Request<PkgbaseRequest>) -> R<CommentList> {
        self.limit(&req, LimitClass::Grpc)?;
        ok(self
            .store
            .comments(&req.into_inner().pkgbase)
            .map(|v| CommentList {
                comments: v.into_iter().map(comment).collect(),
            }))
    }
    async fn get_flag(&self, req: Request<PkgbaseRequest>) -> R<Flag> {
        self.limit(&req, LimitClass::Grpc)?;
        ok(self
            .store
            .flag_info(&req.into_inner().pkgbase)
            .map(|f| Flag {
                out_of_date: f.out_of_date,
                flagger: f.flagger,
                comment: f.comment,
            }))
    }
}

#[tonic::async_trait]
impl accounts_server::Accounts for Svc {
    async fn register(&self, req: Request<RegisterRequest>) -> R<pb::User> {
        self.limit(&req, LimitClass::Auth)?;
        let r = req.into_inner();
        ok(self
            .store
            .register(&r.name, &r.email, &r.password)
            .map(user))
    }
    async fn login(&self, req: Request<LoginRequest>) -> R<Session> {
        self.limit(&req, LimitClass::Auth)?;
        let r = req.into_inner();
        ok(self.store.login(&r.name, &r.password).map(|s| Session {
            token: s.token,
            expires_at: s.expires_at,
        }))
    }
    async fn logout(&self, req: Request<Empty>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty(token(&req).and_then(|t| self.store.logout(&t)))
    }
    async fn whoami(&self, req: Request<Empty>) -> R<pb::User> {
        self.limit(&req, LimitClass::Grpc)?;
        ok(self.user(&req).map(user))
    }
    async fn unregister(&self, req: Request<PasswordRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty(token(&req).and_then(|t| self.store.unregister(&t, &req.get_ref().password)))
    }
    async fn verify(&self, req: Request<TokenRequest>) -> R<pb::User> {
        self.limit(&req, LimitClass::Auth)?;
        ok(self.store.verify_email(&req.into_inner().token).map(user))
    }
    async fn resend_verification(&self, req: Request<Empty>) -> R<Empty> {
        self.limit(&req, LimitClass::Auth)?;
        empty(
            self.user(&req)
                .and_then(|u| self.store.resend_verification(&u)),
        )
    }
    async fn request_password_reset(&self, req: Request<EmailRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Auth)?;
        empty(self.store.request_password_reset(&req.into_inner().email))
    }
    async fn reset_password(&self, req: Request<ResetPasswordRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Auth)?;
        let r = req.into_inner();
        empty(self.store.reset_password(&r.token, &r.password))
    }
}

#[tonic::async_trait]
impl orgs_server::Orgs for Svc {
    async fn list(&self, req: Request<Empty>) -> R<MembershipList> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            Ok(MembershipList {
                orgs: self
                    .store
                    .orgs_for(u.id)?
                    .into_iter()
                    .map(|m| Membership {
                        name: m.name,
                        role: m.role.as_str().into(),
                    })
                    .collect(),
            })
        })())
    }
    async fn get(&self, req: Request<OrgRequest>) -> R<OrgDetail> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            self.org_detail(u.id, &req.get_ref().org)
        })())
    }
    async fn create(&self, req: Request<OrgRequest>) -> R<OrgDetail> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.verified(&req)?;
            self.store.create_org(u.id, &req.get_ref().org)?;
            self.org_detail(u.id, &req.get_ref().org)
        })())
    }
    async fn delete(&self, req: Request<OrgRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            self.store
                .delete_org(u.id, self.store.org_id(&req.get_ref().org)?)
        })())
    }
    async fn set_member(&self, req: Request<SetMemberRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.set_member(
                u.id,
                self.store.org_id(&r.org)?,
                self.store.user_id(&r.user)?,
                role(&r.role)?,
            )
        })())
    }
    async fn remove_member(&self, req: Request<MemberRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.remove_member(
                u.id,
                self.store.org_id(&r.org)?,
                self.store.user_id(&r.user)?,
            )
        })())
    }
}

impl Svc {
    fn org_detail(&self, uid: i64, name: &str) -> taur_core::Result<OrgDetail> {
        let d = self.store.org_detail(uid, name)?;
        Ok(OrgDetail {
            id: d.id,
            name: d.name,
            members: d
                .members
                .into_iter()
                .map(|m| Membership {
                    name: m.name,
                    role: m.role.as_str().into(),
                })
                .collect(),
        })
    }
}

#[tonic::async_trait]
impl manage_server::Manage for Svc {
    async fn my_packages(&self, req: Request<Empty>) -> R<OwnedPackageList> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            Ok(OwnedPackageList {
                packages: self
                    .store
                    .packages_for(u.id)?
                    .into_iter()
                    .map(|p| OwnedPackage {
                        pkgbase: p.pkgbase,
                        maintainer: p.maintainer,
                        org: p.org,
                    })
                    .collect(),
            })
        })())
    }
    async fn transfer(&self, req: Request<TransferRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            let p = self.pb(&r.pkgbase)?;
            match &r.to {
                Some(transfer_request::To::Org(o)) => {
                    self.store
                        .set_pkgbase_org(u.id, p, Some(self.store.org_id(o)?))
                }
                Some(transfer_request::To::User(n)) => {
                    self.store.transfer_to_user(u.id, p, self.store.user_id(n)?)
                }
                None => Err(Error::Invalid("give exactly one of org or user".into())),
            }
        })())
    }
    async fn disown(&self, req: Request<PkgbaseRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            self.store.disown(u.id, self.pb(&req.get_ref().pkgbase)?)
        })())
    }
    async fn adopt(&self, req: Request<PkgbaseRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            self.store.adopt(u.id, self.pb(&req.get_ref().pkgbase)?)?;
            Ok(())
        })())
    }
    async fn set_comaintainers(&self, req: Request<ComaintainersRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.set_comaintainers(u.id, &r.pkgbase, &r.users)
        })())
    }
}

#[tonic::async_trait]
impl community_server::Community for Svc {
    async fn vote(&self, req: Request<Toggle>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            if r.on {
                self.store.vote(u.id, &r.pkgbase)
            } else {
                self.store.unvote(u.id, &r.pkgbase)
            }
        })())
    }
    async fn vote_status(&self, req: Request<PkgbaseRequest>) -> R<pb::Status> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            Ok(pb::Status {
                value: self.store.has_voted(u.id, &req.get_ref().pkgbase)?,
            })
        })())
    }
    async fn notify(&self, req: Request<Toggle>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = if req.get_ref().on {
                self.verified(&req)?
            } else {
                self.user(&req)?
            };
            let r = req.get_ref();
            self.store.subscribe(u.id, &r.pkgbase, r.on)
        })())
    }
    async fn notify_status(&self, req: Request<PkgbaseRequest>) -> R<pb::Status> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            Ok(pb::Status {
                value: self.store.is_subscribed(u.id, &req.get_ref().pkgbase)?,
            })
        })())
    }
    async fn flag_out_of_date(&self, req: Request<FlagRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.flag_out_of_date(u.id, &r.pkgbase, &r.comment)
        })())
    }
    async fn unflag(&self, req: Request<PkgbaseRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            self.store.unflag(u.id, &req.get_ref().pkgbase)
        })())
    }
    async fn add_comment(&self, req: Request<AddCommentRequest>) -> R<Comment> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store
                .add_comment(u.id, &r.pkgbase, &r.content)
                .map(comment)
        })())
    }
    async fn edit_comment(&self, req: Request<EditCommentRequest>) -> R<Comment> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.edit_comment(u.id, r.id, &r.content).map(comment)
        })())
    }
    async fn delete_comment(&self, req: Request<IdRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            self.store.delete_comment(u.id, req.get_ref().id)
        })())
    }
    async fn pin_comment(&self, req: Request<PinRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.pin_comment(u.id, r.id, r.pinned)
        })())
    }
}

#[tonic::async_trait]
impl requests_server::Requests for Svc {
    async fn file(&self, req: Request<FileRequestRequest>) -> R<PackageRequest> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store
                .file_request(
                    u.id,
                    &r.pkgbase,
                    &r.kind,
                    r.merge_into.as_deref(),
                    &r.comment,
                )
                .map(request)
        })())
    }
    async fn list(&self, req: Request<ListRequestsRequest>) -> R<PackageRequestList> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            self.user(&req)?;
            Ok(PackageRequestList {
                requests: self
                    .store
                    .list_requests(!req.get_ref().all)?
                    .into_iter()
                    .map(request)
                    .collect(),
            })
        })())
    }
    async fn get(&self, req: Request<IdRequest>) -> R<PackageRequest> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            self.user(&req)?;
            self.store.request(req.get_ref().id).map(request)
        })())
    }
    async fn accept(&self, req: Request<CloseRequestRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.accept_request(u.id, r.id, &r.comment)
        })())
    }
    async fn reject(&self, req: Request<CloseRequestRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.verified(&req)?;
            let r = req.get_ref();
            self.store.reject_request(u.id, r.id, &r.comment)
        })())
    }
}

#[tonic::async_trait]
impl ssh_keys_server::SshKeys for Svc {
    async fn list(&self, req: Request<Empty>) -> R<SshKeyList> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.user(&req)?;
            Ok(SshKeyList {
                keys: self
                    .store
                    .list_ssh_keys(u.id)?
                    .into_iter()
                    .map(ssh_key)
                    .collect(),
            })
        })())
    }
    async fn add(&self, req: Request<AddSshKeyRequest>) -> R<SshKey> {
        self.limit(&req, LimitClass::Grpc)?;
        ok((|| {
            let u = self.verified(&req)?;
            self.store
                .add_ssh_key(u.id, &req.get_ref().key)
                .map(ssh_key)
        })())
    }
    async fn remove(&self, req: Request<IdRequest>) -> R<Empty> {
        self.limit(&req, LimitClass::Grpc)?;
        empty((|| {
            let u = self.user(&req)?;
            self.store.remove_ssh_key(u.id, req.get_ref().id)
        })())
    }
}

fn ssh_key(k: taur_core::keys::SshKey) -> SshKey {
    SshKey {
        id: k.id,
        fingerprint: k.fingerprint,
        algorithm: k.algorithm,
        comment: k.comment,
        created_at: k.created_at,
    }
}

/// All TAUR gRPC services as one tonic router.
pub fn routes(store: Arc<Store>) -> tonic::service::Routes {
    let s = Svc { store };
    tonic::service::Routes::new(packages_server::PackagesServer::new(s.clone()))
        .add_service(accounts_server::AccountsServer::new(s.clone()))
        .add_service(orgs_server::OrgsServer::new(s.clone()))
        .add_service(manage_server::ManageServer::new(s.clone()))
        .add_service(community_server::CommunityServer::new(s.clone()))
        .add_service(requests_server::RequestsServer::new(s.clone()))
        .add_service(ssh_keys_server::SshKeysServer::new(s))
}

/// Serve gRPC as plaintext HTTP/2. This listener is not TLS.
pub async fn serve(
    listener: tokio::net::TcpListener,
    store: Arc<Store>,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_routes(routes(store))
        .serve_with_incoming(tonic::transport::server::TcpIncoming::from(listener))
        .await
}

/// Serve gRPC over TLS with the same certificate resolver as HTTPS.
pub async fn serve_tls(
    listener: tokio::net::TcpListener,
    store: Arc<Store>,
    config: std::sync::Arc<rustls::ServerConfig>,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_routes(routes(store))
        .serve_with_incoming(crate::acme::tls_incoming(listener, config))
        .await
}
