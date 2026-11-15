//! Navidrome's own REST API (`/api/...`), used for what Subsonic lacks: smart playlist rules,
//! playlist images and the dated scrobble history. Navidrome says this API may change without
//! notice, so everything here is optional. Logging in updates the user's last-login time.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::{Client, WritePolicy};
use crate::error::{Error, Result};

/// Scrobbles fetched per request.
const SCROBBLE_PAGE: usize = 1000;

/// From 0.55, `/api/album?artist_id=` matches album-artist and track-artist credits.
const CREDITED_ALBUMS_MIN_VERSION: (u32, u32, u32) = (0, 55, 0);
/// Albums per request when listing an artist's credited albums. Two in tests to exercise paging.
const ALBUM_PAGE: usize = if cfg!(test) { 2 } else { 500 };

/// Navidrome added `POST`/`DELETE /api/playlist/{id}/image` in 0.61.
const IMAGE_UPLOAD_MIN_VERSION: (u32, u32, u32) = (0, 61, 0);
/// Long enough for a large image on a slow connection.
const IMAGE_UPLOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// How long the public `/app/` page may take.
const PUBLIC_CONFIG_TIMEOUT: Duration = Duration::from_secs(10);
/// Files bigger than this aren't read. Only a sanity bound, the server enforces the real limit.
const IMAGE_MAX_READ: u64 = 256 * 1024 * 1024;
/// Navidrome's default `MaxImageUploadSize`, for a useful message when an upload is cut off.
const SERVER_DEFAULT_IMAGE_LIMIT: u64 = 10 * 1000 * 1000;

/// One play from the server's dated history, for the signed-in user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrobbleEntry {
    pub id: i64,
    pub song_id: String,
    /// Unix seconds.
    pub played_at: i64,
}

/// What one library holds, as `X-Total-Count` counts it with missing files left out. Sync
/// progress is measured against these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LibraryTotals {
    pub artists: u32,
    pub albums: u32,
    pub songs: u32,
}

/// What a server allows for playlist images, found without logging in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistImageSupport {
    /// The server has the upload and removal routes.
    pub supported: bool,
    /// `EnableArtworkUpload`, when the server said. `false` limits uploads to admins.
    pub uploads_enabled: Option<bool>,
    /// How the answer was reached.
    pub evidence: String,
}

impl PlaylistImageSupport {
    /// Whether to offer Upload and Remove to this user.
    pub fn allowed_for(&self, admin: Option<bool>) -> bool {
        self.supported && (admin == Some(true) || self.uploads_enabled != Some(false))
    }
}

/// Why a playlist image couldn't be changed. The messages are written for the user.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PlaylistImageError {
    #[error("Test and read-only runs can't change playlist images.")]
    WritesDisabled,
    #[error("Couldn't read that file: {0}")]
    File(String),
    #[error("That image is too large for this server.")]
    TooLarge,
    #[error("That file isn't an image the server can use. Try a JPEG, PNG, GIF or WebP.")]
    NotAnImage,
    #[error("This server doesn't allow uploading artwork.")]
    UploadsDisabled,
    #[error("You can't change this playlist's image.")]
    NotAllowed,
    #[error("This playlist no longer exists on the server.")]
    NotFound,
    #[error("The server is limiting sign-ins right now. Try again in a little while.")]
    Busy,
    #[error("Couldn't reach the server: {0}")]
    Connection(String),
    #[error("The server couldn't change the image: {0}")]
    Server(String),
}

/// Why smart playlist rules weren't saved. The messages are written for the user.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SmartSaveError {
    #[error("Test and read-only runs can't change playlists.")]
    WritesDisabled,
    #[error("{0}")]
    Invalid(String),
    #[error("This isn't a smart playlist.")]
    NotSmart,
    #[error("Only its owner can change this playlist.")]
    NotAllowed,
    #[error("This playlist no longer exists on the server.")]
    NotFound,
    #[error("This server can't save smart playlists.")]
    Unsupported,
    #[error("The server is limiting sign-ins right now. Try again in a little while.")]
    Busy,
    #[error("Couldn't reach the server: {0}")]
    Connection(String),
    #[error("The server couldn't save the playlist: {0}")]
    Server(String),
}

/// A smart playlist the server saved: its id and the rules as it stored them (JSON text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartSaved {
    pub id: String,
    pub rules: Option<String>,
}

/// Native API state kept per client: the token, and whether credited albums are listed.
#[derive(Default)]
pub(crate) struct NativeSession {
    token: Option<String>,
    credited_albums: Option<bool>,
    navidrome: Option<bool>,
}

impl Client {
    /// The kept native token, or a new one from a login. Each reply's `x-nd-authorization`
    /// header refreshes it.
    async fn session_token(&self) -> Result<String> {
        if let Some(token) = self.inner.native.lock().unwrap().token.clone() {
            return Ok(token);
        }
        let token = self.native_token(&self.inner.http).await?;
        self.inner.native.lock().unwrap().token = Some(token.clone());
        Ok(token)
    }

    fn keep_refreshed_token(&self, response: &reqwest::Response) {
        let refreshed = response.headers().get("x-nd-authorization").and_then(|v| v.to_str().ok()).map(|v| v.trim_start_matches("Bearer ").trim().to_string());
        if let Some(token) = refreshed.filter(|t| !t.is_empty()) {
            self.inner.native.lock().unwrap().token = Some(token);
        }
    }

    /// A native GET with the kept token. On a 401 it logs in once more and retries.
    async fn native_get(&self, url: &str, endpoint: &str) -> Result<reqwest::Response> {
        for attempt in 0..2 {
            let token = self.session_token().await?;
            let response = self.inner.http.get(url).header("x-nd-authorization", format!("Bearer {token}")).send().await?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.inner.native.lock().unwrap().token = None;
                continue;
            }
            if !status.is_success() {
                return Err(Error::HttpStatus { status: status.as_u16(), endpoint: endpoint.to_string() });
            }
            self.keep_refreshed_token(&response);
            return Ok(response);
        }
        Err(Error::HttpStatus { status: 401, endpoint: endpoint.to_string() })
    }

    /// Whether this server lists the albums an artist is credited on (Navidrome 0.55+).
    /// Asks `ping` once and doesn't log in.
    pub async fn lists_credited_albums(&self) -> Result<bool> {
        if let Some(known) = self.inner.native.lock().unwrap().credited_albums {
            return Ok(known);
        }
        let info = self.ping().await?;
        let navidrome = info.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("navidrome"));
        let known = navidrome && info.server_version.as_deref().and_then(parse_version).is_some_and(|v| v >= CREDITED_ALBUMS_MIN_VERSION);
        self.inner.native.lock().unwrap().credited_albums = Some(known);
        Ok(known)
    }

    /// The ids of every non-missing album an artist is credited on, from `/api/album?artist_id=`.
    /// Errors on servers without it, see [`Self::lists_credited_albums`].
    pub async fn credited_album_ids(&self, artist_id: &str) -> Result<Vec<String>> {
        if !plain_id(artist_id) {
            return Err(Error::Decode { endpoint: "api/album".to_string(), detail: "not an artist id".to_string() });
        }
        if !self.lists_credited_albums().await? {
            return Err(Error::HttpStatus { status: 404, endpoint: "api/album".to_string() });
        }
        let base = self.config().base_url.trim_end_matches('/').to_string();
        let mut ids = Vec::new();
        let mut start = 0;
        loop {
            let end = start + ALBUM_PAGE;
            let url = format!("{base}/api/album?artist_id={artist_id}&missing=false&_sort=name&_order=ASC&_start={start}&_end={end}");
            let response = self.native_get(&url, "api/album").await?;
            let total = response.headers().get("x-total-count").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<usize>().ok());
            let value: Value = response.json().await?;
            let rows = value.as_array().ok_or_else(|| Error::Decode { endpoint: "api/album".to_string(), detail: "the reply isn't a list".to_string() })?;
            let page: Vec<String> = rows.iter().filter_map(|r| r.get("id").and_then(Value::as_str)).map(str::to_string).collect();
            if page.len() != rows.len() {
                return Err(Error::Decode { endpoint: "api/album".to_string(), detail: "an album has no id".to_string() });
            }
            ids.extend(page);
            let done = rows.len() < ALBUM_PAGE || total.is_some_and(|t| ids.len() >= t);
            if done {
                break;
            }
            start = end;
        }
        Ok(ids)
    }

    /// Artist, album and song counts for one library, or all the user sees, from `X-Total-Count`.
    /// Navidrome only. A reply without the header is an error, never a guess.
    pub async fn library_totals(&self, library_id: Option<&str>) -> Result<LibraryTotals> {
        if library_id.is_some_and(|id| !plain_id(id)) {
            return Err(Error::Decode { endpoint: "api".to_string(), detail: "not a library id".to_string() });
        }
        if !self.is_navidrome().await? {
            return Err(Error::HttpStatus { status: 404, endpoint: "api".to_string() });
        }
        let base = self.config().base_url.trim_end_matches('/').to_string();
        let library = library_id.map(|id| format!("&library_id={id}")).unwrap_or_default();
        let mut counts = [0u32; 3];
        for (count, resource) in counts.iter_mut().zip(["artist", "album", "song"]) {
            let endpoint = format!("api/{resource}");
            let url = format!("{base}/{endpoint}?_start=0&_end=1&missing=false{library}");
            let response = self.native_get(&url, &endpoint).await?;
            let total = response.headers().get("x-total-count").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
            *count = total.ok_or_else(|| Error::Decode { endpoint: endpoint.clone(), detail: "no X-Total-Count".to_string() })?.min(u32::MAX as u64) as u32;
        }
        Ok(LibraryTotals { artists: counts[0], albums: counts[1], songs: counts[2] })
    }

    /// Whether `ping` says this is Navidrome, asked once per client.
    async fn is_navidrome(&self) -> Result<bool> {
        if let Some(known) = self.inner.native.lock().unwrap().navidrome {
            return Ok(known);
        }
        let info = self.ping().await?;
        let known = info.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("navidrome"));
        self.inner.native.lock().unwrap().navidrome = Some(known);
        Ok(known)
    }

    async fn native_token(&self, http: &reqwest::Client) -> Result<String> {
        let config = self.config();
        let url = format!("{}/auth/login", config.base_url.trim_end_matches('/'));
        let body = json!({"username": config.credentials.username(), "password": config.credentials.password()});
        let response = http.post(url).json(&body).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus { status: status.as_u16(), endpoint: "auth/login".to_string() });
        }
        let value: Value = response.json().await?;
        value.get("token").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::Decode { endpoint: "auth/login".to_string(), detail: "no token in the reply".to_string() })
    }

    /// A smart playlist's rules as JSON text. `None` when it has none or the API can't say.
    pub async fn playlist_rules(&self, id: &str) -> Result<Option<String>> {
        let http = reqwest::Client::builder().timeout(self.config().timeout).build()?;
        let token = self.native_token(&http).await?;
        let url = format!("{}/api/playlist/{id}", self.config().base_url.trim_end_matches('/'));
        let response = http.get(url).header("x-nd-authorization", format!("Bearer {token}")).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::HttpStatus { status: status.as_u16(), endpoint: "api/playlist".to_string() });
        }
        let value: Value = response.json().await?;
        Ok(value.get("rules").filter(|r| !r.is_null()).map(Value::to_string))
    }

    /// Saves a smart playlist with `PUT /api/playlist/{id}` or `POST /api/playlist`.
    /// An existing playlist is only changed if it already has rules and the user owns it or is
    /// an admin, so a regular playlist never becomes a smart one. `criteria` must have passed
    /// [`crate::smart_rules::checked_criteria`].
    pub async fn save_smart_playlist(&self, id: Option<&str>, name: &str, criteria: &Value, admin: Option<bool>) -> std::result::Result<SmartSaved, SmartSaveError> {
        if self.write_policy() == WritePolicy::ReadOnly {
            return Err(SmartSaveError::WritesDisabled);
        }
        if id.is_some_and(|i| !plain_id(i)) {
            return Err(SmartSaveError::NotFound);
        }
        let base = self.config().base_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder().timeout(self.config().timeout).build().map_err(|e| SmartSaveError::Connection(e.to_string()))?;
        let token = self.native_token(&http).await.map_err(|e| match e {
            Error::HttpStatus { status: 429, .. } => SmartSaveError::Busy,
            Error::HttpStatus { status: 404 | 405, .. } => SmartSaveError::Unsupported,
            Error::HttpStatus { status, .. } => SmartSaveError::Server(format!("the sign-in was refused (HTTP {status})")),
            other => SmartSaveError::Connection(other.to_string()),
        })?;
        let auth = format!("Bearer {token}");
        let read = |id: String| {
            let (http, auth, url) = (http.clone(), auth.clone(), format!("{base}/api/playlist/{id}"));
            async move {
                let response = http.get(url).header("x-nd-authorization", auth).send().await.map_err(|e| SmartSaveError::Connection(e.to_string()))?;
                smart_reply(response, false).await
            }
        };
        let body = serde_json::json!({"name": name, "rules": criteria});
        let id = match id {
            Some(id) => {
                let current = read(id.to_string()).await?;
                if current.get("rules").is_none_or(Value::is_null) {
                    return Err(SmartSaveError::NotSmart);
                }
                let owner = current.get("ownerName").and_then(Value::as_str).unwrap_or_default();
                if !owner.is_empty() && !owner.eq_ignore_ascii_case(self.config().credentials.username()) && admin != Some(true) {
                    return Err(SmartSaveError::NotAllowed);
                }
                let response = http.put(format!("{base}/api/playlist/{id}")).header("x-nd-authorization", &auth).json(&body).send().await.map_err(|e| SmartSaveError::Connection(e.to_string()))?;
                let saved = smart_reply(response, false).await?;
                // The reply is usually the stored playlist, so only re-read when it lacks the rules.
                if let Some(rules) = saved.get("rules").filter(|r| !r.is_null()) {
                    return Ok(SmartSaved { id: id.to_string(), rules: Some(rules.to_string()) });
                }
                id.to_string()
            }
            None => {
                let response = http.post(format!("{base}/api/playlist")).header("x-nd-authorization", &auth).json(&body).send().await.map_err(|e| SmartSaveError::Connection(e.to_string()))?;
                let created = smart_reply(response, true).await?;
                created.get("id").and_then(Value::as_str).filter(|i| plain_id(i)).map(str::to_string).ok_or_else(|| SmartSaveError::Server("the reply had no playlist id".to_string()))?
            }
        };
        // Reading the rules back is only a check. The save stands without it.
        let rules = read(id.clone()).await.ok().and_then(|p| p.get("rules").filter(|r| !r.is_null()).map(Value::to_string));
        Ok(SmartSaved { id, rules })
    }

    /// The `window.__APP_CONFIG__` object from the server's public `/app/` page. Sends no
    /// credentials. `None` when there is none, for example behind a proxy or on another server.
    pub async fn public_config(&self) -> Option<Arc<Value>> {
        if let Some(known) = self.public_config_slot().lock().unwrap().clone() {
            return known;
        }
        let http = reqwest::Client::builder().timeout(PUBLIC_CONFIG_TIMEOUT).user_agent(super::user_agent()).build().ok()?;
        let url = format!("{}/app/", self.config().base_url.trim_end_matches('/'));
        let response = http.get(url).send().await.ok()?;
        let status = response.status();
        // Server errors may be temporary, so those are asked again.
        if status.is_server_error() {
            return None;
        }
        let config = if status.is_success() { parse_app_config(&response.text().await.ok()?).map(Arc::new) } else { None };
        *self.public_config_slot().lock().unwrap() = Some(config.clone());
        config
    }

    /// What this server allows for playlist images, from its public page or else its version.
    pub async fn playlist_image_support(&self) -> Result<PlaylistImageSupport> {
        if let Some(config) = self.public_config().await {
            let flag = config.get("enableArtworkUpload").or_else(|| config.get("enableCoverArtUpload")).and_then(Value::as_bool);
            if flag.is_some() {
                return Ok(PlaylistImageSupport { supported: true, uploads_enabled: flag, evidence: "the server's public configuration has the upload setting".to_string() });
            }
            if let Some(version) = config.get("version").and_then(Value::as_str) {
                return Ok(support_from_version(version));
            }
        }
        // No readable page. The version still says whether the route exists.
        let info = self.ping().await?;
        let navidrome = info.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("navidrome"));
        match (navidrome, info.server_version.as_deref()) {
            (true, Some(version)) => Ok(support_from_version(version)),
            _ => Ok(PlaylistImageSupport { supported: false, uploads_enabled: None, evidence: "not a Navidrome server with a known version".to_string() }),
        }
    }

    /// Uploads `bytes` as the playlist's image. Only for an explicit user action. Logs in.
    pub async fn set_playlist_image(&self, id: &str, bytes: Vec<u8>) -> std::result::Result<(), PlaylistImageError> {
        let url = self.playlist_image_url(id)?;
        let http = reqwest::Client::builder().timeout(IMAGE_UPLOAD_TIMEOUT).build().map_err(|e| PlaylistImageError::Connection(e.to_string()))?;
        let token = self.image_token(&http).await?;
        let size = bytes.len() as u64;
        let boundary = format!("navia{:x}", md5::compute(&bytes));
        let response = http
            .post(url)
            .header("x-nd-authorization", format!("Bearer {token}"))
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(multipart_body(&boundary, &bytes))
            .send()
            .await
            .map_err(|e| if size > SERVER_DEFAULT_IMAGE_LIMIT { PlaylistImageError::TooLarge } else { PlaylistImageError::Connection(e.to_string()) })?;
        image_result(response).await
    }

    /// Removes an uploaded image so the server's own cover shows again. Logs in.
    pub async fn remove_playlist_image(&self, id: &str) -> std::result::Result<(), PlaylistImageError> {
        let url = self.playlist_image_url(id)?;
        let http = reqwest::Client::builder().timeout(self.config().timeout).build().map_err(|e| PlaylistImageError::Connection(e.to_string()))?;
        let token = self.image_token(&http).await?;
        let response = http.delete(url).header("x-nd-authorization", format!("Bearer {token}")).send().await.map_err(|e| PlaylistImageError::Connection(e.to_string()))?;
        image_result(response).await
    }

    /// The route for one playlist's image. The write gate is checked first, before any login.
    fn playlist_image_url(&self, id: &str) -> std::result::Result<String, PlaylistImageError> {
        if self.write_policy() == WritePolicy::ReadOnly {
            return Err(PlaylistImageError::WritesDisabled);
        }
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) {
            return Err(PlaylistImageError::NotFound);
        }
        Ok(format!("{}/api/playlist/{id}/image", self.config().base_url.trim_end_matches('/')))
    }

    async fn image_token(&self, http: &reqwest::Client) -> std::result::Result<String, PlaylistImageError> {
        self.native_token(http).await.map_err(|e| match e {
            Error::HttpStatus { status: 401 | 403, .. } => PlaylistImageError::NotAllowed,
            Error::HttpStatus { status: 429, .. } => PlaylistImageError::Busy,
            other => PlaylistImageError::Connection(other.to_string()),
        })
    }

    /// A copy of this client that may write. Only for tests that change their own playlist.
    pub fn writable_copy(&self) -> Result<Client> {
        let mut config = self.config().clone();
        config.write_policy = WritePolicy::Allowed;
        Client::new(config)
    }

    /// Whether the server has the scrobble history route (Navidrome 0.64+). Asked without
    /// credentials, since the route answers 401 when it exists.
    pub async fn has_scrobble_history(&self) -> Result<bool> {
        let http = reqwest::Client::builder().timeout(self.config().timeout).build()?;
        let url = format!("{}/api/scrobble?_start=0&_end=1", self.config().base_url.trim_end_matches('/'));
        Ok(http.get(url).send().await?.status() == reqwest::StatusCode::UNAUTHORIZED)
    }

    /// The user's plays since `from` (Unix seconds, inclusive), newest first, at most `limit`.
    /// Empty when `EnableScrobbleHistory` is off. An unknown reply shape is an error.
    pub async fn scrobbles_since(&self, from: i64, limit: usize) -> Result<Vec<ScrobbleEntry>> {
        let http = reqwest::Client::builder().timeout(self.config().timeout).build()?;
        let token = self.native_token(&http).await?;
        let base = self.config().base_url.trim_end_matches('/').to_string();
        let mut out = Vec::new();
        let mut start = 0;
        while out.len() < limit {
            let end = start + SCROBBLE_PAGE;
            let url = format!("{base}/api/scrobble?_start={start}&_end={end}&_sort=submission_time&_order=DESC&from={from}");
            let response = http.get(url).header("x-nd-authorization", format!("Bearer {token}")).send().await?;
            let status = response.status();
            if !status.is_success() {
                return Err(Error::HttpStatus { status: status.as_u16(), endpoint: "api/scrobble".to_string() });
            }
            let value: Value = response.json().await?;
            let rows = value.as_array().ok_or_else(|| scrobble_shape("the reply isn't a list"))?;
            let page: Vec<ScrobbleEntry> = rows.iter().filter_map(scrobble_entry).collect();
            if page.is_empty() && !rows.is_empty() {
                return Err(scrobble_shape("no row has the expected fields"));
            }
            let older = page.last().is_some_and(|p| p.played_at < from);
            out.extend(page.into_iter().filter(|p| p.played_at >= from));
            if rows.len() < SCROBBLE_PAGE || older {
                break;
            }
            start = end;
        }
        out.truncate(limit);
        Ok(out)
    }
}

/// An id that can go in a path as it is. Navidrome's are short alphanumeric strings.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A native playlist reply as JSON. `creating` tells a missing route from a missing playlist.
async fn smart_reply(response: reqwest::Response, creating: bool) -> std::result::Result<Value, SmartSaveError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return response.json().await.map_err(|e| SmartSaveError::Server(e.to_string()));
    }
    let text = response.text().await.unwrap_or_default();
    Err(smart_error(status, text.trim(), creating))
}

/// Maps REST controller replies: 403 another user's, 404 missing, 422 unparsable rules,
/// 400 validation error.
fn smart_error(status: u16, text: &str, creating: bool) -> SmartSaveError {
    // Error bodies are `{"error": "..."}`. Keep the server's words when present.
    let said = serde_json::from_str::<Value>(text).ok().and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| text.to_string());
    match status {
        403 => SmartSaveError::NotAllowed,
        404 | 405 if creating => SmartSaveError::Unsupported,
        404 => SmartSaveError::NotFound,
        429 => SmartSaveError::Busy,
        400 | 422 => SmartSaveError::Invalid(if said.is_empty() { "The server didn't accept these rules.".to_string() } else { format!("The server didn't accept these rules: {said}") }),
        _ if said.is_empty() => SmartSaveError::Server(format!("HTTP {status}")),
        _ => SmartSaveError::Server(said),
    }
}

/// Reads an image for upload.
pub fn read_image_file(path: &Path) -> std::result::Result<Vec<u8>, PlaylistImageError> {
    let meta = std::fs::metadata(path).map_err(|e| PlaylistImageError::File(e.to_string()))?;
    if !meta.is_file() {
        return Err(PlaylistImageError::File("not a file".to_string()));
    }
    if meta.len() > IMAGE_MAX_READ {
        return Err(PlaylistImageError::TooLarge);
    }
    std::fs::read(path).map_err(|e| PlaylistImageError::File(e.to_string()))
}

/// One multipart file part named `image`, as Navidrome's web UI sends. The server sniffs the format.
fn multipart_body(boundary: &str, bytes: &[u8]) -> Vec<u8> {
    let head = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"cover\"\r\nContent-Type: application/octet-stream\r\n\r\n");
    let tail = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(head.len() + bytes.len() + tail.len());
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(tail.as_bytes());
    body
}

async fn image_result(response: reqwest::Response) -> std::result::Result<(), PlaylistImageError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    Err(image_error(status, text.trim()))
}

/// Maps the upload handler's replies. Unknown texts keep the server's own words.
fn image_error(status: u16, text: &str) -> PlaylistImageError {
    let lower = text.to_ascii_lowercase();
    match status {
        400 if lower.contains("too large") => PlaylistImageError::TooLarge,
        400 if lower.contains("invalid image") || lower.contains("image type") => PlaylistImageError::NotAnImage,
        413 => PlaylistImageError::TooLarge,
        403 if lower.contains("disabled") => PlaylistImageError::UploadsDisabled,
        401 | 403 => PlaylistImageError::NotAllowed,
        429 => PlaylistImageError::Busy,
        404 => PlaylistImageError::NotFound,
        _ if text.is_empty() => PlaylistImageError::Server(format!("HTTP {status}")),
        _ => PlaylistImageError::Server(text.to_string()),
    }
}

/// The `window.__APP_CONFIG__` object, which may be a JSON object or a JSON string holding one.
fn parse_app_config(page: &str) -> Option<Value> {
    let after = &page[page.find("__APP_CONFIG__")? + "__APP_CONFIG__".len()..];
    let start = after.find('=')? + 1;
    let value = serde_json::Deserializer::from_str(after[start..].trim_start()).into_iter::<Value>().next()?.ok()?;
    match value {
        Value::String(inner) => serde_json::from_str(&inner).ok(),
        object @ Value::Object(_) => Some(object),
        _ => None,
    }
}

fn support_from_version(version: &str) -> PlaylistImageSupport {
    match parse_version(version) {
        Some(found) => {
            PlaylistImageSupport { supported: found >= IMAGE_UPLOAD_MIN_VERSION, uploads_enabled: None, evidence: format!("server version {}.{}.{}", found.0, found.1, found.2) }
        }
        None => PlaylistImageSupport { supported: false, uploads_enabled: None, evidence: format!("can't read server version {version:?}") },
    }
}

/// The leading `major.minor.patch` of a version such as `0.64.2 (10114574)`.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(str::parse::<u32>);
    let major = parts.next()?.ok()?;
    let minor = parts.next()?.ok()?;
    let patch = parts.next().and_then(|p| p.ok()).unwrap_or(0);
    Some((major, minor, patch))
}

fn scrobble_entry(row: &Value) -> Option<ScrobbleEntry> {
    Some(ScrobbleEntry { id: row.get("id")?.as_i64()?, song_id: row.get("mediaFileId")?.as_str()?.to_string(), played_at: row.get("submissionTime")?.as_i64()? })
}

fn scrobble_shape(detail: &str) -> Error {
    Error::Decode { endpoint: "api/scrobble".to_string(), detail: detail.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_config_navidrome_embeds_in_its_page() {
        let inner = serde_json::to_string(&serde_json::json!({"enableArtworkUpload": false, "version": "0.64.2 (10114574)"})).unwrap();
        let page = format!("<script>window.__APP_CONFIG__ = {};</script>", serde_json::Value::String(inner));
        let config = parse_app_config(&page).unwrap();
        assert_eq!(config["enableArtworkUpload"], false);
        assert!(parse_app_config("<html>not navidrome</html>").is_none());
    }

    #[test]
    fn version_gate_is_0_61() {
        assert!(support_from_version("0.61.0 (abc)").supported);
        assert!(support_from_version("0.64.2 (10114574)").supported);
        assert!(!support_from_version("0.60.3 (abc)").supported);
        assert!(!support_from_version("dev").supported);
    }

    #[test]
    fn upload_setting_only_limits_non_admins() {
        let off = PlaylistImageSupport { supported: true, uploads_enabled: Some(false), evidence: String::new() };
        assert!(!off.allowed_for(Some(false)));
        assert!(!off.allowed_for(None));
        assert!(off.allowed_for(Some(true)));
        let unknown = PlaylistImageSupport { supported: true, uploads_enabled: None, evidence: String::new() };
        assert!(unknown.allowed_for(None));
        let old = PlaylistImageSupport { supported: false, uploads_enabled: Some(true), evidence: String::new() };
        assert!(!old.allowed_for(Some(true)));
    }

    #[test]
    fn server_replies_become_plain_errors() {
        assert_eq!(image_error(400, "file too large or invalid form"), PlaylistImageError::TooLarge);
        assert_eq!(image_error(400, "invalid image file"), PlaylistImageError::NotAnImage);
        assert_eq!(image_error(403, "artwork upload is disabled"), PlaylistImageError::UploadsDisabled);
        assert_eq!(image_error(403, "not authorized"), PlaylistImageError::NotAllowed);
        assert_eq!(image_error(404, "not found"), PlaylistImageError::NotFound);
        assert_eq!(image_error(500, "disk full"), PlaylistImageError::Server("disk full".to_string()));
    }

    #[test]
    fn smart_replies_become_plain_errors() {
        assert_eq!(smart_error(403, r#"{"error":"Updating playlist: Permission denied"}"#, false), SmartSaveError::NotAllowed);
        assert_eq!(smart_error(404, "", false), SmartSaveError::NotFound);
        assert_eq!(smart_error(404, "", true), SmartSaveError::Unsupported);
        assert_eq!(smart_error(422, r#"{"error":"Invalid request payload"}"#, false), SmartSaveError::Invalid("The server didn't accept these rules: Invalid request payload".to_string()));
        assert_eq!(smart_error(500, "", false), SmartSaveError::Server("HTTP 500".to_string()));
        assert!(!plain_id("../x") && plain_id("a1-b_2"));
    }

    #[test]
    fn form_has_one_file_part_named_image() {
        let body = multipart_body("b", &[1, 2, 3]);
        assert!(body.starts_with(b"--b\r\nContent-Disposition: form-data; name=\"image\""));
        assert!(body.windows(3).any(|w| w == [1, 2, 3]));
        assert!(body.ends_with(b"\r\n--b--\r\n"));
    }
}
