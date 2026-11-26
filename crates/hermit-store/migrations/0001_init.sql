-- Hermit schema v1
CREATE TABLE IF NOT EXISTS repositories (
    id              BIGSERIAL PRIMARY KEY,
    full_name       TEXT NOT NULL UNIQUE,          -- owner/name, or local:<path>
    installation_id BIGINT,                        -- GitHub App installation (NULL for local repos)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS analyses (
    id           BIGSERIAL PRIMARY KEY,
    repo_id      BIGINT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    pr_number    INT,
    title        TEXT NOT NULL DEFAULT '',
    author       TEXT NOT NULL DEFAULT '',
    base_ref     TEXT NOT NULL,
    head_ref     TEXT NOT NULL,
    head_sha     TEXT NOT NULL DEFAULT '',
    status       TEXT NOT NULL DEFAULT 'queued',   -- queued | running | done | failed
    stage        TEXT NOT NULL DEFAULT 'queued',   -- current pipeline stage
    risk_score   INT,
    risk_tier    TEXT,
    report       JSONB,
    error        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS analyses_repo_idx ON analyses (repo_id, created_at DESC);

-- Durable work queue (claimed with FOR UPDATE SKIP LOCKED).
CREATE TABLE IF NOT EXISTS jobs (
    id          BIGSERIAL PRIMARY KEY,
    analysis_id BIGINT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
    payload     JSONB NOT NULL,
    status      TEXT NOT NULL DEFAULT 'queued',    -- queued | running | done | failed
    attempts    INT NOT NULL DEFAULT 0,
    run_after   TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_at   TIMESTAMPTZ,
    last_error  TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS jobs_ready_idx ON jobs (status, run_after, id);

-- Embedding cache keyed by sha256(model, text).
CREATE TABLE IF NOT EXISTS embeddings (
    key        TEXT PRIMARY KEY,
    vec        REAL[] NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
