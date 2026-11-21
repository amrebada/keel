//! auth-core tables. Portable SQL: VARCHAR keys, BIGINT times (unix seconds)
//! and flags (0/1), so the same text runs on Postgres, MySQL and SQLite.

use keel_sdk::Migration;

pub fn all() -> Vec<Migration> {
    vec![
        Migration::new(
            "001_init",
            "CREATE TABLE {ns}users (
            id VARCHAR(36) NOT NULL PRIMARY KEY,
            email VARCHAR(255),
            email_verified BIGINT NOT NULL DEFAULT 0,
            password_hash VARCHAR(255),
            name VARCHAR(255),
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        );
        CREATE UNIQUE INDEX {ns}users_email ON {ns}users (email);
        CREATE TABLE {ns}sessions (
            id VARCHAR(36) NOT NULL PRIMARY KEY,
            user_id VARCHAR(36) NOT NULL,
            refresh_hash VARCHAR(64) NOT NULL,
            created_at BIGINT NOT NULL,
            expires_at BIGINT NOT NULL,
            revoked_at BIGINT
        );
        CREATE INDEX {ns}sessions_user ON {ns}sessions (user_id);
        CREATE UNIQUE INDEX {ns}sessions_refresh ON {ns}sessions (refresh_hash);
        CREATE TABLE {ns}tokens (
            hash VARCHAR(64) NOT NULL PRIMARY KEY,
            purpose VARCHAR(20) NOT NULL,
            user_id VARCHAR(36) NOT NULL,
            expires_at BIGINT NOT NULL,
            used_at BIGINT
        );
        CREATE INDEX {ns}tokens_user ON {ns}tokens (user_id)",
        ),
        // Refresh tokens that were rotated away. Presenting one again means it
        // leaked, so the whole session is revoked.
        Migration::new(
            "002_refresh_reuse",
            "CREATE TABLE {ns}used_refresh (
            hash VARCHAR(64) NOT NULL PRIMARY KEY,
            session_id VARCHAR(36) NOT NULL,
            used_at BIGINT NOT NULL
        );
        CREATE INDEX {ns}used_refresh_at ON {ns}used_refresh (used_at)",
        ),
    ]
}
