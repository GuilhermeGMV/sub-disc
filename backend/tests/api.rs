use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use subdisc::{AppState, app, config::Config, db};
use tower::ServiceExt;
struct TestApp {
    app: Router,
    state: AppState,
    _dir: tempfile::TempDir,
}
impl TestApp {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            database_url: format!("sqlite://{}", dir.path().join("test.db").display()),
            origin: "http://localhost".into(),
            bind: "127.0.0.1:0".into(),
            secure: false,
            setup_key: "a-safe-setup-test-key".into(),
            livekit_url: "ws://localhost:7880".into(),
            livekit_internal: "http://127.0.0.1:1".into(),
            livekit_key: "testkey".into(),
            livekit_secret: "test-secret-at-least-thirty-two-characters".into(),
            static_dir: "../frontend/dist".into(),
        };
        let state = AppState::new(db::connect(&config.database_url).await.unwrap(), config);
        Self {
            app: app(state.clone()),
            state,
            _dir: dir,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Value,
        cookie: &str,
    ) -> (StatusCode, Value, String) {
        self.origin(method, path, body, cookie, "http://localhost")
            .await
    }
    async fn origin(
        &self,
        method: &str,
        path: &str,
        body: Value,
        cookie: &str,
        origin: &str,
    ) -> (StatusCode, Value, String) {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("cookie", cookie)
            .header("origin", origin)
            .extension(ConnectInfo(
                "127.0.0.1:1234".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let cookie = res
            .headers()
            .get("set-cookie")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            cookie,
        )
    }
    async fn admin(&self) -> String {
        let(status,_,cookie)=self.call("POST","/api/setup",json!({"username":"admin","password":"strong password 123","setup_key":"a-safe-setup-test-key"}),"").await;
        assert_eq!(status, StatusCode::OK);
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        cookie
    }
    async fn member(&self, admin: &str) -> String {
        let (s, v, _) = self
            .call("POST", "/api/invites", json!({"hours":24}), admin)
            .await;
        assert_eq!(s, 200);
        let (s, _, c) = self
            .call(
                "POST",
                "/api/register",
                json!({"username":"friend","password":"strong password 123","invite":v["token"]}),
                "",
            )
            .await;
        assert_eq!(s, 200);
        c
    }
}
#[tokio::test]
async fn setup_login_logout_and_persistence() {
    let t = TestApp::new().await;
    assert_eq!(t.call("GET", "/api/me", json!({}), "").await.0, 401);
    assert_eq!(
        t.call(
            "POST",
            "/api/setup",
            json!({"username":"admin","password":"strong password 123","setup_key":"bad"}),
            ""
        )
        .await
        .0,
        403
    );
    let admin = t.admin().await;
    assert_eq!(t.call("POST","/api/setup",json!({"username":"admin","password":"strong password 123","setup_key":"a-safe-setup-test-key"}),"").await.0,409);
    assert_eq!(
        t.call(
            "POST",
            "/api/login",
            json!({"username":"admin","password":"wrong password 123"}),
            ""
        )
        .await
        .0,
        401
    );
    let (s, u, c) = t
        .call(
            "POST",
            "/api/login",
            json!({"username":"ADMIN","password":"strong password 123"}),
            "",
        )
        .await;
    assert_eq!(s, 200);
    assert_eq!(u["role"], "admin");
    assert!(!u.to_string().contains("password"));
    let pool = db::connect(&t.state.config.database_url).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let h: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(h.starts_with("$argon2id$"));
    assert_eq!(t.call("POST", "/api/logout", json!({}), &c).await.0, 200);
    assert_eq!(t.call("GET", "/api/me", json!({}), &c).await.0, 401);
    assert_eq!(t.call("GET", "/api/me", json!({}), &admin).await.0, 200);
}
#[tokio::test]
async fn invitations_are_private_expiring_single_use_and_atomic() {
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    assert_eq!(
        t.call("POST", "/api/invites", json!({"hours":24}), &member)
            .await
            .0,
        403
    );
    assert_eq!(
        t.call(
            "POST",
            "/api/register",
            json!({"username":"stranger","password":"strong password 123"}),
            ""
        )
        .await
        .0,
        400
    );
    let (_, v, _) = t
        .call("POST", "/api/invites", json!({"hours":1}), &admin)
        .await;
    let body = json!({"username":"second","password":"strong password 123","invite":v["token"]});
    let (a, b) = tokio::join!(
        t.call("POST", "/api/register", body.clone(), ""),
        t.call("POST", "/api/register", body, "")
    );
    assert_eq!([a.0, b.0].into_iter().filter(|s| *s == 200).count(), 1);
    let (_, v, _) = t
        .call("POST", "/api/invites", json!({"hours":1}), &admin)
        .await;
    sqlx::query("UPDATE invites SET expires_at=0")
        .execute(&t.state.db)
        .await
        .unwrap();
    assert_eq!(
        t.call(
            "POST",
            "/api/register",
            json!({"username":"third","password":"strong password 123","invite":v["token"]}),
            ""
        )
        .await
        .0,
        400
    );
}
#[tokio::test]
async fn channels_crud_permissions_and_validation() {
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    assert_eq!(
        t.call(
            "POST",
            "/api/channels",
            json!({"name":"test","kind":"text"}),
            &member
        )
        .await
        .0,
        403
    );
    assert_eq!(
        t.call(
            "POST",
            "/api/channels",
            json!({"name":" ","kind":"other"}),
            &admin
        )
        .await
        .0,
        400
    );
    let (s, c, _) = t
        .call(
            "POST",
            "/api/channels",
            json!({"name":"test","kind":"text"}),
            &admin,
        )
        .await;
    assert_eq!(s, 200);
    let path = format!("/api/channels/{}", c["id"].as_str().unwrap());
    assert_eq!(
        t.call(
            "PATCH",
            &path,
            json!({"name":"renamed","kind":"text"}),
            &admin
        )
        .await
        .1["name"],
        "renamed"
    );
    assert_eq!(
        t.call(
            "PATCH",
            &path,
            json!({"name":"renamed","kind":"voice"}),
            &admin
        )
        .await
        .0,
        400
    );
    assert_eq!(t.call("DELETE", &path, json!({}), &member).await.0, 403);
    assert_eq!(t.call("DELETE", &path, json!({}), &admin).await.0, 200);
    assert_eq!(t.call("DELETE", &path, json!({}), &admin).await.0, 404);
}
#[tokio::test]
async fn messages_pagination_identity_limits_and_rate() {
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    let (_, channels, _) = t.call("GET", "/api/channels", json!({}), &admin).await;
    let id = channels
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let path = format!("/api/channels/{id}/messages");
    assert_eq!(
        t.call(
            "POST",
            &path,
            json!({"content":"hi","user_id":"forged"}),
            &member
        )
        .await
        .0,
        422
    );
    for content in ["one", "two", "<script>alert(1)</script>"] {
        assert_eq!(
            t.call("POST", &path, json!({"content":content}), &member)
                .await
                .0,
            200
        )
    }
    let (_, items, _) = t
        .call("GET", &(path.clone() + "?limit=2"), json!({}), &admin)
        .await;
    assert_eq!(items.as_array().unwrap().len(), 2);
    assert_eq!(items[0]["username"], "friend");
    let (_, older, _) = t
        .call(
            "GET",
            &format!("{path}?before={}", items[0]["id"]),
            json!({}),
            &admin,
        )
        .await;
    assert_eq!(older.as_array().unwrap().len(), 1);
    assert_eq!(older[0]["content"], "one");
    for content in [" ".into(), "x".repeat(4001)] {
        assert_eq!(
            t.call("POST", &path, json!({"content":content}), &member)
                .await
                .0,
            400
        )
    }
    assert_eq!(
        t.call("GET", &(path.clone() + "?limit=101"), json!({}), &admin)
            .await
            .0,
        400
    );
    for _ in 0..60 {
        t.call("POST", &path, json!({"content":"x"}), &member).await;
    }
    assert_eq!(
        t.call("POST", &path, json!({"content":"x"}), &member)
            .await
            .0,
        429
    );
}
#[tokio::test]
async fn csrf_expired_sessions_and_secure_cookie() {
    let mut t = TestApp::new().await;
    let cookie = t.admin().await;
    assert_eq!(
        t.origin(
            "POST",
            "/api/invites",
            json!({"hours":1}),
            &cookie,
            "https://evil.example"
        )
        .await
        .0,
        403
    );
    sqlx::query("UPDATE sessions SET expires_at=0")
        .execute(&t.state.db)
        .await
        .unwrap();
    assert_eq!(t.call("GET", "/api/me", json!({}), &cookie).await.0, 401);
    let mut cfg = t.state.config.clone();
    cfg.secure = true;
    t.state = AppState::new(t.state.db.clone(), cfg);
    t.app = app(t.state.clone());
    let (s, _, c) = t
        .call(
            "POST",
            "/api/login",
            json!({"username":"admin","password":"strong password 123"}),
            "",
        )
        .await;
    assert_eq!(s, 200);
    assert!(c.contains("; Secure"));
}
#[tokio::test]
async fn media_grants_are_scoped_and_webhooks_verified() {
    use livekit_api::access_token::TokenVerifier;
    let t = TestApp::new().await;
    assert_eq!(
        t.call("POST", "/api/channels/any/join", json!({}), "")
            .await
            .0,
        401
    );
    let jwt = subdisc::media::mint(&t.state, "opaque-room", "opaque-user", "name").unwrap();
    let claims =
        TokenVerifier::with_api_key(&t.state.config.livekit_key, &t.state.config.livekit_secret)
            .verify(&jwt)
            .unwrap();
    assert_eq!(claims.sub, "opaque-user");
    assert_eq!(claims.video.room, "opaque-room");
    assert!(claims.video.room_join);
    assert!(!claims.video.room_admin && !claims.video.room_create && !claims.video.room_list);
    assert!(!claims.video.can_publish_data());
    assert!(claims.exp - claims.nbf <= 60);
    assert_eq!(
        t.call(
            "POST",
            "/api/livekit/webhook",
            json!({"event":"participant_joined"}),
            ""
        )
        .await
        .0,
        401
    );
    let admin = t.admin().await;
    let (_, c, _) = t.call("GET", "/api/channels", json!({}), &admin).await;
    let text = c
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap();
    assert_eq!(
        t.call(
            "POST",
            &format!("/api/channels/{}/join", text["id"].as_str().unwrap()),
            json!({}),
            &admin
        )
        .await
        .0,
        400
    );
}
#[tokio::test]
async fn websocket_auth_events_and_logout_revocation() {
    use futures_util::StreamExt;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Message, client::IntoClientRequest},
    };
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(
        axum::serve(
            listener,
            t.app
                .clone()
                .into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .into_future(),
    );
    let url = format!("ws://{addr}/api/ws");
    let mut req = url.clone().into_client_request().unwrap();
    req.headers_mut()
        .insert("origin", "http://localhost".parse().unwrap());
    assert!(connect_async(req).await.is_err());
    let mut req = url.into_client_request().unwrap();
    req.headers_mut()
        .insert("origin", "http://localhost".parse().unwrap());
    req.headers_mut().insert("cookie", member.parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    let first = ws.next().await.unwrap().unwrap();
    assert!(first.to_text().unwrap().contains("snapshot"));
    let (_, c, _) = t.call("GET", "/api/channels", json!({}), &admin).await;
    let id = c
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    t.call(
        "POST",
        &format!("/api/channels/{id}/messages"),
        json!({"content":"realtime hello"}),
        &admin,
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(Ok(Message::Text(v))) = ws.next().await
                && v.contains("realtime hello")
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    t.call("POST", "/api/logout", json!({}), &member).await;
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while let Some(msg) = ws.next().await {
            if matches!(msg, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
        }
    })
    .await
    .unwrap();
    server.abort();
}
