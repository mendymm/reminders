//! Schema migrations, applied in order. Append new ones. Never edit one that has
//! shipped: databases already past it would not run it again.
//!
//! The SQL lives in Rust strings, not `.sql` files, so builds that copy only
//! Rust sources still see it.

pub(super) const MIGRATIONS: &[&str] = &[V1];

/// The first schema.
///
/// Timestamps are Unix milliseconds. Ids are 16-byte blobs. Local dates and
/// times are ISO 8601 text. A reminder's plan is JSON, written and read through
/// the model's serde implementations.
const V1: &str = r"
CREATE TABLE reminders (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    title       TEXT    NOT NULL CHECK (trim(title) <> ''),
    notes       TEXT    NOT NULL,
    plan        TEXT    NOT NULL,
    archived_at INTEGER,
    revision    INTEGER NOT NULL CHECK (revision >= 1),
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    deleted_at  INTEGER
) STRICT;

CREATE TABLE progress (
    reminder_id      BLOB    NOT NULL PRIMARY KEY REFERENCES reminders (id),
    kind             TEXT    NOT NULL CHECK (kind IN ('once', 'cadence')),
    done_at          INTEGER,
    through          TEXT,
    handled          INTEGER CHECK (handled >= 0),
    previous_through TEXT,
    previous_handled INTEGER CHECK (previous_handled >= 0),
    last_resolution  BLOB    CHECK (last_resolution IS NULL OR length(last_resolution) = 16),
    revision         INTEGER NOT NULL CHECK (revision >= 1),
    updated_at       INTEGER NOT NULL,
    CHECK (
        (kind = 'once'
            AND through IS NULL AND handled IS NULL
            AND previous_through IS NULL AND previous_handled IS NULL)
        OR
        (kind = 'cadence'
            AND done_at IS NULL AND handled IS NOT NULL
            AND (previous_handled IS NOT NULL OR previous_through IS NULL))
    )
) STRICT;

CREATE TABLE resolutions (
    id          BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    reminder_id BLOB    NOT NULL REFERENCES reminders (id),
    resolved_at INTEGER NOT NULL,
    outcome     TEXT    NOT NULL CHECK (outcome IN ('done', 'skipped')),
    slot        TEXT,
    note        TEXT,
    deleted_at  INTEGER,
    CHECK (outcome <> 'skipped' OR slot IS NOT NULL)
) STRICT;

CREATE INDEX resolutions_by_reminder
    ON resolutions (reminder_id, resolved_at DESC, id DESC);
";
