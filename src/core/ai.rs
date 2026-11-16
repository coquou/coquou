//! Image-model access for "hatching" pets with AI.
//!
//! Users bring their own endpoint: a profile is { protocol, base URL, model }
//! and its API key lives in the OS keychain (macOS Keychain / Windows
//! Credential Manager), never in the profile file. Two OpenAI-style protocols:
//!
//! - `images`: POST {base}/images/generations (text only, JSON) or
//!   POST {base}/images/edits (multipart, reference images as repeated
//!   `image[]` parts); result in `data[0].b64_json|url`.
//! - `chat`: POST {base}/chat/completions with `modalities: ["image","text"]`
//!   and reference images as `image_url` data URLs; result in
//!   `choices[0].message.images[].image_url.url` (data URL), with fallbacks
//!   for relays that put the image in `content`.
//!
//! Requests run in Rust (no CORS, key never reaches the webview). The key is
//! only sent over https, or plain http to this machine (the offline
//! self-test). Profiles are read-modified-written under one lock.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

/// Keychain / Credential Manager service: the app identifier.
const KEYCHAIN_SERVICE: &str = "io.github.coquou";
/// Where keys lived before (moved on first use): the old app names, and
/// `io.github.coquou.ai` before the service became the app identifier.
const OLD_KEYCHAIN_SERVICES: [&str; 3] = ["io.github.coquou.ai", "io.github.pipkin.ai", "io.perchy.desktop.ai"];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// "images" | "chat"
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    /// Optional `quality` for the images protocol (low / medium / high / auto).
    #[serde(default)]
    pub quality: Option<String>,
    /// Ask the images protocol for `background: transparent`.
    #[serde(default)]
    pub transparent: bool,
    /// How this endpoint takes reference images (images protocol), learnt on
    /// first use: None = OpenAI multipart `/images/edits`; "references" =
    /// JSON `/images` with `input_references` (OpenRouter).
    #[serde(default)]
    pub edits: Option<String>,
    /// Whether a key was stored for this profile. Kept here so listing
    /// profiles never touches the keychain (each read can prompt on macOS).
    #[serde(default)]
    pub key_saved: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    #[serde(flatten)]
    profile: Profile,
    has_key: bool,
}

fn profiles_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|d| d.join("ai-profiles.json"))
        .map_err(|e| e.to_string())
}

/// Serialises every read-modify-write of ai-profiles.json.
static PROFILES_LOCK: Mutex<()> = Mutex::new(());

fn load<R: Runtime>(app: &AppHandle<R>) -> Vec<Profile> {
    profiles_path(app).map(|p| crate::fsutil::read_json_or_default(&p)).unwrap_or_default()
}

/// Load the profiles, apply `change`, save them — all under `PROFILES_LOCK`.
fn modify<R: Runtime, T>(app: &AppHandle<R>, change: impl FnOnce(&mut Vec<Profile>) -> Result<T, String>) -> Result<T, String> {
    let _guard = PROFILES_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut profiles = load(app);
    let out = change(&mut profiles)?;
    let path = profiles_path(app)?;
    let json = serde_json::to_string_pretty(&profiles).map_err(|e| e.to_string())?;
    crate::fsutil::write_atomic(&path, json.as_bytes()).map_err(|e| format!("保存 AI 配置失败：{e}"))?;
    Ok(out)
}

/// The key may travel over plain http only to this machine.
fn transport_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').filter(|(h, port)| !h.is_empty() && port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

fn entry(id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, id).map_err(|e| format!("钥匙串不可用：{e}"))
}

/// Keys read from the keychain are cached for the life of the process, so
/// macOS asks for keychain access at most once per profile per launch.
fn key_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn get_key(id: &str) -> Option<String> {
    if let Some(k) = key_cache().lock().unwrap().get(id) {
        return Some(k.clone());
    }
    let key = match entry(id).ok()?.get_password().ok().filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => {
            // Saved under an old name: copy it over.
            let (old, k) = OLD_KEYCHAIN_SERVICES.iter().find_map(|s| {
                let e = keyring::Entry::new(s, id).ok()?;
                let k = e.get_password().ok().filter(|k| !k.is_empty())?;
                Some((e, k))
            })?;
            if entry(id).and_then(|e| e.set_password(&k).map_err(|e| e.to_string())).is_ok() {
                let _ = old.delete_credential();
            }
            k
        }
    };
    key_cache().lock().unwrap().insert(id.to_string(), key.clone());
    Some(key)
}

fn view(p: Profile) -> ProfileView {
    // Profiles saved before `key_saved` existed: true is the likely answer
    // and the first real use reads (and caches) the key anyway.
    let has_key = p.key_saved.unwrap_or(true) || key_cache().lock().unwrap().contains_key(&p.id);
    ProfileView { profile: p, has_key }
}

#[tauri::command]
pub fn ai_profiles(app: AppHandle) -> Vec<ProfileView> {
    load(&app).into_iter().map(view).collect()
}

/// Create or update a profile. `api_key`: None/empty keeps the stored key.
#[tauri::command]
pub fn ai_save_profile(app: AppHandle, mut profile: Profile, api_key: Option<String>) -> Result<ProfileView, String> {
    if !matches!(profile.protocol.as_str(), "images" | "chat") {
        return Err(format!("不支持的协议：{}", profile.protocol));
    }
    let url = profile.base_url.trim().trim_end_matches('/').to_string();
    if !transport_allowed(&url) {
        return Err("API 地址需要以 https:// 开头（http:// 只能用于本机地址）".into());
    }
    profile.base_url = url;
    profile.model = profile.model.trim().to_string();
    if profile.model.is_empty() {
        return Err("请填写模型名".into());
    }
    if profile.id.is_empty() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        profile.id = format!("p{n}");
    }
    if let Some(key) = api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        entry(&profile.id)?.set_password(&key).map_err(|e| format!("保存 API key 失败：{e}"))?;
        key_cache().lock().unwrap().insert(profile.id.clone(), key);
        profile.key_saved = Some(true);
    }
    let saved = modify(&app, |profiles| {
        if profile.key_saved.is_none() {
            profile.key_saved = profiles.iter().find(|p| p.id == profile.id).and_then(|p| p.key_saved).or(Some(false));
        }
        match profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile.clone(),
            None => profiles.push(profile.clone()),
        }
        Ok(profile)
    })?;
    Ok(view(saved))
}

#[tauri::command]
pub fn ai_delete_profile(app: AppHandle, id: String) -> Result<(), String> {
    modify(&app, |profiles| {
        profiles.retain(|p| p.id != id);
        Ok(())
    })?;
    if let Ok(e) = entry(&id) {
        let _ = e.delete_credential();
    }
    for s in OLD_KEYCHAIN_SERVICES {
        if let Ok(e) = keyring::Entry::new(s, &id) {
            let _ = e.delete_credential();
        }
    }
    key_cache().lock().unwrap().remove(&id);
    Ok(())
}

/// One HTTP client for all requests (connection reuse).
fn client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c.clone());
    }
    let c = reqwest::Client::builder().timeout(Duration::from_secs(360)).build().map_err(|e| e.to_string())?;
    Ok(CLIENT.get_or_init(|| c).clone())
}

/// Largest response body accepted (generated images are a few MB).
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

async fn read_body(mut resp: reqwest::Response) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("接口返回的数据太大".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Turn an HTTP error into something a user can act on.
async fn check(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let text = String::from_utf8_lossy(&read_body(resp).await?).into_owned();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body["error"]["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| text.chars().take(300).collect());
        let hint = match status.as_u16() {
            401 | 403 => "（API key 无效或没有权限）",
            404 => "（地址或模型名不对？地址一般以 /v1 结尾）",
            429 => "（请求太频繁或额度不足）",
            _ => "",
        };
        return Err(format!("HTTP {status}{hint}：{msg}"));
    }
    if body.is_null() {
        return Err(format!("返回的不是 JSON：{}", text.chars().take(200).collect::<String>()));
    }
    Ok(body)
}

async fn fetch_b64(url: &str) -> Result<String, String> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (_, data) = rest.split_once(";base64,").ok_or("无法解析返回的 data URL")?;
        return Ok(data.to_string());
    }
    let resp = client()?.get(url).send().await.map_err(|e| format!("下载生成的图片失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载生成的图片失败：HTTP {}", resp.status()));
    }
    Ok(B64.encode(read_body(resp).await?))
}

/// JSON `/images` with `input_references` (OpenRouter's image API).
async fn images_with_references(p: &Profile, key: &str, prompt: &str, images: &[String], size: &str) -> Result<reqwest::Response, String> {
    let refs: Vec<Value> = images
        .iter()
        .map(|img| json!({ "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{img}") } }))
        .collect();
    let mut req = json!({ "model": p.model, "prompt": prompt, "size": size, "input_references": refs });
    if let Some(q) = &p.quality {
        req["quality"] = json!(q);
    }
    if p.transparent {
        req["background"] = json!("transparent");
    }
    client()?
        .post(format!("{}/images", p.base_url))
        .bearer_auth(key)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("连接失败：{e}"))
}

/// `fell_back` is set as soon as the endpoint turns out to have no
/// `/images/edits` and `/images` + `input_references` is used instead, even
/// if that request then fails, so the caller can remember it on the profile
/// (and a retry doesn't probe `/images/edits` again).
async fn via_images(p: &Profile, key: &str, prompt: &str, images: &[String], size: &str, fell_back: &AtomicBool) -> Result<String, String> {
    let http = client()?;
    let body = if !images.is_empty() && p.edits.as_deref() == Some("references") {
        images_with_references(p, key, prompt, images, size).await?
    } else if images.is_empty() {
        // `n` is left out on purpose: 1 is the default and some relays reject it.
        let mut req = json!({ "model": p.model, "prompt": prompt, "size": size });
        if let Some(q) = &p.quality {
            req["quality"] = json!(q);
        }
        if p.transparent {
            req["background"] = json!("transparent");
        }
        http.post(format!("{}/images/generations", p.base_url))
            .bearer_auth(key)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("连接失败：{e}"))?
    } else {
        let mut form = reqwest::multipart::Form::new()
            .text("model", p.model.clone())
            .text("prompt", prompt.to_string())
            .text("size", size.to_string());
        if let Some(q) = &p.quality {
            form = form.text("quality", q.clone());
        }
        if p.transparent {
            form = form.text("background", "transparent");
        }
        for (i, img) in images.iter().enumerate() {
            let bytes = B64.decode(img).map_err(|e| e.to_string())?;
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("ref{i}.png"))
                .mime_str("image/png")
                .expect("static MIME type");
            form = form.part("image[]", part);
        }
        let resp = http
            .post(format!("{}/images/edits", p.base_url))
            .bearer_auth(key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("连接失败：{e}"))?;
        // No /images/edits here (e.g. OpenRouter): use /images + input_references.
        if matches!(resp.status().as_u16(), 404 | 405) {
            log::info!("ai: /images/edits not found, trying /images with input_references");
            fell_back.store(true, Ordering::Relaxed);
            images_with_references(p, key, prompt, images, size).await?
        } else {
            resp
        }
    };
    let body = check(body).await?;
    let item = &body["data"][0];
    if let Some(b64) = item["b64_json"].as_str() {
        return Ok(b64.to_string());
    }
    if let Some(url) = item["url"].as_str() {
        return fetch_b64(url).await;
    }
    Err("返回里没有图片（data[0].b64_json / url）".into())
}

/// Find the first image in a chat completion, tolerating relay variants.
fn chat_image(body: &Value) -> Option<String> {
    let msg = &body["choices"][0]["message"];
    if let Some(url) = msg["images"][0]["image_url"]["url"].as_str() {
        return Some(url.to_string());
    }
    if let Some(parts) = msg["content"].as_array() {
        for part in parts {
            if let Some(url) = part["image_url"]["url"].as_str() {
                return Some(url.to_string());
            }
        }
    }
    let text = msg["content"].as_str()?;
    if let Some(start) = text.find("data:image/") {
        let end = text[start..].find(|c: char| c == ')' || c == '"' || c.is_whitespace()).map_or(text.len(), |e| start + e);
        return Some(text[start..end].to_string());
    }
    // Markdown image link ![..](https://...)
    let start = text.find("](http")? + 2;
    let end = text[start..].find(')').map(|e| start + e)?;
    Some(text[start..end].to_string())
}

async fn via_chat(p: &Profile, key: &str, prompt: &str, images: &[String], size: &str) -> Result<String, String> {
    let mut content = vec![json!({ "type": "text", "text": format!("{prompt}\n\nOutput one image, size {size}.") })];
    for img in images {
        content.push(json!({ "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{img}") } }));
    }
    let req = json!({
        "model": p.model,
        "messages": [{ "role": "user", "content": content }],
        "modalities": ["image", "text"],
    });
    let resp = client()?
        .post(format!("{}/chat/completions", p.base_url))
        .bearer_auth(key)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("连接失败：{e}"))?;
    let body = check(resp).await?;
    let url = chat_image(&body).ok_or_else(|| {
        let said = body["choices"][0]["message"]["content"].as_str().unwrap_or("").chars().take(160).collect::<String>();
        format!("模型没有返回图片（确认模型支持图像输出）{}", if said.is_empty() { String::new() } else { format!("：{said}") })
    })?;
    fetch_b64(&url).await
}

/// Generate one image. `images` are base64 PNG references; returns base64 image bytes.
#[tauri::command]
pub async fn ai_generate(
    app: AppHandle,
    profile_id: String,
    prompt: String,
    images: Vec<String>,
    size: String,
) -> Result<String, String> {
    let profile = load(&app).into_iter().find(|p| p.id == profile_id).ok_or("找不到这个 AI 配置")?;
    let key = get_key(&profile.id).ok_or("这个配置还没有填 API key")?;
    let started = std::time::Instant::now();
    let fell_back = AtomicBool::new(false);
    let call = |p: Profile| {
        let (key, prompt, images, size, fell_back) = (key.clone(), prompt.clone(), images.clone(), size.clone(), &fell_back);
        async move {
            match p.protocol.as_str() {
                "images" => via_images(&p, &key, &prompt, &images, &size, fell_back).await,
                _ => via_chat(&p, &key, &prompt, &images, &size).await,
            }
        }
    };
    // What a call teaches about the endpoint is written onto the *current*
    // saved profile (field by field), never by copying this request's
    // snapshot back: other rows may run at once and the user may edit it.
    let learn = |change: &dyn Fn(&mut Profile)| {
        let r = modify(&app, |all| {
            if let Some(p) = all.iter_mut().find(|p| p.id == profile_id) {
                change(p);
            }
            Ok(())
        });
        if let Err(e) = r {
            log::warn!("ai: {e}");
        }
    };
    let mut used = profile.clone();
    let mut result = call(used.clone()).await;
    // Optional parameters the model/relay rejects (e.g. gpt-image-2 has no
    // transparent background): drop them, remember that, and retry once.
    // Rejected requests are normally not billed.
    if let Err(e) = &result {
        if let Some(fixed) = without_rejected(&used, &e.to_lowercase()) {
            log::info!("ai: endpoint rejected optional params, retrying without them");
            let (transparent, quality) = (fixed.transparent, fixed.quality.clone());
            learn(&|p| {
                if !transparent {
                    p.transparent = false;
                }
                if quality.is_none() {
                    p.quality = None;
                }
            });
            used = fixed;
            if fell_back.load(Ordering::Relaxed) {
                used.edits = Some("references".into());
            }
            result = call(used.clone()).await;
        }
    }
    if fell_back.load(Ordering::Relaxed) {
        learn(&|p| p.edits = Some("references".into()));
    }
    let profile = used;
    log::info!(
        "ai {} {} refs={} size={} → {} in {:.1}s",
        profile.protocol,
        profile.model,
        images.len(),
        size,
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed().as_secs_f32()
    );
    result
}

/// The profile minus the optional parameters a 400 error complains about.
fn without_rejected(p: &Profile, error_lower: &str) -> Option<Profile> {
    if !error_lower.starts_with("http 400") {
        return None;
    }
    let mut q = p.clone();
    if q.transparent && error_lower.contains("background") {
        q.transparent = false;
    }
    if q.quality.is_some() && error_lower.contains("quality") {
        q.quality = None;
    }
    (q.transparent != p.transparent || q.quality != p.quality).then_some(q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_only_over_https_or_to_this_machine() {
        assert!(transport_allowed("https://api.openai.com/v1"));
        assert!(transport_allowed("http://127.0.0.1:8765/v1"));
        assert!(transport_allowed("http://localhost/v1"));
        assert!(transport_allowed("http://[::1]:8000/v1"));
        assert!(!transport_allowed("http://relay.example.com/v1"));
        assert!(!transport_allowed("http://localhost.evil.com/v1"));
        assert!(!transport_allowed("ftp://x"));
    }

    #[test]
    fn drops_rejected_params() {
        let p = Profile {
            id: "x".into(), name: "x".into(), protocol: "images".into(), base_url: "u".into(),
            model: "m".into(), quality: Some("low".into()), transparent: true, edits: None, key_saved: None,
        };
        let err = "http 400 bad request：no provider for openai/gpt-image-2 supports the requested parameter(s): background \"transparent\"";
        let q = without_rejected(&p, err).unwrap();
        assert!(!q.transparent);
        assert_eq!(q.quality.as_deref(), Some("low"));
        assert!(without_rejected(&q, err).is_none(), "no second retry");
        assert!(without_rejected(&p, "http 401 unauthorized：background").is_none());
    }

    #[test]
    fn chat_image_variants() {
        let a = json!({"choices":[{"message":{"images":[{"image_url":{"url":"data:image/png;base64,AAA"}}]}}]});
        assert_eq!(chat_image(&a).unwrap(), "data:image/png;base64,AAA");
        let b = json!({"choices":[{"message":{"content":[{"type":"image_url","image_url":{"url":"https://x/y.png"}}]}}]});
        assert_eq!(chat_image(&b).unwrap(), "https://x/y.png");
        let c = json!({"choices":[{"message":{"content":"here ![img](https://cdn/x.png) done"}}]});
        assert_eq!(chat_image(&c).unwrap(), "https://cdn/x.png");
        let d = json!({"choices":[{"message":{"content":"![](data:image/webp;base64,QQ==)"}}]});
        assert_eq!(chat_image(&d).unwrap(), "data:image/webp;base64,QQ==");
        let e = json!({"choices":[{"message":{"content":"sorry, text only"}}]});
        assert!(chat_image(&e).is_none());
    }
}
