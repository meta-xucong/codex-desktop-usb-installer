CREATE TABLE IF NOT EXISTS licenses (
 product TEXT NOT NULL,
 installation_id TEXT NOT NULL,
 key_id TEXT NOT NULL,
 license_signature_sha256 TEXT NOT NULL,
 fingerprint_digest TEXT NOT NULL,
 usb_asset_id TEXT NOT NULL DEFAULT '',
 status TEXT NOT NULL CHECK(status IN ('active','revoked')),
 state_version INTEGER NOT NULL CHECK(state_version > 0),
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL,
 revoked_at TEXT,
 revoke_reason TEXT NOT NULL DEFAULT '',
 PRIMARY KEY(product, installation_id)
);

CREATE TABLE IF NOT EXISTS audit_events (
 sequence INTEGER NOT NULL PRIMARY KEY,
 event_id TEXT NOT NULL UNIQUE,
 installation_id TEXT NOT NULL,
 actor TEXT NOT NULL,
 action TEXT NOT NULL CHECK(action IN ('import','asset_bind','revoke','reactivate')),
 old_status TEXT,
 new_status TEXT NOT NULL,
 old_version INTEGER,
 new_version INTEGER NOT NULL,
 reason TEXT NOT NULL DEFAULT '',
 occurred_at TEXT NOT NULL,
 previous_hash TEXT NOT NULL,
 event_hash TEXT NOT NULL UNIQUE
);
