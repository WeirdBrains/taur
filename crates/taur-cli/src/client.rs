//! Thin HTTP client for the TAUR API. Never logs the token.

use crate::CliError;
use serde_json::Value;

pub struct Client {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

impl Client {
    pub fn new(base: &str, token: Option<String>) -> Client {
        Client {
            base: base.trim_end_matches('/').to_string(),
            token,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .build(),
        }
    }

    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        auth: bool,
    ) -> Result<Value, CliError> {
        let mut req = self.agent.request(method, &format!("{}{path}", self.base));
        if auth {
            let t = self.token.as_deref().ok_or(CliError::NotLoggedIn)?;
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let res = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        let resp = match res {
            Ok(r) => r,
            Err(ureq::Error::Status(code, r)) => {
                let msg = r
                    .into_json::<Value>()
                    .ok()
                    .and_then(|v| v["error"].as_str().map(String::from))
                    .unwrap_or_else(|| "request failed".into());
                return Err(CliError::Api(code, msg));
            }
            Err(e) => return Err(CliError::Transport(e.kind().to_string())),
        };
        let text = resp
            .into_string()
            .map_err(|e| CliError::Transport(e.to_string()))?;
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|e| CliError::Transport(format!("bad response: {e}")))
    }

    /// Generic authenticated request (path segments must already be encoded).
    pub fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, CliError> {
        self.call(method, path, body, true)
    }

    /// Generic anonymous GET.
    pub fn public_get(&self, path: &str) -> Result<Value, CliError> {
        self.call("GET", path, None, false)
    }

    pub fn enc(s: &str) -> String {
        enc(s)
    }

    fn get(&self, path: &str) -> Result<Value, CliError> {
        self.call("GET", path, None, true)
    }

    // accounts
    pub fn register(&self, name: &str, email: &str, password: &str) -> Result<Value, CliError> {
        let b = serde_json::json!({"name": name, "email": email, "password": password});
        self.call("POST", "/api/v1/register", Some(b), false)
    }
    /// Returns the session token (callers store it, never print it).
    pub fn login(&self, name: &str, password: &str) -> Result<String, CliError> {
        let b = serde_json::json!({"name": name, "password": password});
        let v = self.call("POST", "/api/v1/login", Some(b), false)?;
        v["token"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| CliError::Transport("login response has no token".into()))
    }
    pub fn logout(&self) -> Result<(), CliError> {
        self.call("POST", "/api/v1/logout", None, true).map(drop)
    }
    pub fn whoami(&self) -> Result<Value, CliError> {
        self.get("/api/v1/account")
    }
    pub fn unregister(&self, password: &str) -> Result<(), CliError> {
        let b = serde_json::json!({"password": password});
        self.call("DELETE", "/api/v1/account", Some(b), true)
            .map(drop)
    }

    pub fn verify(&self, token: &str) -> Result<Value, CliError> {
        self.call(
            "POST",
            "/api/v1/verify",
            Some(serde_json::json!({"token": token})),
            false,
        )
    }
    pub fn resend_verification(&self) -> Result<(), CliError> {
        self.call("POST", "/api/v1/verify/resend", None, true)
            .map(drop)
    }
    pub fn reset_request(&self, email: &str) -> Result<(), CliError> {
        let b = serde_json::json!({"email": email});
        self.call("POST", "/api/v1/password-reset", Some(b), false)
            .map(drop)
    }
    pub fn reset_confirm(&self, token: &str, password: &str) -> Result<(), CliError> {
        let b = serde_json::json!({"token": token, "password": password});
        self.call("POST", "/api/v1/password-reset/confirm", Some(b), false)
            .map(drop)
    }

    // RPC
    pub fn search(&self, term: &str, by: &str) -> Result<Value, CliError> {
        let path = format!("/rpc/v5/search/{}?by={}", enc(term), enc(by));
        rpc_ok(self.call("GET", &path, None, false)?)
    }
    pub fn info(&self, names: &[String]) -> Result<Value, CliError> {
        let q: Vec<String> = names.iter().map(|n| format!("arg[]={}", enc(n))).collect();
        rpc_ok(self.call("GET", &format!("/rpc/v5/info?{}", q.join("&")), None, false)?)
    }

    // orgs
    pub fn orgs(&self) -> Result<Value, CliError> {
        self.get("/api/v1/orgs")
    }
    pub fn org_create(&self, name: &str) -> Result<Value, CliError> {
        self.call(
            "POST",
            "/api/v1/orgs",
            Some(serde_json::json!({"name": name})),
            true,
        )
    }
    pub fn org_show(&self, name: &str) -> Result<Value, CliError> {
        self.get(&format!("/api/v1/orgs/{}", enc(name)))
    }
    pub fn org_delete(&self, name: &str) -> Result<(), CliError> {
        self.call("DELETE", &format!("/api/v1/orgs/{}", enc(name)), None, true)
            .map(drop)
    }
    pub fn set_member(&self, org: &str, user: &str, role: &str) -> Result<(), CliError> {
        let path = format!("/api/v1/orgs/{}/members/{}", enc(org), enc(user));
        self.call("PUT", &path, Some(serde_json::json!({"role": role})), true)
            .map(drop)
    }
    pub fn remove_member(&self, org: &str, user: &str) -> Result<(), CliError> {
        let path = format!("/api/v1/orgs/{}/members/{}", enc(org), enc(user));
        self.call("DELETE", &path, None, true).map(drop)
    }

    // ssh keys
    pub fn keys(&self) -> Result<Value, CliError> {
        self.get("/api/v1/ssh-keys")
    }
    pub fn key_add(&self, public_key: &str) -> Result<Value, CliError> {
        let b = serde_json::json!({"public_key": public_key});
        self.call("POST", "/api/v1/ssh-keys", Some(b), true)
    }
    pub fn key_remove(&self, id: i64) -> Result<(), CliError> {
        self.call("DELETE", &format!("/api/v1/ssh-keys/{id}"), None, true)
            .map(drop)
    }

    // packages
    pub fn my_packages(&self) -> Result<Value, CliError> {
        self.get("/api/v1/packages")
    }
    pub fn pkg_action(
        &self,
        pkgbase: &str,
        action: &str,
        body: Option<Value>,
    ) -> Result<(), CliError> {
        let path = format!("/api/v1/packages/{}/{action}", enc(pkgbase));
        self.call("POST", &path, Some(body.unwrap_or(Value::Null)), true)
            .map(drop)
    }
}

fn rpc_ok(v: Value) -> Result<Value, CliError> {
    match v["error"].as_str() {
        Some(e) => Err(CliError::Api(400, e.to_string())),
        None => Ok(v),
    }
}
