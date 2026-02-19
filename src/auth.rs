use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, REFERER, UPGRADE_INSECURE_REQUESTS};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Used to verify the session is valid. We check multiple endpoints since O'Reilly
/// changes these over time.
const SESSION_CHECK_URLS: &[&str] = &[
    "https://learning.oreilly.com/profile/",
    "https://learning.oreilly.com/api/v1/me/",
    "https://learning.oreilly.com/api/v2/me/",
];

fn default_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        ),
    );
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://learning.oreilly.com/"),
    );
    headers.insert(
        UPGRADE_INSECURE_REQUESTS,
        HeaderValue::from_static("1"),
    );
    headers
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Serialize, Deserialize)]
struct StoredCookies {
    cookies: Vec<StoredCookie>,
}

#[derive(Serialize, Deserialize)]
struct StoredCookie {
    name: String,
    value: String,
    domain: String,
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("Could not determine config directory")?
        .join("oreilly-terminal-reader");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn cookies_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cookies.json"))
}

/// Add a cookie to the jar with proper Domain and Path attributes so it gets
/// sent to all oreilly.com subdomains (learning.oreilly.com, api.oreilly.com, etc.)
fn add_cookie(jar: &reqwest::cookie::Jar, name: &str, value: &str, domain: &str) {
    // Always use .oreilly.com as the domain for oreilly cookies so they're
    // sent to all subdomains
    let cookie_domain = if domain.contains("oreilly.com") {
        ".oreilly.com"
    } else {
        domain
    };

    let cookie_str = format!(
        "{}={}; Domain={}; Path=/",
        name, value, cookie_domain
    );

    // The URL we pass to add_cookie_str just needs to match the domain
    let url: reqwest::Url = format!(
        "https://{}",
        cookie_domain.trim_start_matches('.')
    )
    .parse()
    .unwrap();

    jar.add_cookie_str(&cookie_str, &url);
}

pub async fn build_authenticated_client(cookie_file: Option<&str>) -> Result<Client> {
    // If user provided an explicit cookie file, import it
    if let Some(path) = cookie_file {
        return load_cookies_from_file(path).await;
    }

    // Try stored cookies
    if let Ok(client) = try_stored_cookies().await {
        eprintln!("Using stored session.");
        return Ok(client);
    }

    let cookies_file = cookies_path()?;
    anyhow::bail!(
        "No valid session found.\n\n\
         O'Reilly's login is protected by bot detection (Akamai CDN), so you need to \
         export cookies from your browser after logging in.\n\n\
         Steps:\n\
         1. Log in to https://learning.oreilly.com in your browser\n\
         2. Export cookies using a browser extension:\n\
            - Chrome: \"Get cookies.txt LOCALLY\" or \"Cookie-Editor\"\n\
            - Firefox: \"Cookie-Editor\" or \"cookies.txt\"\n\
         3. Save as cookies.json or cookies.txt\n\
         4. Run: oreilly-terminal-reader --cookies <path-to-file> <book-url>\n\n\
         The cookies will be stored at {} for future use.",
        cookies_file.display()
    );
}

async fn load_cookies_from_file(path: &str) -> Result<Client> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("Could not read cookie file: {}", path))?;

    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());

    // Try to detect format and parse
    let data_trimmed = data.trim();
    let mut stored_cookies = Vec::new();

    if data_trimmed.starts_with('{') {
        // safaribooks format: {"cookie_name": "cookie_value", ...}
        let map: HashMap<String, String> = serde_json::from_str(data_trimmed)
            .context("Could not parse cookies.json as {name: value} map")?;
        for (name, value) in &map {
            add_cookie(&jar, name, value, ".oreilly.com");
            stored_cookies.push(StoredCookie {
                name: name.clone(),
                value: value.clone(),
                domain: ".oreilly.com".to_string(),
            });
        }
    } else if data_trimmed.starts_with('[') {
        // Array format: [{"name": "x", "value": "y", "domain": "z"}, ...]
        // This handles Cookie-Editor JSON export
        let entries: Vec<serde_json::Value> = serde_json::from_str(data_trimmed)
            .context("Could not parse cookies.json as array")?;
        for entry in &entries {
            let name = entry["name"].as_str().unwrap_or("");
            let value = entry["value"].as_str().unwrap_or("");
            let domain = entry["domain"].as_str().unwrap_or(".oreilly.com");
            if !name.is_empty() && !value.is_empty() {
                add_cookie(&jar, name, value, domain);
                stored_cookies.push(StoredCookie {
                    name: name.to_string(),
                    value: value.to_string(),
                    domain: domain.to_string(),
                });
            }
        }
    } else if data_trimmed.contains('\t') {
        // Netscape cookies.txt format (tab-separated)
        for line in data_trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // Skip comment lines, but handle #HttpOnly_ prefix
            // (httponly cookies are prefixed with #HttpOnly_ in Netscape format)
            let line = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
                rest
            } else if line.starts_with('#') {
                continue;
            } else {
                line
            };
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() >= 7 {
                let domain = fields[0];
                let name = fields[5];
                let value = fields[6];
                if domain.contains("oreilly.com") {
                    add_cookie(&jar, name, value, domain);
                    stored_cookies.push(StoredCookie {
                        name: name.to_string(),
                        value: value.to_string(),
                        domain: domain.to_string(),
                    });
                }
            }
        }
        if stored_cookies.is_empty() {
            anyhow::bail!("No oreilly.com cookies found in cookies.txt file");
        }
    } else {
        anyhow::bail!(
            "Unrecognized cookie file format. Supported formats:\n\
             - JSON object: {{\"name\": \"value\", ...}}\n\
             - JSON array (Cookie-Editor export): [{{\"name\": ..., \"value\": ..., \"domain\": ...}}]\n\
             - Netscape cookies.txt (tab-separated)"
        );
    }

    eprintln!("Loaded {} cookies from {}", stored_cookies.len(), path);
    save_stored_cookies(&StoredCookies { cookies: stored_cookies })?;

    let client = Client::builder()
        .cookie_provider(jar)
        .default_headers(default_headers())
        .user_agent(UA)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    // Verify session - try multiple endpoints, but don't fail hard
    // since the real test will be fetching the book
    eprintln!("Verifying session...");
    if verify_session(&client).await {
        eprintln!("Session valid.");
    } else {
        eprintln!("Warning: Could not verify session, but will try to fetch book content anyway.");
    }
    Ok(client)
}

async fn try_stored_cookies() -> Result<Client> {
    let path = cookies_path()?;
    if !path.exists() {
        anyhow::bail!("No stored cookies");
    }

    let data = std::fs::read_to_string(&path)?;
    let stored: StoredCookies = serde_json::from_str(&data)?;

    if stored.cookies.is_empty() {
        anyhow::bail!("No stored cookies");
    }

    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    for cookie in &stored.cookies {
        add_cookie(&jar, &cookie.name, &cookie.value, &cookie.domain);
    }

    let client = Client::builder()
        .cookie_provider(jar)
        .default_headers(default_headers())
        .user_agent(UA)
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    // Verify the session is still valid
    if !verify_session(&client).await {
        anyhow::bail!("Stored session expired");
    }

    Ok(client)
}

async fn verify_session(client: &Client) -> bool {
    for url in SESSION_CHECK_URLS {
        match client.get(*url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return true;
                }
                if status.is_redirection() || status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return false;
                }
            }
            Err(_) => continue,
        }
    }
    true
}

fn save_stored_cookies(stored: &StoredCookies) -> Result<()> {
    let path = cookies_path()?;
    std::fs::write(&path, serde_json::to_string_pretty(stored)?)?;
    eprintln!("Session saved to {}", path.display());
    Ok(())
}
