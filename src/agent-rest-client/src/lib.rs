//! Agent Rest Client: an HTTP client inside Agentty — requests, environments, saved collections,
//! authorization and a proxy — the kind of tool Agentty itself does not provide, added as a plugin.
//!
//! It is a Rust program compiled to WebAssembly, so it has no files, no sockets and no processes
//! of its own. Requests go through `net/fetch` (the `net.request` permission), which bounds the
//! method, headers, sizes, redirects and time; what the plugin remembers goes through
//! `storage/*`, which is its own folder under `plugin-data` and nothing else.

use agentty_plugin::text::{t, Lang};
use agentty_plugin::{export_plugin, ui, FetchRequest, Host, Plugin, UiEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response body kept for the panel (the panel itself refuses much more).
const BODY_SHOWN: usize = 8_000;
/// Requests remembered.
const HISTORY: usize = 20;
/// Headers per request, and variables per environment.
const MAX_ROWS: usize = 24;

const METHODS: &[(&str, &str)] =
    &[("GET", "GET"), ("POST", "POST"), ("PUT", "PUT"), ("PATCH", "PATCH"), ("DELETE", "DELETE"), ("HEAD", "HEAD"), ("OPTIONS", "OPTIONS")];

/// The tabs, and the kinds of authorization. The value on the left is the plugin's own and never
/// changes; the label on the right is what the user reads.
fn views(lang: Lang) -> Vec<(&'static str, &'static str)> {
    vec![
        ("request", t(lang, ["Request", "요청", "リクエスト", "请求"])),
        ("environments", t(lang, ["Environments", "환경", "環境", "环境"])),
        ("collection", t(lang, ["Collection", "모음", "コレクション", "收藏"])),
        ("settings", t(lang, ["Settings", "설정", "設定", "设置"])),
    ]
}

fn auth_kinds(lang: Lang) -> Vec<(&'static str, &'static str)> {
    vec![
        ("none", t(lang, ["No auth", "인증 없음", "認証なし", "无认证"])),
        ("bearer", t(lang, ["Bearer token", "Bearer 토큰", "Bearer トークン", "Bearer 令牌"])),
        ("basic", t(lang, ["Basic", "Basic", "Basic", "Basic"])),
        ("header", t(lang, ["API key header", "API 키 헤더", "API キーヘッダー", "API 密钥头"])),
    ]
}

/// What is kept between runs, one storage key each.
const KEYS: &[&str] = &["request", "environments", "collection", "history", "settings"];

// ----------------------------------------------------------------------------- state

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Auth {
    /// `none`, `bearer`, `basic` or `header`.
    kind: String,
    token: String,
    user: String,
    password: String,
    header: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Request {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    auth: Auth,
}

impl Request {
    fn method(&self) -> String {
        if self.method.is_empty() {
            "GET".to_string()
        } else {
            self.method.clone()
        }
    }

    fn has_body(&self) -> bool {
        !matches!(self.method().as_str(), "GET" | "HEAD")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Environment {
    name: String,
    /// Ordered, so rows stay where the user put them.
    values: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    name: String,
    request: Request,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    /// `http://host:port`, empty for none.
    proxy: String,
    /// Seconds; 0 means Agentty's own default.
    timeout: u64,
}

struct Outcome {
    status: u16,
    status_text: String,
    ok: bool,
    duration_ms: u64,
    bytes: usize,
    truncated: bool,
    headers: Vec<(String, String)>,
    body: String,
    pretty: Option<String>,
}

#[derive(Default)]
struct AgentRestClient {
    /// What the user reads, so the panel is in the language the rest of the window is in.
    lang: Lang,
    view: String,
    request: Request,
    environments: Vec<Environment>,
    active_env: String,
    collection: Vec<Saved>,
    history: Vec<Request>,
    settings: Settings,
    /// The `net/fetch` being waited for.
    pending: Option<u64>,
    outcome: Option<Outcome>,
    /// Stored values still being read at startup: (request id, key).
    loading: Vec<(u64, String)>,
    /// Name typed for "Save" and for a new environment.
    name: String,
    /// The environment whose values are open, by name.
    editing: String,
    pretty: bool,
    show_response_headers: bool,
    message: String,
}

// ----------------------------------------------------------------------------- variables

/// `{{name}}` replaced from the active environment. What has no value is left as it is and
/// reported, so a request never goes out half-filled without the user being told.
fn substitute(text: &str, values: &[(String, String)], missing: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = after[..end].trim();
        match values.iter().find(|(key, _)| key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                if !name.is_empty() && !missing.iter().any(|already| already == name) {
                    missing.push(name.to_string());
                }
                out.push_str(&rest[start..start + 2 + end + 2]);
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Pretty JSON, when it is JSON.
fn pretty_json(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

fn base64(bytes: &[u8]) -> String {
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let block = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let packed = (u32::from(block[0]) << 16) | (u32::from(block[1]) << 8) | u32::from(block[2]);
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(SET[(packed >> (18 - 6 * index)) as usize & 0x3f] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A shell argument, quoted the way a shell wants it: single quotes, and the one way out of them.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// What the current request looks like as a `curl` command — the same headers, the same body, the
/// same proxy, with the environment's values already filled in.
fn as_curl(method: &str, url: &str, headers: &[(String, String)], body: &str, proxy: &str) -> String {
    let mut parts = vec!["curl".to_string()];
    if method != "GET" {
        parts.push(format!("-X {method}"));
    }
    for (name, value) in headers {
        if name.trim().is_empty() {
            continue;
        }
        parts.push(format!("-H {}", shell_quote(&format!("{}: {value}", name.trim()))));
    }
    if !proxy.trim().is_empty() {
        parts.push(format!("--proxy {}", shell_quote(proxy.trim())));
    }
    if !body.trim().is_empty() {
        parts.push(format!("--data {}", shell_quote(body)));
    }
    parts.push(shell_quote(url));
    parts.join(" \\\n  ")
}

fn cut(text: &str, limit: usize) -> String {
    match text.chars().count() > limit {
        true => format!("{}…", text.chars().take(limit).collect::<String>()),
        false => text.to_string(),
    }
}

/// The line under the body field: how much is in it.
fn body_note(body: &str) -> Option<String> {
    let lines = body.lines().count();
    (!body.is_empty()).then(|| format!("{lines} lines, {} characters", body.chars().count()))
}

// ----------------------------------------------------------------------------- the plugin

impl AgentRestClient {
    fn view(&self) -> &str {
        if self.view.is_empty() {
            "request"
        } else {
            &self.view
        }
    }

    fn env_values(&self) -> Vec<(String, String)> {
        self.environments.iter().find(|env| env.name == self.active_env).map(|env| env.values.clone()).unwrap_or_default()
    }

    fn save(&self, host: &Host, key: &str) {
        let value = match key {
            "request" => serde_json::to_value(&self.request).ok(),
            "environments" => Some(serde_json::json!({ "list": self.environments, "active": self.active_env })),
            "collection" => serde_json::to_value(&self.collection).ok(),
            "history" => serde_json::to_value(&self.history).ok(),
            "settings" => serde_json::to_value(&self.settings).ok(),
            _ => None,
        };
        if let Some(value) = value {
            host.storage_set(key, value);
        }
    }

    fn restore(&mut self, key: &str, value: Value) {
        if value.is_null() {
            return;
        }
        match key {
            "request" => self.request = serde_json::from_value(value).unwrap_or_default(),
            "environments" => {
                self.environments = serde_json::from_value(value.get("list").cloned().unwrap_or(Value::Null)).unwrap_or_default();
                self.active_env = value.get("active").and_then(Value::as_str).unwrap_or_default().to_string();
            }
            "collection" => self.collection = serde_json::from_value(value).unwrap_or_default(),
            "history" => self.history = serde_json::from_value(value).unwrap_or_default(),
            "settings" => self.settings = serde_json::from_value(value).unwrap_or_default(),
            _ => {}
        }
    }

    // -- sending -------------------------------------------------------------------------

    fn send(&mut self, host: &Host) {
        if self.pending.is_some() {
            host.notify_user("info", t(self.lang, ["That request is still running", "그 요청은 아직 진행 중입니다", "そのリクエストはまだ実行中です", "该请求仍在进行中"]));
            return;
        }
        let values = self.env_values();
        let mut missing = Vec::new();
        let url = substitute(self.request.url.trim(), &values, &mut missing);
        if url.is_empty() {
            host.notify_user("warning", t(self.lang, ["Enter a URL first", "먼저 URL을 입력하세요", "先に URL を入力してください", "请先输入 URL"]));
            return;
        }
        let method = self.request.method();
        let mut fetch = FetchRequest::new(&method, &url);
        for (name, value) in &self.request.headers {
            let name = substitute(name.trim(), &values, &mut missing);
            if name.is_empty() {
                continue;
            }
            fetch = fetch.header(name, substitute(value, &values, &mut missing));
        }
        if let Some((name, value)) = self.auth_header(&values, &mut missing) {
            fetch = fetch.header(name, value);
        }
        let body = substitute(&self.request.body, &values, &mut missing);
        if self.request.has_body() && !body.trim().is_empty() {
            fetch = fetch.body(body);
        }
        fetch = fetch.proxy(substitute(self.settings.proxy.trim(), &values, &mut missing));
        if self.settings.timeout > 0 {
            fetch = fetch.timeout_ms(self.settings.timeout.min(60) * 1000);
        }
        if !missing.is_empty() {
            host.notify_user("warning", format!("{} {{{{{}}}}}", t(self.lang, ["No value for", "값이 없습니다:", "値がありません:", "没有值："]), missing.join("}, {")));
        }
        self.outcome = None;
        self.message = format!("{method} {url}");
        self.pending = Some(host.fetch(fetch));
        self.remember(host);
        host.set_badge("…");
    }

    /// The header the chosen authorization adds, if any.
    fn auth_header(&self, values: &[(String, String)], missing: &mut Vec<String>) -> Option<(String, String)> {
        let auth = &self.request.auth;
        let fill = |text: &str, missing: &mut Vec<String>| substitute(text.trim(), values, missing);
        match auth.kind.as_str() {
            "bearer" => {
                let token = fill(&auth.token, missing);
                (!token.is_empty()).then(|| ("Authorization".to_string(), format!("Bearer {token}")))
            }
            "basic" => {
                let user = fill(&auth.user, missing);
                let password = fill(&auth.password, missing);
                (!user.is_empty() || !password.is_empty())
                    .then(|| ("Authorization".to_string(), format!("Basic {}", base64(format!("{user}:{password}").as_bytes()))))
            }
            "header" => {
                let name = fill(&auth.header, missing);
                let token = fill(&auth.token, missing);
                (!name.is_empty() && !token.is_empty()).then_some((name, token))
            }
            _ => None,
        }
    }

    fn remember(&mut self, host: &Host) {
        let entry = self.request.clone();
        self.history.retain(|old| !(old.url == entry.url && old.method() == entry.method()));
        self.history.insert(0, entry);
        self.history.truncate(HISTORY);
        self.save(host, "history");
        self.save(host, "request");
    }

    // -- panel ---------------------------------------------------------------------------

    fn draw(&self, host: &Host) {
        let mut children = vec![ui::choice("view", &views(self.lang), self.view())];
        if !self.message.is_empty() {
            children.push(ui::styled_text(cut(&self.message, 300), "muted"));
        }
        children.push(ui::divider());
        match self.view() {
            "environments" => children.extend(self.environments_view()),
            "collection" => children.extend(self.collection_view()),
            "settings" => children.extend(self.settings_view()),
            _ => children.extend(self.request_view()),
        }
        host.set_panel(ui::column(children));
    }

    fn request_view(&self) -> Vec<Value> {
        let mut children = Vec::new();
        let env = if self.active_env.is_empty() { t(self.lang, ["No environment", "환경 없음", "環境なし", "无环境"]).to_string() } else { format!("{}: {}", t(self.lang, ["Environment", "환경", "環境", "环境"]), self.active_env) };
        children.push(ui::row(vec![ui::choice("method", METHODS, self.request.method()), ui::badge(env, "info")]));
        children.push(ui::input("url", t(self.lang, ["https://api.example.com/v1/things   ({{baseUrl}} works)", "https://api.example.com/v1/things   ({{baseUrl}} 사용 가능)", "https://api.example.com/v1/things   （{{baseUrl}} が使えます）", "https://api.example.com/v1/things   （可用 {{baseUrl}}）"]), self.request.url.clone()));
        children.push(ui::row(vec![
            ui::styled_button("send", if self.pending.is_some() { t(self.lang, ["Sending…", "보내는 중…", "送信中…", "发送中…"]) } else { t(self.lang, ["Send", "보내기", "送信", "发送"]) }, "primary"),
            ui::button("save", t(self.lang, ["Save", "저장", "保存", "保存"])),
            ui::button("curl", t(self.lang, ["Copy as cURL", "cURL로 복사", "cURL としてコピー", "复制为 cURL"])),
        ]));
        children.push(ui::input("name", t(self.lang, ["Name to save it under", "저장할 이름", "保存する名前", "保存名称"]), self.name.clone()));

        let mut headers = Vec::new();
        for (index, (name, value)) in self.request.headers.iter().enumerate() {
            headers.push(ui::row(vec![
                ui::input(format!("h.name.{index}"), t(self.lang, ["Header", "헤더", "ヘッダー", "标头"]), name.clone()),
                ui::input(format!("h.value.{index}"), t(self.lang, ["Value", "값", "値", "值"]), value.clone()),
                ui::styled_button(format!("h.del.{index}"), t(self.lang, ["Remove", "삭제", "削除", "移除"]), "ghost"),
            ]));
        }
        headers.push(ui::button("h.add", t(self.lang, ["Add header", "헤더 추가", "ヘッダーを追加", "添加标头"])));
        children.push(ui::section(t(self.lang, ["Headers", "헤더", "ヘッダー", "标头"]), headers));

        let auth = &self.request.auth;
        let kind = if auth.kind.is_empty() { "none".to_string() } else { auth.kind.clone() };
        let mut auth_rows = vec![ui::choice("auth.kind", &auth_kinds(self.lang), kind.clone())];
        match kind.as_str() {
            "bearer" => auth_rows.push(ui::input("auth.token", t(self.lang, ["Token   ({{token}} works)", "토큰   ({{token}} 사용 가능)", "トークン   （{{token}} が使えます）", "令牌   （可用 {{token}}）"]), auth.token.clone())),
            "basic" => auth_rows
                .push(ui::row(vec![ui::input("auth.user", t(self.lang, ["User", "사용자", "ユーザー", "用户"]), auth.user.clone()), ui::input("auth.password", t(self.lang, ["Password", "비밀번호", "パスワード", "密码"]), auth.password.clone())])),
            "header" => auth_rows.push(ui::row(vec![
                ui::input("auth.header", t(self.lang, ["Header name (X-API-Key)", "헤더 이름 (X-API-Key)", "ヘッダー名 (X-API-Key)", "标头名称 (X-API-Key)"]), auth.header.clone()),
                ui::input("auth.token", t(self.lang, ["Value", "값", "値", "值"]), auth.token.clone()),
            ])),
            _ => {}
        }
        children.push(ui::section(t(self.lang, ["Authorization", "인증", "認証", "认证"]), auth_rows));

        if self.request.has_body() {
            let mut body = vec![
                ui::textarea("body", "{\n  \"name\": \"value\"\n}", self.request.body.clone(), 10),
                ui::row(vec![ui::button("body.format", t(self.lang, ["Format JSON", "JSON 정리", "JSON を整形", "格式化 JSON"])), ui::button("body.clear", t(self.lang, ["Clear", "지우기", "クリア", "清空"]))]),
            ];
            if let Some(note) = body_note(&self.request.body) {
                body.push(ui::styled_text(note, "muted"));
            }
            children.push(ui::section(t(self.lang, ["Body", "본문", "ボディ", "正文"]), body));
        }

        if self.pending.is_some() {
            children.push(ui::spinner(t(self.lang, ["Waiting for the server…", "서버를 기다리는 중…", "サーバーを待っています…", "正在等待服务器…"])));
        }
        if let Some(outcome) = &self.outcome {
            children.push(ui::divider());
            children.push(ui::row(vec![
                ui::badge(format!("{} {}", outcome.status, outcome.status_text), if outcome.ok { "success" } else { "error" }),
                ui::styled_text(format!("{} ms", outcome.duration_ms), "muted"),
                ui::styled_text(format!("{} bytes", outcome.bytes), "muted"),
            ]));
            let mut row = vec![ui::toggle("response.headers", t(self.lang, ["Response headers", "응답 헤더", "レスポンスヘッダー", "响应标头"]), self.show_response_headers)];
            if outcome.pretty.is_some() {
                row.push(ui::toggle("response.pretty", t(self.lang, ["Pretty", "보기 좋게", "整形", "美化"]), self.pretty));
            }
            row.push(ui::button("response.copy", t(self.lang, ["Copy response", "응답 복사", "レスポンスをコピー", "复制响应"])));
            children.push(ui::row(row));
            if self.show_response_headers {
                let items: Vec<Value> =
                    outcome.headers.iter().map(|(name, value)| ui::item(name.clone(), name.clone(), cut(value, 200))).collect();
                children.push(ui::list("response.header.list", items, t(self.lang, ["None", "없음", "なし", "无"])));
            }
            if outcome.truncated {
                children.push(ui::styled_text(t(self.lang, ["The response was longer than 4 MB and is cut off.", "응답이 4MB를 넘어 잘렸습니다.", "レスポンスが 4 MB を超えたため途中で切れています。", "响应超过 4 MB，已被截断。"]), "muted"));
            }
            let body = match (&outcome.pretty, self.pretty) {
                (Some(pretty), true) => pretty.clone(),
                _ => outcome.body.clone(),
            };
            children.push(ui::styled_text(if body.is_empty() { t(self.lang, ["(empty body)", "(본문 없음)", "（ボディなし）", "（无正文）"]).to_string() } else { body }, "code"));
        }

        if !self.history.is_empty() {
            let items: Vec<Value> = self
                .history
                .iter()
                .enumerate()
                .map(|(index, request)| ui::item(index.to_string(), request.url.clone(), request.method()))
                .collect();
            children.push(ui::section(t(self.lang, ["History", "기록", "履歴", "历史"]), vec![ui::list("history", items, t(self.lang, ["Nothing yet", "아직 없음", "まだありません", "暂无"]))]));
        }
        children
    }

    fn environments_view(&self) -> Vec<Value> {
        let mut children = vec![ui::styled_text(
            t(self.lang, [
                "Values here fill in {{name}} anywhere in a request: the URL, a header, the body, a token, even the proxy.",
                "여기에 적은 값이 요청 어디에서나 {{name}} 자리에 들어갑니다. URL, 헤더, 본문, 토큰, 프록시까지.",
                "ここの値はリクエストのどこでも {{name}} に入ります。URL、ヘッダー、ボディ、トークン、プロキシまで。",
                "这里的值会填入请求中任何位置的 {{name}}：URL、标头、正文、令牌，以及代理。",
            ]),
            "muted",
        )];
        children.push(ui::row(vec![
            ui::input("name", t(self.lang, ["New environment name", "새 환경 이름", "新しい環境の名前", "新环境名称"]), self.name.clone()),
            ui::button("env.new", t(self.lang, ["Add environment", "환경 추가", "環境を追加", "添加环境"])),
        ]));
        let items: Vec<Value> = self
            .environments
            .iter()
            .map(|env| {
                let mark = if env.name == self.active_env { t(self.lang, [" · in use", " · 사용 중", " · 使用中", " · 使用中"]) } else { "" };
                ui::item(env.name.clone(), env.name.clone(), format!("{} {}{mark}", env.values.len(), t(self.lang, ["values", "개 값", "個の値", "个值"])))
            })
            .collect();
        children.push(ui::list("env.list", items, t(self.lang, ["No environments yet", "아직 환경이 없습니다", "まだ環境がありません", "还没有环境"])));

        if let Some(env) = self.environments.iter().find(|env| env.name == self.editing) {
            let mut rows = Vec::new();
            for (index, (key, value)) in env.values.iter().enumerate() {
                rows.push(ui::row(vec![
                    ui::input(format!("v.name.{index}"), t(self.lang, ["Name", "이름", "名前", "名称"]), key.clone()),
                    ui::input(format!("v.value.{index}"), t(self.lang, ["Value", "값", "値", "值"]), value.clone()),
                    ui::styled_button(format!("v.del.{index}"), t(self.lang, ["Remove", "삭제", "削除", "移除"]), "ghost"),
                ]));
            }
            rows.push(ui::row(vec![
                ui::button("v.add", t(self.lang, ["Add value", "값 추가", "値を追加", "添加值"])),
                ui::styled_button(
                    "env.use",
                    if env.name == self.active_env { t(self.lang, ["In use", "사용 중", "使用中", "使用中"]) } else { t(self.lang, ["Use this one", "이것 사용", "これを使う", "使用这个"]) },
                    "secondary",
                ),
                ui::styled_button("env.delete", t(self.lang, ["Delete environment", "환경 삭제", "環境を削除", "删除环境"]), "danger"),
            ]));
            children.push(ui::section(format!("{} — {}", env.name, t(self.lang, ["values", "값", "値", "值"])), rows));
        }
        children
    }

    fn collection_view(&self) -> Vec<Value> {
        let items: Vec<Value> = self
            .collection
            .iter()
            .map(|saved| {
                let mut item = ui::item(saved.name.clone(), saved.name.clone(), format!("{} {}", saved.request.method(), saved.request.url));
                item["actions"] = serde_json::json!([{ "id": "delete", "label": t(self.lang, ["Delete", "삭제", "削除", "删除"]) }]);
                item
            })
            .collect();
        vec![
            ui::styled_text(
                t(self.lang, [
                    "Saved requests. Pick one to load it back into the request view.",
                    "저장한 요청입니다. 하나를 고르면 요청 화면으로 불러옵니다.",
                    "保存したリクエストです。選ぶとリクエスト画面に読み込みます。",
                    "已保存的请求。选一个即可载回请求页。",
                ]),
                "muted",
            ),
            ui::list(
                "collection.list",
                items,
                t(self.lang, [
                    "Nothing saved yet — fill in a request, name it and press Save.",
                    "아직 저장한 것이 없습니다 — 요청을 채우고 이름을 적은 뒤 저장을 누르세요.",
                    "まだ保存がありません — リクエストを書き、名前を付けて保存を押してください。",
                    "还没有保存 — 填好请求、取个名字，然后按保存。",
                ]),
            ),
        ]
    }

    fn settings_view(&self) -> Vec<Value> {
        vec![
            ui::section(
                t(self.lang, ["Proxy", "프록시", "プロキシ", "代理"]),
                vec![
                    ui::input(
                        "settings.proxy",
                        t(self.lang, [
                            "http://127.0.0.1:8888   (empty: none)",
                            "http://127.0.0.1:8888   (비우면 사용 안 함)",
                            "http://127.0.0.1:8888   （空なら使わない）",
                            "http://127.0.0.1:8888   （留空则不使用）",
                        ]),
                        self.settings.proxy.clone(),
                    ),
                    ui::styled_text(
                        t(self.lang, [
                            "HTTP and HTTPS proxies, with user:password@ if yours asks for it.",
                            "HTTP·HTTPS 프록시를 쓸 수 있고, 인증이 필요하면 user:password@ 를 붙이세요.",
                            "HTTP と HTTPS のプロキシが使えます。認証が要るなら user:password@ を付けてください。",
                            "支持 HTTP 和 HTTPS 代理；若需认证，请加上 user:password@。",
                        ]),
                        "muted",
                    ),
                ],
            ),
            ui::section(
                t(self.lang, ["Timeout", "제한 시간", "タイムアウト", "超时"]),
                vec![
                    ui::input(
                        "settings.timeout",
                        t(self.lang, ["Seconds (empty: 15)", "초 (비우면 15)", "秒（空なら 15）", "秒（留空为 15）"]),
                        if self.settings.timeout == 0 { String::new() } else { self.settings.timeout.to_string() },
                    ),
                    ui::styled_text(t(self.lang, ["Agentty allows 60 seconds at most.", "Agentty는 최대 60초까지 허용합니다.", "Agentty が許すのは最大 60 秒です。", "Agentty 最多允许 60 秒。"]), "muted"),
                ],
            ),
            ui::section(
                t(self.lang, ["What this plugin may do", "이 플러그인이 할 수 있는 일", "このプラグインにできること", "这个插件能做什么"]),
                vec![ui::styled_text(
                    "It runs as WebAssembly inside Agentty: no files, no processes, no network of its own. Requests go out through \
                     Agentty with the net.request permission, and what you save here is kept in this plugin's own folder.",
                    "muted",
                )],
            ),
        ]
    }

    // -- events --------------------------------------------------------------------------

    fn edit_row(rows: &mut [(String, String)], index: usize, value: String, name_side: bool) {
        if let Some(row) = rows.get_mut(index) {
            if name_side {
                row.0 = value;
            } else {
                row.1 = value;
            }
        }
    }

    fn request_event(&mut self, host: &Host, event: &UiEvent) -> bool {
        let value = event.text();
        let parts: Vec<&str> = event.element.split('.').collect();
        match parts.as_slice() {
            ["method"] => self.request.method = value,
            ["url"] => {
                self.request.url = value;
                if event.event == "submit" {
                    self.send(host);
                }
            }
            ["name"] => self.name = value,
            ["send"] => self.send(host),
            ["save"] => {
                let name = if self.name.trim().is_empty() { self.request.url.trim().to_string() } else { self.name.trim().to_string() };
                if name.is_empty() {
                    host.notify_user("warning", t(self.lang, ["Give it a name first", "먼저 이름을 적으세요", "先に名前を付けてください", "请先取个名字"]));
                    return true;
                }
                let saved = Saved { name: name.clone(), request: self.request.clone() };
                self.collection.retain(|old| old.name != name);
                self.collection.insert(0, saved);
                self.name.clear();
                self.save(host, "collection");
                host.notify_user("success", format!("{} {name}", t(self.lang, ["Saved", "저장했습니다:", "保存しました:", "已保存："])));
            }
            ["h", "add"] => {
                if self.request.headers.len() < MAX_ROWS {
                    self.request.headers.push((String::new(), String::new()));
                }
            }
            ["h", what, index] => {
                let index: usize = index.parse().unwrap_or(usize::MAX);
                match *what {
                    "del" if index < self.request.headers.len() => {
                        self.request.headers.remove(index);
                    }
                    "name" => Self::edit_row(&mut self.request.headers, index, value, true),
                    "value" => Self::edit_row(&mut self.request.headers, index, value, false),
                    _ => return false,
                }
            }
            ["auth", field] => match *field {
                "kind" => self.request.auth.kind = value,
                "token" => self.request.auth.token = value,
                "user" => self.request.auth.user = value,
                "password" => self.request.auth.password = value,
                "header" => self.request.auth.header = value,
                _ => return false,
            },
            ["body"] => self.request.body = value,
            ["body", "format"] => match pretty_json(&self.request.body) {
                Some(pretty) => self.request.body = pretty,
                None => host.notify_user("warning", t(self.lang, ["That body is not JSON", "본문이 JSON이 아닙니다", "ボディが JSON ではありません", "正文不是 JSON"])),
            },
            ["body", "clear"] => self.request.body.clear(),
            ["curl"] => {
                let values = self.env_values();
                let mut missing = Vec::new();
                let mut headers: Vec<(String, String)> = self
                    .request
                    .headers
                    .iter()
                    .map(|(name, value)| (substitute(name, &values, &mut missing), substitute(value, &values, &mut missing)))
                    .collect();
                if let Some(auth) = self.auth_header(&values, &mut missing) {
                    headers.push(auth);
                }
                let command = as_curl(
                    &self.request.method(),
                    &substitute(self.request.url.trim(), &values, &mut missing),
                    &headers,
                    &substitute(&self.request.body, &values, &mut missing),
                    &substitute(&self.settings.proxy, &values, &mut missing),
                );
                host.copy(command);
                host.notify_user("success", t(self.lang, ["Copied the request as a cURL command", "요청을 cURL 명령으로 복사했습니다", "リクエストを cURL コマンドとしてコピーしました", "已将请求复制为 cURL 命令"]));
                return true;
            }
            ["response", "copy"] => {
                let Some(outcome) = &self.outcome else { return true };
                let body = match (&outcome.pretty, self.pretty) {
                    (Some(pretty), true) => pretty.clone(),
                    _ => outcome.body.clone(),
                };
                host.copy(body);
                host.notify_user("success", t(self.lang, ["Copied the response body", "응답 본문을 복사했습니다", "レスポンスのボディをコピーしました", "已复制响应正文"]));
                return true;
            }
            ["response", "pretty"] => self.pretty = event.is_on(),
            ["response", "headers"] => self.show_response_headers = event.is_on(),
            ["history"] => {
                if let Some(request) = event.item.as_deref().and_then(|id| id.parse::<usize>().ok()).and_then(|i| self.history.get(i)) {
                    self.request = request.clone();
                }
            }
            _ => return false,
        }
        self.save(host, "request");
        true
    }

    fn environment_event(&mut self, host: &Host, event: &UiEvent) -> bool {
        let value = event.text();
        let parts: Vec<&str> = event.element.split('.').collect();
        match parts.as_slice() {
            ["name"] => {
                self.name = value;
                return true;
            }
            ["env", "new"] => {
                let name = self.name.trim().to_string();
                if name.is_empty() {
                    host.notify_user("warning", t(self.lang, ["Name it first", "먼저 이름을 적으세요", "先に名前を付けてください", "请先取个名字"]));
                    return true;
                }
                if !self.environments.iter().any(|env| env.name == name) {
                    self.environments.push(Environment { name: name.clone(), values: Vec::new() });
                }
                self.editing = name.clone();
                if self.active_env.is_empty() {
                    self.active_env = name;
                }
                self.name.clear();
            }
            ["env", "list"] => {
                if let Some(name) = event.item.clone() {
                    self.editing = name;
                }
            }
            ["env", "use"] => self.active_env = self.editing.clone(),
            ["env", "delete"] => {
                self.environments.retain(|env| env.name != self.editing);
                if self.active_env == self.editing {
                    self.active_env.clear();
                }
                self.editing.clear();
            }
            ["v", "add"] => {
                if let Some(env) = self.environments.iter_mut().find(|env| env.name == self.editing) {
                    if env.values.len() < MAX_ROWS {
                        env.values.push((String::new(), String::new()));
                    }
                }
            }
            ["v", what, index] => {
                let index: usize = index.parse().unwrap_or(usize::MAX);
                let editing = self.editing.clone();
                let Some(env) = self.environments.iter_mut().find(|env| env.name == editing) else { return false };
                match *what {
                    "del" if index < env.values.len() => {
                        env.values.remove(index);
                    }
                    "name" => Self::edit_row(&mut env.values, index, value, true),
                    "value" => Self::edit_row(&mut env.values, index, value, false),
                    _ => return false,
                }
            }
            _ => return false,
        }
        self.save(host, "environments");
        true
    }

    fn collection_event(&mut self, host: &Host, event: &UiEvent) -> bool {
        if event.element != "collection.list" {
            return false;
        }
        let Some(name) = event.item.clone() else { return false };
        if event.action.as_deref() == Some("delete") {
            self.collection.retain(|saved| saved.name != name);
            self.save(host, "collection");
            return true;
        }
        if let Some(saved) = self.collection.iter().find(|saved| saved.name == name) {
            self.request = saved.request.clone();
            self.view = "request".to_string();
            self.save(host, "request");
        }
        true
    }

    fn settings_event(&mut self, host: &Host, event: &UiEvent) -> bool {
        let value = event.text();
        match event.element.as_str() {
            "settings.proxy" => self.settings.proxy = value,
            "settings.timeout" => self.settings.timeout = value.trim().parse().unwrap_or(0).min(60),
            _ => return false,
        }
        self.save(host, "settings");
        true
    }
}

impl Plugin for AgentRestClient {
    fn init(&mut self, host: &Host, _: &Value) {
        self.lang = host.language();
        for key in KEYS {
            self.loading.push((host.storage_get(key), key.to_string()));
        }
    }

    /// The user changed language, or moved to another workspace: either way this is where
    /// Agentty says what they read now.
    fn context(&mut self, host: &Host, _: &Value) {
        let lang = host.language();
        if lang != self.lang {
            self.lang = lang;
            self.draw(host);
        }
    }

    fn panel_open(&mut self, host: &Host) {
        self.draw(host);
    }

    fn command(&mut self, host: &Host, _: &str) {
        host.show_panel();
        self.draw(host);
    }

    fn ui_event(&mut self, host: &Host, event: UiEvent) {
        if event.element == "view" {
            self.view = event.text();
            self.message.clear();
        } else {
            let known = match self.view() {
                "environments" => self.environment_event(host, &event),
                "collection" => self.collection_event(host, &event),
                "settings" => self.settings_event(host, &event),
                _ => self.request_event(host, &event),
            };
            if !known {
                return;
            }
        }
        self.draw(host);
    }

    fn answer(&mut self, host: &Host, id: u64, result: Result<Value, String>) {
        // A stored value coming back while the plugin starts.
        if let Some(position) = self.loading.iter().position(|(pending, _)| *pending == id) {
            let (_, key) = self.loading.remove(position);
            if let Ok(value) = result {
                self.restore(&key, value.get("value").cloned().unwrap_or(Value::Null));
            }
            if self.loading.is_empty() {
                self.draw(host);
            }
            return;
        }
        if self.pending != Some(id) {
            return;
        }
        self.pending = None;
        match result {
            Ok(response) => {
                let status = response.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
                let body = response.get("body").and_then(Value::as_str).unwrap_or("");
                let pretty = pretty_json(body).map(|pretty| cut(&pretty, BODY_SHOWN));
                self.pretty = pretty.is_some();
                self.outcome = Some(Outcome {
                    status,
                    status_text: response.get("statusText").and_then(Value::as_str).unwrap_or("").to_string(),
                    ok: (200..400).contains(&status),
                    duration_ms: response.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
                    bytes: response.get("bytes").and_then(Value::as_u64).unwrap_or(0) as usize,
                    truncated: response.get("truncated").and_then(Value::as_bool).unwrap_or(false),
                    headers: response
                        .get("headers")
                        .and_then(Value::as_object)
                        .map(|headers| {
                            headers.iter().map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_string())).collect()
                        })
                        .unwrap_or_default(),
                    body: cut(body, BODY_SHOWN),
                    pretty,
                });
                host.set_badge(status.to_string());
            }
            Err(error) => {
                host.notify_user("error", &error);
                host.set_badge("!");
                self.message = error;
            }
        }
        self.draw(host);
    }
}

export_plugin!(AgentRestClient);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every view, in every language: the panel draws, and what the user reads is theirs.
    #[test]
    fn the_panel_is_in_the_language_the_user_reads() {
        let host = Host::new();
        let _ = agentty_plugin::host_stubs::taken();
        for lang in [Lang::En, Lang::Ko, Lang::Ja, Lang::Zh] {
            for view in ["request", "environments", "collection", "settings"] {
                let mut client = AgentRestClient { lang, view: view.to_string(), ..AgentRestClient::default() };
                client.environments.push(Environment { name: "local".into(), values: vec![("baseUrl".into(), "x".into())] });
                client.editing = "local".into();
                client.request.headers.push(("A".into(), "b".into()));
                client.request.auth.kind = "bearer".into();
                client.request.method = "POST".into();
                client.draw(&host);
                let sent = agentty_plugin::host_stubs::taken();
                let tree = sent.last().expect("a panel was drawn").clone();
                assert_eq!(tree["method"], "ui/setPanel", "{lang:?}/{view}");
                let drawn = tree.to_string();
                assert!(drawn.len() > 200, "{lang:?}/{view} drew almost nothing");
                if lang == Lang::Ko {
                    // Something on the panel is Korean: the tabs alone are, in every view.
                    let korean = drawn.chars().any(|c| ('\u{ac00}'..='\u{d7af}').contains(&c));
                    assert!(korean, "{view} has nothing Korean on it");
                }
            }
        }
    }

    #[test]
    fn what_is_on_the_wire_is_not_translated() {
        // The method and the tab are the plugin's own values, not what is shown: translating
        // them would send a Korean word where a server expects a verb.
        for lang in [Lang::En, Lang::Ko, Lang::Ja, Lang::Zh] {
            for (value, _) in views(lang) {
                assert!(value.is_ascii(), "{value} is not a value");
            }
            for (value, _) in auth_kinds(lang) {
                assert!(value.is_ascii(), "{value} is not a value");
            }
        }
        assert_eq!(views(Lang::Ko)[0].0, "request");
        assert_eq!(auth_kinds(Lang::Ja)[1].0, "bearer");
    }

    #[test]
    fn variables_are_filled_in_and_what_is_missing_is_named() {
        let values = vec![("baseUrl".to_string(), "https://api.example.com".to_string()), ("v".to_string(), "v2".to_string())];
        let mut missing = Vec::new();
        assert_eq!(substitute("{{baseUrl}}/{{v}}/things", &values, &mut missing), "https://api.example.com/v2/things");
        assert!(missing.is_empty());
        // What has no value stays as it is, and is named once.
        assert_eq!(substitute("{{baseUrl}}/{{token}}/{{token}}", &values, &mut missing), "https://api.example.com/{{token}}/{{token}}");
        assert_eq!(missing, ["token"]);
        // Half a placeholder is text.
        let mut missing = Vec::new();
        assert_eq!(substitute("{{unclosed", &values, &mut missing), "{{unclosed");
        assert_eq!(substitute("no placeholders", &values, &mut missing), "no placeholders");
        assert!(missing.is_empty());
    }

    #[test]
    fn basic_authorization_is_encoded() {
        assert_eq!(base64(b"user:password"), "dXNlcjpwYXNzd29yZA==");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }

    #[test]
    fn the_authorization_choice_becomes_a_header() {
        let mut client = AgentRestClient::default();
        let mut missing = Vec::new();
        assert!(client.auth_header(&[], &mut missing).is_none());

        client.request.auth = Auth { kind: "bearer".into(), token: "{{token}}".into(), ..Default::default() };
        let values = vec![("token".to_string(), "example_not_a_real_token".to_string())];
        assert_eq!(
            client.auth_header(&values, &mut missing),
            Some(("Authorization".to_string(), "Bearer example_not_a_real_token".to_string()))
        );

        client.request.auth = Auth { kind: "header".into(), header: "X-API-Key".into(), token: "k".into(), ..Default::default() };
        assert_eq!(client.auth_header(&values, &mut missing), Some(("X-API-Key".to_string(), "k".to_string())));

        client.request.auth = Auth { kind: "basic".into(), user: "u".into(), password: "p".into(), ..Default::default() };
        assert_eq!(client.auth_header(&values, &mut missing), Some(("Authorization".to_string(), format!("Basic {}", base64(b"u:p")))));
    }

    #[test]
    fn the_body_says_how_much_is_in_it() {
        assert!(body_note("").is_none());
        assert!(body_note("{\"a\":1}").unwrap().starts_with("1 lines"));
        assert!(body_note("{\n  \"a\": 1\n}").unwrap().starts_with("3 lines"));
        assert_eq!(pretty_json("{\"a\":1}").as_deref(), Some("{\n  \"a\": 1\n}"));
        assert!(pretty_json("not json").is_none());
    }

    #[test]
    fn a_request_becomes_a_curl_command() {
        let headers = vec![("Accept".to_string(), "application/json".to_string())];
        let command = as_curl("POST", "https://api.example.com/things", &headers, "{\"a\":1}", "");
        assert!(command.starts_with("curl \\\n  -X POST"));
        assert!(command.contains("-H 'Accept: application/json'"));
        assert!(command.contains("--data '{\"a\":1}'"));
        assert!(command.ends_with("'https://api.example.com/things'"));
        // A GET needs no -X, and a proxy comes along when there is one.
        let plain = as_curl("GET", "https://api.example.com", &[], "", "http://127.0.0.1:8888");
        assert!(!plain.contains("-X"));
        assert!(plain.contains("--proxy 'http://127.0.0.1:8888'"));
    }

    #[test]
    fn a_quote_in_a_value_cannot_end_the_quoting() {
        // The one thing that would let a value become a command of its own.
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        let command = as_curl("GET", "https://x.example/'; rm -rf /; echo '", &[], "", "");
        assert!(command.ends_with("'https://x.example/'\\''; rm -rf /; echo '\\'''"), "{command}");
    }

    #[test]
    fn a_get_carries_no_body_and_a_post_does() {
        let mut request = Request::default();
        assert_eq!(request.method(), "GET");
        assert!(!request.has_body());
        request.method = "POST".into();
        assert!(request.has_body());
    }

    #[test]
    fn what_is_saved_comes_back_the_same() {
        let mut client = AgentRestClient::default();
        client.request = Request {
            method: "POST".into(),
            url: "{{baseUrl}}/things".into(),
            headers: vec![("Accept".into(), "application/json".into())],
            body: "{\"a\":1}".into(),
            auth: Auth { kind: "bearer".into(), token: "t".into(), ..Default::default() },
        };
        let stored = serde_json::to_value(&client.request).unwrap();
        let mut restored = AgentRestClient::default();
        restored.restore("request", stored);
        assert_eq!(restored.request, client.request);

        // An environment keeps its order and which one is in use.
        client.environments = vec![Environment { name: "local".into(), values: vec![("baseUrl".into(), "http://localhost:3000".into())] }];
        client.active_env = "local".into();
        let stored = serde_json::json!({ "list": client.environments, "active": client.active_env });
        restored.restore("environments", stored);
        assert_eq!(restored.active_env, "local");
        assert_eq!(restored.env_values(), [("baseUrl".to_string(), "http://localhost:3000".to_string())]);
    }
}
