CREATE TABLE users (
 id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE,
 password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK(role IN ('admin','member')),
 created_at INTEGER NOT NULL
);
CREATE TABLE sessions (
 token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE TABLE invites (
 token_hash TEXT PRIMARY KEY, created_by TEXT NOT NULL REFERENCES users(id),
 expires_at INTEGER NOT NULL, used_by TEXT REFERENCES users(id)
);
CREATE TABLE channels (
 id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('text','voice')),
 created_at INTEGER NOT NULL
);
CREATE TABLE messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT, channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
 user_id TEXT NOT NULL REFERENCES users(id), content TEXT NOT NULL CHECK(length(content) BETWEEN 1 AND 4000),
 created_at INTEGER NOT NULL
);
CREATE INDEX messages_channel_cursor ON messages(channel_id,id DESC);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
