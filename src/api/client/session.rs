//! Session upkeep: device fingerprint cookies, cookie refresh and logout.

use super::{ApiClient, ApiResponse, BilibiliApiDomain};
use crate::storage::Credentials;
use anyhow::{Context, Result, anyhow};
use reqwest::header::COOKIE;
use serde::Deserialize;

/// Public key used to encrypt the `correspond` path of the cookie refresh flow.
const CORRESPOND_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDLgd2OAkcGVtoE3ThUREbio0Eg
Uc/prcajMKXvkCKFCWhJYJcLkcM2DKKcSeFpD/j6Boy538YXnR6VhcuUJOhH2x71
nzPjfdTcqMz7djHum0qSZA0AyCBDABUqCrfNgCiJ00Ra7GmRj+YCK1NJEuewlb40
JNrRuoEUXpabUzGB8QIDAQAB
-----END PUBLIC KEY-----";

/// HMAC key the web client uses to sign `GenWebTicket` requests.
const BILI_TICKET_KEY: &[u8] = b"XgwSnGZ1p";
/// Refetch the ticket this long before it expires.
const BILI_TICKET_MARGIN_SECS: i64 = 60 * 60;

pub(super) fn account_cookie(credentials: &Credentials) -> String {
    format!(
        "SESSDATA={}; bili_jct={}; DedeUserID={}",
        credentials.sessdata, credentials.bili_jct, credentials.dede_user_id
    )
}

#[derive(Debug, Default, Clone)]
pub(super) struct DeviceCookies {
    buvid3: Option<String>,
    buvid4: Option<String>,
    b_nut: Option<i64>,
    /// `bili_ticket` and its expiry as a Unix timestamp.
    bili_ticket: Option<(String, i64)>,
}

impl DeviceCookies {
    pub(super) fn header(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(buvid3) = &self.buvid3 {
            parts.push(format!("buvid3={buvid3}"));
        }
        if let Some(buvid4) = &self.buvid4 {
            parts.push(format!("buvid4={buvid4}"));
        }
        if let Some(b_nut) = self.b_nut {
            parts.push(format!("b_nut={b_nut}"));
        }
        if let Some((ticket, expires)) = &self.bili_ticket {
            parts.push(format!("bili_ticket={ticket}"));
            parts.push(format!("bili_ticket_expires={expires}"));
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }

    fn ticket_is_fresh(&self, now: i64) -> bool {
        self.bili_ticket
            .as_ref()
            .is_some_and(|(_, expires)| *expires - now > BILI_TICKET_MARGIN_SECS)
    }
}

/// Result of `/x/passport-login/web/cookie/info`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct CookieRefreshInfo {
    #[serde(default)]
    pub refresh: bool,
    /// Milliseconds; part of the encrypted `correspond` path.
    #[serde(default)]
    pub timestamp: i64,
}

#[derive(Deserialize)]
struct SpiData {
    b_3: String,
    #[serde(default)]
    b_4: Option<String>,
}

#[derive(Deserialize)]
struct TicketData {
    ticket: String,
    #[serde(default, deserialize_with = "crate::api::serde_util::lenient_i64")]
    created_at: i64,
    #[serde(default, deserialize_with = "crate::api::serde_util::lenient_i64")]
    ttl: i64,
}

#[derive(Deserialize)]
struct CookieRefreshData {
    refresh_token: String,
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn hmac_sha256_hex(key: &[u8], message: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(message.as_bytes());
    hex_lower(&mac.finalize().into_bytes())
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `refresh_{timestamp}` encrypted with RSA-OAEP/SHA-256, as lowercase hex.
fn correspond_path(timestamp_ms: i64) -> Result<String> {
    use rsa::{Oaep, RsaPublicKey, pkcs8::DecodePublicKey};
    let key = RsaPublicKey::from_public_key_pem(CORRESPOND_PUBLIC_KEY)
        .context("invalid correspond public key")?;
    let encrypted = key
        .encrypt(
            &mut rsa::rand_core::OsRng,
            Oaep::new::<sha2::Sha256>(),
            format!("refresh_{timestamp_ms}").as_bytes(),
        )
        .context("failed to encrypt correspond path")?;
    Ok(hex_lower(&encrypted))
}

fn decode_page(body: &[u8], gzip: bool) -> Result<String> {
    if !gzip {
        return Ok(String::from_utf8_lossy(body).into_owned());
    }
    use std::io::Read;
    let mut html = String::new();
    flate2::read::GzDecoder::new(body).read_to_string(&mut html)?;
    Ok(html)
}

/// The `refresh_csrf` inside `<div id="1-name">…</div>` of the correspond page.
fn parse_refresh_csrf(html: &str) -> Option<String> {
    const OPEN: &str = "<div id=\"1-name\">";
    let start = html.find(OPEN)? + OPEN.len();
    let end = html[start..].find("</div>")? + start;
    let value = html[start..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl ApiClient {
    pub(super) fn cached_buvid3(&self) -> Option<String> {
        self.device_cookies
            .read()
            .expect("device cookies lock poisoned")
            .buvid3
            .clone()
    }

    /// Fetch `buvid3`/`buvid4` and a `bili_ticket` when missing or about to
    /// expire. Requests work without them, so callers may ignore failures.
    pub async fn ensure_device_cookies(&self) -> Result<()> {
        self.ensure_buvid().await?;
        let fresh = self
            .device_cookies
            .read()
            .expect("device cookies lock poisoned")
            .ticket_is_fresh(unix_now());
        if !fresh {
            let ticket = self.fetch_bili_ticket().await?;
            self.device_cookies
                .write()
                .expect("device cookies lock poisoned")
                .bili_ticket = Some(ticket);
        }
        Ok(())
    }

    pub(super) async fn ensure_buvid(&self) -> Result<()> {
        if self.cached_buvid3().is_some() {
            return Ok(());
        }
        let url = "https://api.bilibili.com/x/frontend/finger/spi";
        let response: ApiResponse<SpiData> = self.get(url).await?;
        let data = response.data.ok_or_else(|| anyhow!("buvid3 响应为空"))?;
        let mut device = self
            .device_cookies
            .write()
            .expect("device cookies lock poisoned");
        device.buvid3 = Some(data.b_3);
        device.buvid4 = data.b_4.filter(|value| !value.is_empty());
        device.b_nut = Some(unix_now());
        Ok(())
    }

    /// Returns the ticket and its expiry as a Unix timestamp.
    async fn fetch_bili_ticket(&self) -> Result<(String, i64)> {
        let ts = unix_now();
        let csrf = self.csrf_token().unwrap_or_default();
        let url = format!(
            "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket?key_id=ec02&hexsign={}&context%5Bts%5D={ts}&csrf={csrf}",
            hmac_sha256_hex(BILI_TICKET_KEY, &format!("ts{ts}"))
        );
        let mut request = self.client.post(&url);
        if let Some(cookies) = self.cookie_header() {
            request = request.header(COOKIE, cookies);
        }
        let body = request
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("bili_ticket request failed")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        let response: ApiResponse<TicketData> =
            Self::decode_api_response(&body).map_err(|error| anyhow!("bili_ticket: {error}"))?;
        let data = response
            .data
            .ok_or_else(|| anyhow!("bili_ticket response has no data"))?;
        Ok((data.ticket, data.created_at + data.ttl))
    }

    pub fn is_logged_in(&self) -> bool {
        self.csrf_token().is_some()
    }

    /// The account's `bili_jct`, which Bilibili uses as the CSRF token.
    pub(super) fn csrf_token(&self) -> Option<String> {
        let cookies = self.cookies.read().expect("cookies lock poisoned");
        cookies.as_ref()?.split(';').find_map(|part| {
            part.trim()
                .split_once('=')
                .filter(|(name, _)| *name == "bili_jct")
                .map(|(_, value)| value.to_string())
        })
    }

    /// Ask whether the logged-in cookies should be refreshed.
    pub async fn get_cookie_refresh_info(&self) -> Result<CookieRefreshInfo> {
        let url = format!(
            "{}/x/passport-login/web/cookie/info?csrf={}",
            BilibiliApiDomain::Passport.as_str(),
            self.csrf_token().unwrap_or_default()
        );
        let response: ApiResponse<CookieRefreshInfo> = self.get(&url).await?;
        response
            .data
            .ok_or_else(|| anyhow!("cookie info response has no data"))
    }

    /// Exchange `credentials` for new cookies with the refresh token, then
    /// confirm so the old session is retired. On success the client uses the
    /// new cookies; the caller persists the returned credentials.
    pub async fn refresh_cookies(
        &self,
        credentials: &Credentials,
        timestamp_ms: i64,
    ) -> Result<Credentials> {
        let refresh_token = credentials
            .refresh_token
            .clone()
            .ok_or_else(|| anyhow!("没有 refresh_token，无法刷新登录状态"))?;
        let refresh_csrf = self.get_refresh_csrf(timestamp_ms).await?;

        let url = format!(
            "{}/x/passport-login/web/cookie/refresh",
            BilibiliApiDomain::Passport.as_str()
        );
        let response = self
            .client
            .post(&url)
            .header(COOKIE, account_cookie(credentials))
            .form(&[
                ("csrf", credentials.bili_jct.as_str()),
                ("refresh_csrf", refresh_csrf.as_str()),
                ("source", "main_web"),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("cookie refresh request failed")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?;
        let cookies: Vec<(String, String)> = response
            .cookies()
            .map(|cookie| (cookie.name().to_string(), cookie.value().to_string()))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        let data: ApiResponse<CookieRefreshData> =
            Self::decode_api_response(&body).map_err(|error| anyhow!("cookie refresh: {error}"))?;
        let new_token = data
            .data
            .ok_or_else(|| anyhow!("cookie refresh response has no data"))?
            .refresh_token;
        let refreshed = Credentials::from_cookies(&cookies, Some(new_token))
            .ok_or_else(|| anyhow!("cookie refresh response did not set new cookies"))?;

        // The new cookies already work; a failed confirmation only leaves
        // the old session valid a little longer.
        let _ = self
            .confirm_cookie_refresh(&refreshed, &refresh_token)
            .await;
        self.set_credentials(&refreshed);
        Ok(refreshed)
    }

    async fn get_refresh_csrf(&self, timestamp_ms: i64) -> Result<String> {
        let url = format!(
            "https://www.bilibili.com/correspond/1/{}",
            correspond_path(timestamp_ms)?
        );
        let mut request = self.client.get(&url);
        if let Some(cookies) = self.cookie_header() {
            request = request.header(COOKIE, cookies);
        }
        let response = request
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("correspond request failed")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)
            .context("correspond page rejected the refresh")?;
        // The page is gzip-encoded even when the request did not offer it.
        let gzip = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"gzip"));
        let body = response
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        let html = decode_page(&body, gzip).context("failed to decode correspond page")?;
        parse_refresh_csrf(&html).ok_or_else(|| anyhow!("correspond page has no refresh_csrf"))
    }

    async fn confirm_cookie_refresh(
        &self,
        refreshed: &Credentials,
        old_refresh_token: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/x/passport-login/web/confirm/refresh",
            BilibiliApiDomain::Passport.as_str()
        );
        let body = self
            .client
            .post(&url)
            .header(COOKIE, account_cookie(refreshed))
            .form(&[
                ("csrf", refreshed.bili_jct.as_str()),
                ("refresh_token", old_refresh_token),
            ])
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        Self::decode_api_response::<serde_json::Value>(&body)
            .map(|_| ())
            .map_err(|error| anyhow!("confirm refresh: {error}"))
    }

    /// End the session on the server. Uses `credentials` rather than the
    /// client's cookies, which the caller may already have cleared.
    pub async fn logout(&self, credentials: &Credentials) -> Result<()> {
        let url = format!("{}/login/exit/v2", BilibiliApiDomain::Passport.as_str());
        let body = self
            .client
            .post(&url)
            .header(COOKIE, account_cookie(credentials))
            .form(&[("biliCSRF", credentials.bili_jct.as_str())])
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("logout request failed")?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        Self::decode_api_response::<serde_json::Value>(&body)
            .map(|_| ())
            .map_err(|error| anyhow!("logout: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_cookie_header_lists_only_known_values() {
        assert_eq!(DeviceCookies::default().header(), None);
        let cookies = DeviceCookies {
            buvid3: Some("B3".into()),
            buvid4: None,
            b_nut: Some(100),
            bili_ticket: Some(("T".into(), 200)),
        };
        assert_eq!(
            cookies.header().as_deref(),
            Some("buvid3=B3; b_nut=100; bili_ticket=T; bili_ticket_expires=200")
        );
    }

    #[test]
    fn bili_ticket_is_refetched_an_hour_before_expiry() {
        let cookies = DeviceCookies {
            bili_ticket: Some(("T".into(), 10_000)),
            ..DeviceCookies::default()
        };
        assert!(cookies.ticket_is_fresh(10_000 - BILI_TICKET_MARGIN_SECS - 1));
        assert!(!cookies.ticket_is_fresh(10_000 - BILI_TICKET_MARGIN_SECS));
        assert!(!DeviceCookies::default().ticket_is_fresh(0));
    }

    #[test]
    fn ticket_signature_matches_hmac_sha256() {
        // RFC 4231 test case 2.
        assert_eq!(
            hmac_sha256_hex(b"Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn correspond_path_is_a_1024_bit_ciphertext() {
        let path = correspond_path(1_700_000_000_000).unwrap();
        assert_eq!(path.len(), 256);
        assert!(path.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn refresh_csrf_is_read_from_the_correspond_page() {
        let html = r#"<html><div id="1-name">abc123</div><div id="2-name">x</div></html>"#;
        assert_eq!(parse_refresh_csrf(html).as_deref(), Some("abc123"));
        assert_eq!(parse_refresh_csrf("<html></html>"), None);
        assert_eq!(parse_refresh_csrf(r#"<div id="1-name"> </div>"#), None);
    }

    #[test]
    fn gzip_correspond_page_is_decoded() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(b"<div id=\"1-name\">x</div>").unwrap();
        let body = encoder.finish().unwrap();
        let html = decode_page(&body, true).unwrap();
        assert_eq!(parse_refresh_csrf(&html).as_deref(), Some("x"));
        assert!(decode_page(b"plain", false).unwrap() == "plain");
    }

    #[test]
    fn cookie_info_tolerates_missing_fields() {
        let info: CookieRefreshInfo = serde_json::from_str(r#"{"refresh":true}"#).unwrap();
        assert!(info.refresh);
        assert_eq!(info.timestamp, 0);
    }

    #[tokio::test]
    #[ignore = "contacts the live Bilibili API"]
    async fn guest_device_cookies_contract() {
        let client = ApiClient::new();
        client.ensure_device_cookies().await.unwrap();
        let header = client.cookie_header().unwrap();
        assert!(header.contains("buvid3=") && header.contains("bili_ticket="));
    }

    /// Read-only: asks whether a refresh is due and fetches `refresh_csrf`,
    /// which proves the correspond key, without refreshing.
    #[tokio::test]
    #[ignore = "contacts the live Bilibili API with saved credentials"]
    async fn cookie_refresh_probe_contract() {
        let credentials = crate::storage::load_credentials().expect("saved credentials");
        let client = ApiClient::with_cookies(&credentials);
        let info = client.get_cookie_refresh_info().await.unwrap();
        let timestamp = if info.timestamp > 0 {
            info.timestamp
        } else {
            chrono::Utc::now().timestamp_millis()
        };
        let csrf = client.get_refresh_csrf(timestamp).await.unwrap();
        assert!(!csrf.is_empty());
    }
}
