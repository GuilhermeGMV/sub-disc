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
        Self::with_livekit("http://127.0.0.1:1".into()).await
    }
    async fn with_livekit(livekit_internal: String) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            database_url: format!("sqlite://{}", dir.path().join("test.db").display()),
            origin: "http://localhost".into(),
            bind: "127.0.0.1:0".into(),
            secure: false,
            setup_key: "a-safe-setup-test-key".into(),
            livekit_url: "ws://localhost:7880".into(),
            livekit_internal,
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
async fn session_expiry(t: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT expires_at FROM sessions")
        .fetch_one(&t.state.db)
        .await
        .unwrap()
}
#[tokio::test]
async fn sessions_renew_while_in_use() {
    let t = TestApp::new().await;
    let cookie = t.admin().await;
    let token = cookie.split(';').next().unwrap();
    let first = session_expiry(&t).await;
    let (s, _, c) = t.call("GET", "/api/me", json!({}), &cookie).await;
    assert_eq!(s, 200);
    assert!(c.starts_with(token) && c.contains("HttpOnly"));
    // Sessão recente: nenhuma escrita a cada pedido.
    assert_eq!(session_expiry(&t).await, first);
    sqlx::query("UPDATE sessions SET expires_at=?")
        .bind(subdisc::now() + 10 * 86400)
        .execute(&t.state.db)
        .await
        .unwrap();
    let (s, _, c) = t.call("GET", "/api/me", json!({}), &cookie).await;
    assert_eq!(s, 200);
    assert!(session_expiry(&t).await >= subdisc::now() + 30 * 86400 - 5);
    let age: i64 = c
        .split("Max-Age=")
        .nth(1)
        .and_then(|v| v.split(';').next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(age > 29 * 86400);
    assert!(c.starts_with(token));
}
#[tokio::test]
async fn invitations_are_private_expiring_and_atomic() {
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

#[tokio::test]
async fn admin_accounts_permissions_roles_and_session_revocation() {
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    let (_, me, _) = t.call("GET", "/api/me", json!({}), &member).await;
    let id = me["id"].as_str().unwrap();
    let path = format!("/api/users/{id}");
    let input = json!({"username":"created","password":"strong password 123","role":"member"});
    assert_eq!(t.call("POST", "/api/users", input.clone(), "").await.0, 401);
    for (method, route, body) in [
        ("POST", "/api/users".to_string(), input.clone()),
        ("PATCH", path.clone(), input.clone()),
        ("DELETE", path.clone(), json!({})),
        ("POST", format!("{path}/disconnect"), json!({})),
        (
            "POST",
            "/api/channels".into(),
            json!({"name":"forbidden","kind":"voice"}),
        ),
    ] {
        assert_eq!(t.call(method, &route, body, &member).await.0, 403);
    }
    assert_eq!(
        t.call(
            "POST",
            "/api/users",
            json!({"username":"invalid","password":"strong password 123","role":"owner"}),
            &admin
        )
        .await
        .0,
        400
    );
    assert_eq!(
        t.call(
            "POST",
            "/api/users",
            json!({"username":"invalid","password":"short","role":"member"}),
            &admin
        )
        .await
        .0,
        400
    );
    let (status, created, _) = t.call("POST", "/api/users", input.clone(), &admin).await;
    assert_eq!(status, 200);
    assert_eq!(created["role"], "member");
    assert!(created.get("password_hash").is_none());
    assert_eq!(t.call("POST", "/api/users", input, &admin).await.0, 409);
    let created_path = format!("/api/users/{}", created["id"].as_str().unwrap());
    assert_eq!(
        t.call(
            "PATCH",
            &created_path,
            json!({"username":"renamed","role":"admin"}),
            &admin
        )
        .await
        .0,
        200
    );
    let (status, user, created_cookie) = t
        .call(
            "POST",
            "/api/login",
            json!({"username":"renamed","password":"strong password 123"}),
            "",
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(user["role"], "admin");
    assert_eq!(
        t.call(
            "PATCH",
            &created_path,
            json!({"username":"renamed","role":"member","password":"replacement password 123"}),
            &admin
        )
        .await
        .0,
        200
    );
    assert_eq!(
        t.call("GET", "/api/me", json!({}), &created_cookie).await.0,
        401
    );
    assert_eq!(
        t.call(
            "POST",
            "/api/login",
            json!({"username":"renamed","password":"replacement password 123"}),
            ""
        )
        .await
        .0,
        200
    );
    assert_eq!(
        t.call(
            "PATCH",
            "/api/users/missing",
            json!({"username":"valid","role":"member"}),
            &admin
        )
        .await
        .0,
        404
    );
    assert_eq!(
        t.call("DELETE", &created_path, json!({}), &admin).await.0,
        200
    );
    let (_, me, _) = t.call("GET", "/api/me", json!({}), &admin).await;
    let admin_path = format!("/api/users/{}", me["id"].as_str().unwrap());
    assert_eq!(
        t.call(
            "PATCH",
            &admin_path,
            json!({"username":"admin","role":"member"}),
            &admin
        )
        .await
        .0,
        400
    );
    assert_eq!(
        t.call("DELETE", &admin_path, json!({}), &admin).await.0,
        400
    );
}

#[tokio::test]
async fn admin_disconnect_and_delete_account_with_references() {
    use axum::{body::Bytes, extract::Path, routing::post};
    use prost::Message;
    use std::sync::{Arc, Mutex};
    let removed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let recorded = removed.clone();
    let mock = Router::new().route(
        "/twirp/livekit.RoomService/{method}",
        post(move |Path(method): Path<String>, body: Bytes| {
            let recorded = recorded.clone();
            async move {
                match method.as_str() {
                    "ListRooms" | "ListParticipants" => vec![],
                    "CreateRoom" => {
                        let request = livekit_protocol::CreateRoomRequest::decode(body).unwrap();
                        livekit_protocol::Room {
                            name: request.name,
                            ..Default::default()
                        }
                        .encode_to_vec()
                    }
                    "RemoveParticipant" => {
                        let request =
                            livekit_protocol::RoomParticipantIdentity::decode(body).unwrap();
                        recorded
                            .lock()
                            .unwrap()
                            .push(json!({"room":request.room,"identity":request.identity}));
                        vec![]
                    }
                    other => panic!("Unexpected LiveKit method: {other}"),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let t = TestApp::with_livekit(url).await;
    let admin = t.admin().await;
    let member = t.member(&admin).await;
    let (_, me, _) = t.call("GET", "/api/me", json!({}), &member).await;
    let id = me["id"].as_str().unwrap();
    let path = format!("/api/users/{id}");
    let (_, state, _) = t.call("GET", "/api/state", json!({}), &admin).await;
    let text = state["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "text")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let voice = state["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "voice")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(
        t.call(
            "POST",
            &format!("/api/channels/{text}/messages"),
            json!({"content":"to be removed"}),
            &member
        )
        .await
        .0,
        200
    );
    assert_eq!(
        t.call(
            "POST",
            &format!("/api/channels/{voice}/join"),
            json!({}),
            &member
        )
        .await
        .0,
        200
    );
    let mut events = t.state.events.subscribe();
    assert_eq!(
        t.call("POST", &format!("{path}/disconnect"), json!({}), &admin)
            .await
            .0,
        200
    );
    assert!(!t.state.voice.lock().await.contains_key(id));
    assert!(
        matches!(events.recv().await.unwrap(), subdisc::websocket::Event::VoiceRemoved { user_id } if user_id == id)
    );
    // Kicking revokes only the call; the account may join again.
    assert_eq!(t.call("GET", "/api/me", json!({}), &member).await.0, 200);
    assert_eq!(
        t.call(
            "POST",
            &format!("/api/channels/{voice}/join"),
            json!({}),
            &member
        )
        .await
        .0,
        200
    );
    assert_eq!(
        t.call(
            "PATCH",
            &path,
            json!({"username":"friend","role":"admin"}),
            &admin
        )
        .await
        .0,
        200
    );
    assert_eq!(
        t.call("POST", "/api/invites", json!({"hours":24}), &member)
            .await
            .0,
        200
    );
    assert_eq!(t.call("DELETE", &path, json!({}), &admin).await.0, 200);
    assert!(!t.state.voice.lock().await.contains_key(id));
    assert_eq!(t.call("GET", "/api/me", json!({}), &member).await.0, 401);
    assert_eq!(
        t.call(
            "GET",
            &format!("/api/channels/{text}/messages"),
            json!({}),
            &admin
        )
        .await
        .1,
        json!([])
    );
    assert_eq!(t.call("DELETE", &path, json!({}), &admin).await.0, 404);
    let calls = removed.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| {
        call["room"] == voice
            && call["identity"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{id}:"))
    }));
    server.abort();
}

#[tokio::test]
async fn concurrent_demotions_preserve_an_administrator() {
    let t = TestApp::new().await;
    let first = t.admin().await;
    let (_, second_user, _) = t
        .call(
            "POST",
            "/api/users",
            json!({"username":"secondadmin","password":"strong password 123","role":"admin"}),
            &first,
        )
        .await;
    let (_, _, second) = t
        .call(
            "POST",
            "/api/login",
            json!({"username":"secondadmin","password":"strong password 123"}),
            "",
        )
        .await;
    let (_, first_user, _) = t.call("GET", "/api/me", json!({}), &first).await;
    let first_path = format!("/api/users/{}", first_user["id"].as_str().unwrap());
    let second_path = format!("/api/users/{}", second_user["id"].as_str().unwrap());
    let (a, b) = tokio::join!(
        t.call(
            "PATCH",
            &first_path,
            json!({"username":"admin","role":"member"}),
            &first
        ),
        t.call(
            "PATCH",
            &second_path,
            json!({"username":"secondadmin","role":"member"}),
            &second
        ),
    );
    assert!((a.0 == 200 && b.0 == 400) || (a.0 == 400 && b.0 == 200));
    let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin'")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert_eq!(admins, 1);
}

#[tokio::test]
async fn invites_have_fixed_expiry_and_five_atomic_uses() {
    let t = TestApp::new().await;
    let admin = t.admin().await;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (status, invite, _) = t
        .call("POST", "/api/invites", json!({"hours":168}), &admin)
        .await;
    assert_eq!(status, 200);
    let expiry = invite["expires_at"].as_i64().unwrap();
    assert!(
        expiry >= before + 86400
            && expiry
                <= std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64
                    + 86400
    );
    assert_eq!(invite["max_uses"], 5);
    // Duplicate username fails inside the transaction and must not spend a use.
    assert_eq!(
        t.call(
            "POST",
            "/api/register",
            json!({"username":"admin", "password":"strong password 123", "invite":invite["token"]}),
            ""
        )
        .await
        .0,
        409
    );
    let register = |index| {
        t.call("POST", "/api/register", json!({"username":format!("guest{index}"), "password":"strong password 123", "invite":invite["token"]}), "")
    };
    // Stay below the password-hashing concurrency limit while racing the
    // remaining two uses with three distinct registrations.
    for index in 1..=3 {
        assert_eq!(register(index).await.0, 200);
    }
    let results = tokio::join!(register(4), register(5), register(6));
    let statuses = [results.0.0, results.1.0, results.2.0];
    assert_eq!(statuses.iter().filter(|s| **s == 200).count(), 2);
    assert_eq!(statuses.iter().filter(|s| **s == 400).count(), 1);
    let row: (i64, i64) = sqlx::query_as("SELECT use_count, expires_at FROM invites")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert_eq!(row, (5, expiry));
    let (status, fresh, _) = t.call("POST", "/api/invites", json!({}), &admin).await;
    assert_eq!(status, 200);
    sqlx::query("UPDATE invites SET expires_at=0 WHERE use_count=0")
        .execute(&t.state.db)
        .await
        .unwrap();
    assert_eq!(t.call("POST", "/api/register", json!({"username":"expired", "password":"strong password 123", "invite":fresh["token"]}), "").await.0, 400);
    let count: i64 = sqlx::query_scalar("SELECT use_count FROM invites WHERE expires_at=0")
        .fetch_one(&t.state.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
