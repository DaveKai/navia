//! Subsonic token authentication: `t = md5(password + salt)` with a fresh salt per request.
//! Credentials are never placed in Debug output or error messages.

use std::fmt;

#[derive(Clone)]
pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self { username: username.into(), password: password.into() }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    /// Only for Navidrome's own login endpoint, which needs the real password.
    pub(crate) fn password(&self) -> &str {
        &self.password
    }

    /// The token and salt query parameters for one request.
    pub(crate) fn token(&self) -> (String, String) {
        let salt = format!("{:016x}", rand::random::<u64>());
        (format!("{:x}", md5::compute(format!("{}{}", self.password, salt))), salt)
    }

    /// A stable identifier for this account on this server. It never contains the password.
    pub fn account_key(&self, server_url: &str) -> String {
        format!("{:x}", md5::compute(format!("{}\0{}", server_url.trim_end_matches('/'), self.username)))
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials").field("username", &self.username).field("password", &"<redacted>").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_md5_of_password_plus_salt() {
        let c = Credentials::new("u", "sesame");
        let (token, salt) = c.token();
        assert_eq!(token, format!("{:x}", md5::compute(format!("sesame{salt}"))));
    }

    #[test]
    fn salts_differ_per_request() {
        let c = Credentials::new("u", "p");
        assert_ne!(c.token().1, c.token().1);
    }

    #[test]
    fn debug_hides_the_password() {
        let c = Credentials::new("u", "hunter2");
        assert!(!format!("{c:?}").contains("hunter2"));
    }

    #[test]
    fn account_key_ignores_trailing_slash_and_differs_by_user() {
        let a = Credentials::new("a", "x");
        let b = Credentials::new("b", "x");
        assert_eq!(a.account_key("http://s/"), a.account_key("http://s"));
        assert_ne!(a.account_key("http://s"), b.account_key("http://s"));
    }
}
