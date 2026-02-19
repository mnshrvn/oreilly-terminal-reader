use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use std::collections::HashSet;

const API_BASE: &str = "https://learning.oreilly.com";

#[derive(Debug, Clone)]
pub struct Chapter {
    pub title: String,
    pub url: String,
}

pub fn extract_book_id(url: &str) -> Result<String> {
    let re = Regex::new(r"(?:learning|www)\.oreilly\.com/library/view/[^/]+/(\d{10,13})")?;
    let caps = re.captures(url).context(
        "Could not extract book ID from URL. Expected format: \
         https://learning.oreilly.com/library/view/book-name/ISBN/ or \
         https://www.oreilly.com/library/view/book-name/ISBN/",
    )?;
    Ok(caps[1].to_string())
}

/// Fetch a URL and parse as JSON, with proper error messages on failure.
async fn get_json(client: &Client, url: &str) -> Result<Option<serde_json::Value>> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;

    if !status.is_success() {
        eprintln!("  {} returned HTTP {}", url, status);
        // Only flag as auth issue on 401/403 or explicit redirect to login page
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
            || (status.is_redirection()
                && body.contains("/login"))
        {
            anyhow::bail!(
                "API returned HTTP {}. Your session cookies are likely expired.\n\
                 Please re-export cookies from your browser and try again.",
                status
            );
        }
        return Ok(None);
    }

    match serde_json::from_str(&body) {
        Ok(json) => Ok(Some(json)),
        Err(e) => {
            eprintln!("  {} returned non-JSON (first 200 chars):", url);
            eprintln!("  {}", &body[..body.len().min(200)]);
            eprintln!("  Parse error: {}", e);
            Ok(None)
        }
    }
}

pub async fn fetch_book_info(client: &Client, book_id: &str) -> Result<(String, Vec<Chapter>)> {
    let mut title = format!("Book {}", book_id);
    let mut chapters = Vec::new();

    // Get book title from v1 API
    let v1_url = format!("{}/api/v1/book/{}/", API_BASE, book_id);
    if let Some(body) = get_json(client, &v1_url).await? {
        if let Some(t) = body["title"].as_str() {
            title = t.to_string();
        }
    }

    // Step 2: Get chapters from the paginated chapter endpoint
    // Each chapter entry has a "content" field with the v2 URL to the actual HTML
    {
        let mut page = 1;
        loop {
            let ch_url = format!(
                "{}/api/v1/book/{}/chapter/?page={}",
                API_BASE, book_id, page
            );
            eprint!(".");
            match get_json(client, &ch_url).await? {
                Some(body) => {
                    let results = body["results"].as_array().or_else(|| body.as_array());
                    if let Some(items) = results {
                        if items.is_empty() {
                            break;
                        }
                        for item in items {
                            let ch_title = item["title"]
                                .as_str()
                                .or_else(|| item["filename"].as_str())
                                .unwrap_or("Untitled")
                                .to_string();
                            let content_url = item["content"]
                                .as_str()
                                .unwrap_or("");
                            if !content_url.is_empty() {
                                let full_url = if content_url.starts_with("http") {
                                    content_url.to_string()
                                } else {
                                    format!("{}{}", API_BASE, content_url)
                                };
                                chapters.push(Chapter {
                                    title: ch_title,
                                    url: full_url,
                                });
                            }
                        }
                        if body["next"].is_null()
                            || body["next"].as_str().map_or(true, |s| s.is_empty())
                        {
                            break;
                        }
                        page += 1;
                    } else {
                        break;
                    }
                }
                None => break,
            }
        }
    }

    eprintln!();

    // Deduplicate: the paginated API may return multiple sections per HTML file.
    {
        let mut seen = HashSet::new();
        chapters.retain(|ch| seen.insert(ch.url.clone()));
    }

    // Fallback: try v2 epub files endpoint
    if chapters.is_empty() {
        let v2_url = format!(
            "{}/api/v2/epubs/urn:orm:book:{}/files/",
            API_BASE, book_id
        );
        eprintln!("  Trying v2 fallback...");
        if let Some(body) = get_json(client, &v2_url).await? {
            if let Some(results) = body.as_array().or_else(|| body["results"].as_array()) {
                for item in results {
                    let filename = item["filename"]
                        .as_str()
                        .unwrap_or("");

                    if filename.ends_with(".html") || filename.ends_with(".xhtml") {
                        let ch_title = item["title"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| filename.to_string());

                        let ch_url = format!(
                            "{}/api/v2/epubs/urn:orm:book:{}/files/{}",
                            API_BASE, book_id, filename
                        );

                        chapters.push(Chapter {
                            title: ch_title,
                            url: ch_url,
                        });
                    }
                }
            }
        }
    }

    if chapters.is_empty() {
        anyhow::bail!(
            "Could not retrieve chapters for this book. \
             Check the debug output above - if you see login redirects, \
             your cookies have expired."
        );
    }

    Ok((title, chapters))
}

pub async fn fetch_chapter_content(client: &Client, chapter: &Chapter) -> Result<String> {
    let resp = client
        .get(&chapter.url)
        .send()
        .await
        .context("Failed to fetch chapter")?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("Failed to fetch chapter: HTTP {}", status);
    }

    let body = resp.text().await?;

    // The response might be JSON wrapping HTML content
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(content) = json["content"].as_str() {
            return Ok(content.to_string());
        }
        if let Some(content) = json["html"].as_str() {
            return Ok(content.to_string());
        }
    }

    // Otherwise return as-is (raw HTML)
    Ok(body)
}
