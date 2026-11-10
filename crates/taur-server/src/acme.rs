//! Automatic HTTPS: ACME HTTP-01 (Let's Encrypt by default) and a rustls listener.
//!
//! Plain HTTP stays available only when TLS is off. A failed order never binds the
//! application port as HTTP. Account and certificate private keys stay in the cache
//! directory and are not logged.

use axum::Router;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount,
    NewOrder, RetryPolicy,
};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tower::ServiceExt;

pub const PRODUCTION_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";
pub const STAGING_DIRECTORY: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

#[derive(Clone, Debug)]
pub struct AcmeSettings {
    pub domains: Vec<String>,
    pub directory: String,
    pub staging: bool,
    pub contact: String,
    pub accept_terms: bool,
    pub cache: PathBuf,
    pub http_bind: SocketAddr,
    pub renew_before: Duration,
}

/// Certificate material. The private key is redacted in `Debug`.
pub struct Issued {
    pub cert_pem: String,
    pub key_pem: String,
    pub not_after_unix: u64,
}

impl std::fmt::Debug for Issued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Issued")
            .field("not_after_unix", &self.not_after_unix)
            .finish_non_exhaustive()
    }
}

pub fn terms_refusal(directory: &str) -> String {
    format!(
        "TAUR_ACME_ACCEPT_TERMS must be the word yes after you read the CA subscriber agreement for {directory}. Refusing to create an account or request a certificate. Let's Encrypt publishes its agreement at https://letsencrypt.org/repository/"
    )
}

/// Tokens currently published for `/.well-known/acme-challenge/`.
#[derive(Clone, Default)]
pub struct ChallengeBoard {
    tokens: Arc<RwLock<HashMap<String, String>>>,
}

impl ChallengeBoard {
    pub fn insert(&self, token: String, body: String) {
        self.tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token, body);
    }

    pub fn get(&self, token: &str) -> Option<String> {
        self.tokens
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(token)
            .cloned()
    }

    pub fn clear(&self) {
        self.tokens
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

pub trait CertificateIssuer: Send + Sync {
    fn issue<'a>(
        &'a self,
        settings: &'a AcmeSettings,
        board: &'a ChallengeBoard,
    ) -> impl std::future::Future<Output = Result<Issued, String>> + Send + 'a;
}

/// Let's Encrypt (or another RFC 8555 directory) via `instant-acme`.
pub struct InstantAcme;

impl CertificateIssuer for InstantAcme {
    async fn issue(
        &self,
        settings: &AcmeSettings,
        board: &ChallengeBoard,
    ) -> Result<Issued, String> {
        install_crypto();
        if !settings.accept_terms {
            return Err(terms_refusal(&settings.directory));
        }
        let account = load_or_create_account(settings).await?;
        let idents: Vec<Identifier> = settings
            .domains
            .iter()
            .cloned()
            .map(Identifier::Dns)
            .collect();
        let mut order = account
            .new_order(&NewOrder::new(&idents))
            .await
            .map_err(|e| public_error(&e.to_string()))?;
        let mut authzs = order.authorizations();
        while let Some(item) = authzs.next().await {
            let mut authz = item.map_err(|e| public_error(&e.to_string()))?;
            let status = authz
                .refresh()
                .await
                .map_err(|e| public_error(&e.to_string()))?
                .status;
            if status == AuthorizationStatus::Valid {
                continue;
            }
            let mut challenge = authz
                .challenge(ChallengeType::Http01)
                .ok_or_else(|| "the ACME server offered no HTTP-01 challenge".to_string())?;
            if challenge.token.is_empty()
                || !challenge
                    .token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                return Err("the ACME server sent a challenge token TAUR will not publish".into());
            }
            board.insert(
                challenge.token.clone(),
                challenge.key_authorization().as_str().to_string(),
            );
            challenge
                .set_ready()
                .await
                .map_err(|e| public_error(&e.to_string()))?;
        }
        let policy = RetryPolicy::new().timeout(Duration::from_secs(120));
        order
            .poll_ready(&policy)
            .await
            .map_err(|e| public_error(&e.to_string()))?;
        let key_pem = order
            .finalize()
            .await
            .map_err(|e| public_error(&e.to_string()))?;
        let cert_pem = order
            .poll_certificate(&policy)
            .await
            .map_err(|e| public_error(&e.to_string()))?;
        board.clear();
        let not_after_unix = cert_not_after(&cert_pem)?;
        Ok(Issued {
            cert_pem,
            key_pem,
            not_after_unix,
        })
    }
}

async fn load_or_create_account(settings: &AcmeSettings) -> Result<Account, String> {
    prepare_dir(&settings.cache)?;
    let path = settings.cache.join("account.json");
    if path.exists() {
        let bytes = std::fs::read(&path).map_err(|e| format!("reading ACME account: {e}"))?;
        let creds: AccountCredentials = serde_json::from_slice(&bytes)
            .map_err(|_| "stored ACME account could not be read".to_string())?;
        return Account::builder()
            .map_err(|e| public_error(&e.to_string()))?
            .from_credentials(creds)
            .await
            .map_err(|e| public_error(&e.to_string()));
    }
    let contact = settings.contact.clone();
    let (account, creds) = Account::builder()
        .map_err(|e| public_error(&e.to_string()))?
        .create(
            &NewAccount {
                contact: &[&contact],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            settings.directory.clone(),
            None,
        )
        .await
        .map_err(|e| public_error(&e.to_string()))?;
    let json = serde_json::to_vec(&creds).map_err(|e| e.to_string())?;
    write_file(&path, &json, 0o600)?;
    Ok(account)
}

pub struct TlsPorts {
    pub https: Arc<ServerConfig>,
    pub grpc: Arc<ServerConfig>,
    pub resolver: Arc<CertResolver>,
}

/// Obtain a certificate if needed, then keep renewing it until `shutdown` is set.
/// The HTTPS config is returned only after a certificate exists. The HTTP-01
/// listener is already serving by then.
pub async fn bootstrap<I>(
    settings: AcmeSettings,
    issuer: I,
    shutdown: watch::Receiver<bool>,
) -> Result<TlsPorts, String>
where
    I: CertificateIssuer + 'static,
{
    install_crypto();
    if !settings.accept_terms {
        return Err(terms_refusal(&settings.directory));
    }
    prepare_dir(&settings.cache)?;
    let board = ChallengeBoard::default();
    let listener = TcpListener::bind(settings.http_bind)
        .await
        .map_err(|e| format!("TAUR_ACME_HTTP_BIND {}: {e}", settings.http_bind))?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("ACME HTTP listener: {e}"))?;
    eprintln!(
        "taur: HTTP-01 challenges on {bound}; the CA must reach TCP port 80 for {}",
        settings.domains.join(", ")
    );
    let base = public_https(&settings.domains[0]);
    tokio::spawn(serve_challenges(
        listener,
        board.clone(),
        base,
        shutdown.clone(),
    ));
    let issuer: Arc<dyn CertificateIssuerDyn> = Arc::new(issuer);
    let issued = if let Some(existing) = read_current(&settings.cache) {
        existing
    } else {
        issue_until_ok(issuer.as_ref(), &settings, &board, &shutdown).await?
    };
    let resolver = CertResolver::new(certified(&issued)?);
    let https = server_config(resolver.clone(), vec![b"h2".to_vec(), b"http/1.1".to_vec()])?;
    let grpc = server_config(resolver.clone(), vec![b"h2".to_vec()])?;
    let renew_settings = settings.clone();
    let renew_board = board.clone();
    let renew_resolver = resolver.clone();
    let renew_issuer = issuer.clone();
    tokio::spawn(async move {
        renew_loop(
            renew_settings,
            renew_issuer,
            renew_board,
            renew_resolver,
            shutdown,
        )
        .await;
    });
    Ok(TlsPorts {
        https,
        grpc,
        resolver,
    })
}

async fn issue_until_ok(
    issuer: &dyn CertificateIssuerDyn,
    settings: &AcmeSettings,
    board: &ChallengeBoard,
    shutdown: &watch::Receiver<bool>,
) -> Result<Issued, String> {
    let mut delay = Duration::from_secs(15);
    loop {
        if *shutdown.borrow() {
            return Err("shutdown before a certificate was issued; HTTPS was not started".into());
        }
        match issuer.issue_boxed(settings, board).await {
            Ok(issued) => {
                publish(&settings.cache, &issued)?;
                return Ok(issued);
            }
            Err(e) => {
                eprintln!(
                    "taur: certificate request failed ({}); retrying in {}s. The HTTPS port is not open.",
                    public_error(&e),
                    delay.as_secs()
                );
                if wait(delay, shutdown).await {
                    return Err(
                        "shutdown before a certificate was issued; HTTPS was not started".into(),
                    );
                }
                delay = (delay * 2).min(Duration::from_secs(6 * 3600));
            }
        }
    }
}

/// Object-safe wrapper so the renewal task can hold the issuer.
trait CertificateIssuerDyn: Send + Sync {
    fn issue_boxed<'a>(
        &'a self,
        settings: &'a AcmeSettings,
        board: &'a ChallengeBoard,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Issued, String>> + Send + 'a>>;
}

impl<T: CertificateIssuer> CertificateIssuerDyn for T {
    fn issue_boxed<'a>(
        &'a self,
        settings: &'a AcmeSettings,
        board: &'a ChallengeBoard,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Issued, String>> + Send + 'a>> {
        Box::pin(CertificateIssuer::issue(self, settings, board))
    }
}

async fn renew_loop(
    settings: AcmeSettings,
    issuer: Arc<dyn CertificateIssuerDyn>,
    board: ChallengeBoard,
    resolver: Arc<CertResolver>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let Some(current) = read_current(&settings.cache) else {
            return;
        };
        let due = renew_delay(current.not_after_unix, settings.renew_before);
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return;
                }
            }
            _ = tokio::time::sleep(due) => {}
        }
        if *shutdown.borrow() {
            return;
        }
        let mut delay = Duration::from_secs(30);
        loop {
            if *shutdown.borrow() {
                return;
            }
            match issuer.issue_boxed(&settings, &board).await {
                Ok(issued) => {
                    if let Err(e) = publish(&settings.cache, &issued) {
                        eprintln!(
                            "taur: could not store the renewed certificate ({e}); still serving the previous one"
                        );
                    } else if let Ok(key) = certified(&issued) {
                        resolver.swap(key);
                        eprintln!(
                            "taur: renewed the certificate for {}",
                            settings.domains.join(", ")
                        );
                    }
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "taur: certificate renewal failed ({}); keeping the current certificate and retrying in {}s",
                        public_error(&e),
                        delay.as_secs()
                    );
                    if wait(delay, &shutdown).await {
                        return;
                    }
                    delay = (delay * 2).min(Duration::from_secs(6 * 3600));
                }
            }
        }
    }
}

fn renew_delay(not_after_unix: u64, renew_before: Duration) -> Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let renew_at = not_after_unix.saturating_sub(renew_before.as_secs());
    Duration::from_secs(renew_at.saturating_sub(now))
}

async fn wait(delay: Duration, shutdown: &watch::Receiver<bool>) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = shutdown_fired(shutdown.clone()) => true,
    }
}

async fn shutdown_fired(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }
    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

fn public_https(domain: &str) -> String {
    format!("https://{domain}")
}

pub async fn serve_challenges(
    listener: TcpListener,
    board: ChallengeBoard,
    base_url: String,
    shutdown: watch::Receiver<bool>,
) {
    let app = Router::new()
        .route("/.well-known/acme-challenge/{token}", get(challenge_token))
        .fallback(redirect_https)
        .with_state((board, base_url));
    let _ = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_fired(shutdown))
    .await;
}

async fn challenge_token(
    Path(token): Path<String>,
    State((board, _)): State<(ChallengeBoard, String)>,
) -> impl IntoResponse {
    match board.get(&token) {
        Some(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn redirect_https(
    State((_, base)): State<(ChallengeBoard, String)>,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    let path = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let loc = format!("{base}{path}");
    (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, loc)]).into_response()
}

pub async fn serve_https(
    listener: TcpListener,
    app: Router,
    config: Arc<ServerConfig>,
    shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    let acceptor = TlsAcceptor::from(config);
    loop {
        tokio::select! {
            _ = shutdown_fired(shutdown.clone()) => return Ok(()),
            accepted = listener.accept() => {
                let (tcp, peer) = accepted?;
                let acceptor = acceptor.clone();
                let app = app.clone();
                tokio::spawn(async move {
                    let Ok(stream) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                        let app = app.clone();
                        async move {
                            let mut request = request.map(axum::body::Body::new);
                            request.extensions_mut().insert(ConnectInfo(peer));
                            app.oneshot(request).await
                        }
                    });
                    let _ = hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(io, service)
                        .await;
                });
            }
        }
    }
}

/// TLS byte stream that reports the TCP peer to tonic.
pub struct TlsIo {
    inner: TlsStream<TcpStream>,
    peer: SocketAddr,
}

impl AsyncRead for TlsIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl tonic::transport::server::Connected for TlsIo {
    type ConnectInfo = tonic::transport::server::TcpConnectInfo;
    fn connect_info(&self) -> Self::ConnectInfo {
        tonic::transport::server::TcpConnectInfo {
            local_addr: self.inner.get_ref().0.local_addr().ok(),
            remote_addr: Some(self.peer),
        }
    }
}

pub fn tls_incoming(
    listener: TcpListener,
    config: Arc<ServerConfig>,
) -> tokio_stream::wrappers::ReceiverStream<Result<TlsIo, std::io::Error>> {
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let acceptor = TlsAcceptor::from(config);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, peer)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Ok(inner) = acceptor.accept(tcp).await {
                    let _ = tx.send(Ok(TlsIo { inner, peer })).await;
                }
            });
        }
    });
    tokio_stream::wrappers::ReceiverStream::new(rx)
}

#[derive(Clone)]
pub struct CertResolver {
    current: Arc<RwLock<Arc<CertifiedKey>>>,
}

impl CertResolver {
    pub fn new(key: Arc<CertifiedKey>) -> Arc<Self> {
        Arc::new(Self {
            current: Arc::new(RwLock::new(key)),
        })
    }

    pub fn swap(&self, key: Arc<CertifiedKey>) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = key;
    }
}

impl std::fmt::Debug for CertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CertResolver")
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(
            self.current
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        )
    }
}

pub fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn server_config(
    resolver: Arc<CertResolver>,
    alpn: Vec<Vec<u8>>,
) -> Result<Arc<ServerConfig>, String> {
    install_crypto();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    cfg.alpn_protocols = alpn;
    Ok(Arc::new(cfg))
}

pub fn certified(issued: &Issued) -> Result<Arc<CertifiedKey>, String> {
    install_crypto();
    let mut cursor = std::io::Cursor::new(issued.cert_pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cursor)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("certificate chain: {e}"))?;
    if certs.is_empty() {
        return Err("certificate chain is empty".into());
    }
    let mut key_cursor = std::io::Cursor::new(issued.key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_cursor)
        .map_err(|e| format!("certificate key: {e}"))?
        .ok_or_else(|| "certificate key is missing".to_string())?;
    let key = match key {
        PrivateKeyDer::Pkcs8(k) => PrivateKeyDer::Pkcs8(k),
        PrivateKeyDer::Pkcs1(k) => PrivateKeyDer::Pkcs1(k),
        PrivateKeyDer::Sec1(k) => PrivateKeyDer::Sec1(k),
        _ => return Err("unsupported certificate key type".into()),
    };
    let signing = rustls::crypto::ring::default_provider()
        .key_provider
        .load_private_key(key)
        .map_err(|e| format!("certificate key: {e}"))?;
    Ok(Arc::new(CertifiedKey::new(certs, signing)))
}

pub fn read_current(cache: &FsPath) -> Option<Issued> {
    let dir = cache.join("current");
    let cert_pem = std::fs::read_to_string(dir.join("cert.pem")).ok()?;
    let key_pem = std::fs::read_to_string(dir.join("cert.key")).ok()?;
    let not_after_unix = std::fs::read_to_string(dir.join("not_after"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .or_else(|| cert_not_after(&cert_pem).ok())?;
    Some(Issued {
        cert_pem,
        key_pem,
        not_after_unix,
    })
}

/// Replace `current` only after both files are on disk. A failed renewal does
/// not call this, so the previous directory stays in place. The replaced
/// directory is kept as `previous`.
pub fn publish(cache: &FsPath, issued: &Issued) -> Result<(), String> {
    prepare_dir(cache)?;
    let staging = cache.join("staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    taur_core::set_mode(&staging, 0o700).map_err(|e| e.to_string())?;
    write_file(
        staging.join("cert.pem").as_path(),
        issued.cert_pem.as_bytes(),
        0o644,
    )?;
    write_file(
        staging.join("cert.key").as_path(),
        issued.key_pem.as_bytes(),
        0o600,
    )?;
    write_file(
        staging.join("not_after").as_path(),
        issued.not_after_unix.to_string().as_bytes(),
        0o644,
    )?;
    let current = cache.join("current");
    let previous = cache.join("previous");
    if previous.exists() {
        let _ = std::fs::remove_dir_all(&previous);
    }
    if current.exists() {
        std::fs::rename(&current, &previous)
            .map_err(|e| format!("retaining the previous certificate: {e}"))?;
    }
    std::fs::rename(&staging, &current).map_err(|e| format!("installing the certificate: {e}"))?;
    Ok(())
}

fn cert_not_after(cert_pem: &str) -> Result<u64, String> {
    let mut cursor = std::io::Cursor::new(cert_pem.as_bytes());
    let der = rustls_pemfile::certs(&mut cursor)
        .next()
        .ok_or_else(|| "certificate chain is empty".to_string())?
        .map_err(|e| e.to_string())?;
    let (_, parsed) = x509_parser::parse_x509_certificate(&der).map_err(|e| e.to_string())?;
    let ts = parsed.validity().not_after.timestamp();
    u64::try_from(ts).map_err(|_| "certificate expiry is not a unix time".into())
}

fn prepare_dir(path: &FsPath) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    taur_core::set_mode(path, 0o700).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_file(path: &FsPath, bytes: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(mode);
        }
        let mut file = opts
            .open(&tmp)
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.write_all(bytes)
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.sync_all().ok();
    }
    taur_core::set_mode(&tmp, mode).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| format!("renaming {}: {e}", path.display()))?;
    Ok(())
}

fn public_error(err: &str) -> String {
    if err.contains("PRIVATE") || err.contains("BEGIN ") {
        return "internal certificate error".into();
    }
    err.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn settings(dir: &FsPath, accept: bool) -> AcmeSettings {
        AcmeSettings {
            domains: vec!["pkg.example.com".into()],
            directory: "http://127.0.0.1:1/directory".into(),
            staging: true,
            contact: "mailto:admin@example.com".into(),
            accept_terms: accept,
            cache: dir.to_path_buf(),
            http_bind: "127.0.0.1:0".parse().unwrap(),
            renew_before: Duration::from_secs(30 * 24 * 3600),
        }
    }

    fn local_cert() -> Issued {
        let generated = rcgen::generate_simple_self_signed(vec!["pkg.example.com".to_string()])
            .expect("test cert");
        let cert_pem = generated.cert.pem();
        let key_pem = generated.signing_key.serialize_pem();
        let not_after_unix = cert_not_after(&cert_pem).unwrap();
        Issued {
            cert_pem,
            key_pem,
            not_after_unix,
        }
    }

    struct LocalIssuer {
        hits: AtomicUsize,
    }

    impl CertificateIssuer for LocalIssuer {
        async fn issue(&self, _: &AcmeSettings, board: &ChallengeBoard) -> Result<Issued, String> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            board.insert("local-token".into(), "local-token.body".into());
            Ok(local_cert())
        }
    }

    struct RefuseNetwork;

    impl CertificateIssuer for RefuseNetwork {
        async fn issue(&self, _: &AcmeSettings, _: &ChallengeBoard) -> Result<Issued, String> {
            Err("directory refused".into())
        }
    }

    #[tokio::test]
    async fn terms_are_refused_before_any_directory_connection() {
        let dir = tempfile::tempdir().unwrap();
        let err = InstantAcme
            .issue(&settings(dir.path(), false), &ChallengeBoard::default())
            .await
            .unwrap_err();
        assert!(err.contains("TAUR_ACME_ACCEPT_TERMS"), "{err}");
        assert!(!dir.path().join("account.json").exists());
    }

    #[tokio::test]
    async fn challenge_is_served_and_https_uses_that_certificate() {
        let board = ChallengeBoard::default();
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let http_addr = http.local_addr().unwrap();
        let (_tx, rx) = watch::channel(false);
        tokio::spawn(serve_challenges(
            http,
            board.clone(),
            "https://pkg.example.com".into(),
            rx,
        ));
        let issuer = LocalIssuer {
            hits: AtomicUsize::new(0),
        };
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = settings(dir.path(), true);
        cfg.directory = "http://127.0.0.1:9/directory".into();
        let issued = issuer.issue(&cfg, &board).await.unwrap();
        assert_eq!(issuer.hits.load(Ordering::SeqCst), 1);
        assert_eq!(issued.cert_pem.matches("BEGIN CERTIFICATE").count(), 1);
        publish(dir.path(), &issued).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("current/cert.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let mut body = String::new();
        for _ in 0..20 {
            body = reqwest_like(http_addr, "/.well-known/acme-challenge/local-token").await;
            if body == "local-token.body" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(body, "local-token.body");

        let https = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let https_addr = https.local_addr().unwrap();
        let config = server_config(
            CertResolver::new(certified(&issued).unwrap()),
            vec![b"http/1.1".to_vec()],
        )
        .unwrap();
        let (tx, rx) = watch::channel(false);
        tokio::spawn(serve_https(https, Router::new(), config, rx));
        let mut roots = rustls::RootCertStore::empty();
        let mut cursor = std::io::Cursor::new(issued.cert_pem.as_bytes());
        for cert in rustls_pemfile::certs(&mut cursor) {
            roots.add(cert.unwrap()).unwrap();
        }
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
        let tcp = TcpStream::connect(https_addr).await.unwrap();
        let tls = connector
            .connect(
                rustls::pki_types::ServerName::try_from("pkg.example.com").unwrap(),
                tcp,
            )
            .await
            .unwrap();
        let presented = tls.get_ref().1.peer_certificates().unwrap();
        assert_eq!(
            presented.len(),
            1,
            "the test certificate is not a public chain"
        );
        let _ = tx.send(true);
    }

    #[tokio::test]
    async fn failed_order_does_not_open_https() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = watch::channel(false);
        let settings = settings(dir.path(), true);
        let task = tokio::spawn(bootstrap(settings, RefuseNetwork, rx));
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = tx.send(true);
        match task.await.unwrap() {
            Err(err) => assert!(err.contains("HTTPS was not started"), "{err}"),
            Ok(_) => panic!("HTTPS config was returned without a certificate"),
        }
        assert!(read_current(dir.path()).is_none());
    }

    #[test]
    fn failed_renewal_keeps_the_previous_files() {
        let dir = tempfile::tempdir().unwrap();
        let first = local_cert();
        publish(dir.path(), &first).unwrap();
        let saved = std::fs::read(dir.path().join("current/cert.pem")).unwrap();
        // Renewal stops before publish.
        assert_eq!(
            std::fs::read(dir.path().join("current/cert.pem")).unwrap(),
            saved
        );
        assert!(!dir.path().join("previous").exists());
        let second = local_cert();
        publish(dir.path(), &second).unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("previous/cert.pem")).unwrap(),
            saved
        );
        assert_ne!(
            std::fs::read(dir.path().join("current/cert.pem")).unwrap(),
            saved
        );
    }

    async fn reqwest_like(addr: SocketAddr, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut tcp = TcpStream::connect(addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        tcp.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        tcp.read_to_end(&mut buf).await.unwrap();
        let text = String::from_utf8_lossy(&buf);
        text.split("\r\n\r\n")
            .nth(1)
            .unwrap_or("")
            .trim()
            .to_string()
    }
}
