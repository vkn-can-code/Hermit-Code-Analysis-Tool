# Hermit — Intelligent Code Analysis

**AI-assisted pull request review that runs entirely on-premise.** Hermit combines deterministic analysis (tree-sitter ASTs, Semgrep, a repo-wide call graph) with a local LLM (Qwen 2.5 Coder via Ollama) to tell reviewers what a change touches, whether it is insecure, what it can break, which tests to run, and how risky it is. No source code leaves your machine.

> Semester 6 major project. Full technical documentation: [`docs/Hermit_Features_and_Working.pdf`](docs/Hermit_Features_and_Working.pdf).

## Features

Hermit reviews pull requests the way a careful senior engineer would:

1. **Diff + AST**: maps every changed line to the function or class it belongs to (tree-sitter: Python, JavaScript, TypeScript/TSX, Rust).
2. **Security**: Semgrep with 18 bundled rules (SQL/command/code injection, XSS, unsafe deserialization, weak crypto, TLS bypass, unsafe Rust) plus built-in secret detection. Each finding is labelled **new** (introduced) or **fixed** (removed by the PR).
3. **Blast radius**: a repo-wide call graph shows every caller the change can break, how far away it is, and whether it sits in sensitive code (auth, payments, …). New dependencies are flagged.
4. **Test intelligence**: selects only the existing tests whose call paths reach the change, with ready-to-run commands (`pytest`, `jest`/`vitest`/`mocha`, `cargo test`), and lists changed functions no test covers.
5. **Risk score**: an explainable 0–100 score from six capped factors (security, blast radius, breaking change, complexity, size, test gap).
6. **AI review**: a local `qwen2.5-coder:7b` model (Ollama) writes a summary, inline comments and candidate tests. Its prompt contains only verified facts, its output must match a JSON schema, and a validator drops comments that point outside the diff or cite code that does not exist.
7. **Delivery**: CLI, GitHub App (Check Run, summary comment, inline review), GitHub Action, REST API with live progress, and a web dashboard.

## Sample output

The SQL-injection fix from `fixtures/python_sqli` (run `make demo`):

```text
## 🟢 Hermit risk: LOW (17 / 100)
1 file · +6 / -4 · 1 symbol changed · 3 impacted callers · 0 new / 1 fixed finding · 2 tests selected

Security       🛑 SQL injection (CWE-89) services/billing.py:7 ✅ fixed
Blast radius   subscription_handler (1) · monthly_total (1) · charge_handler (2) — all sensitive
New dependency get_user_subscription now calls services/cache.py::invalidate_cache
Tests          pytest 'tests/test_billing.py::test_get_user_subscription'
               pytest 'tests/test_billing.py::test_monthly_total'
AI review      2 inline comments verified against the diff, 0 dropped (qwen2.5-coder:7b, ~12 s)
```

## Quick start

```sh
brew install rustup semgrep postgresql@16 ollama   # once
rustup default stable
ollama pull qwen2.5-coder:7b && ollama pull nomic-embed-text

make build
./target/release/hermit doctor                     # checks git, semgrep, ollama, postgres
make demo                                          # the SQL-injection fix PR from the slides
./target/release/hermit analyze --repo ~/code/app --base main --head my-branch
```

`hermit analyze` options: `--format markdown|json`, `--out FILE`, `--no-ai`, `--fail-on low|medium|high` (exit code 2), `--config hermit.toml`.

## Server, dashboard and GitHub App

```sh
createdb hermit
(cd web && npm ci && npm run build)                # dashboard -> web/out
cp .env.example .env                               # fill in GitHub App values (optional)
scripts/dev.sh                                     # http://127.0.0.1:8080
```

Without GitHub credentials you can still analyze local repositories from the dashboard form or the API:

```sh
curl -X POST localhost:8080/api/analyses -H 'content-type: application/json' \
     -d '{"repo_path":"/path/to/repo","base":"main","head":"feature"}'
```

### GitHub App setup

1. GitHub → Settings → Developer settings → GitHub Apps → **New GitHub App**.
2. Webhook URL: a tunnel to your machine, e.g. create a channel at <https://smee.io> and run `SMEE_URL=https://smee.io/<id> scripts/dev.sh`. Set a webhook secret.
3. Permissions: *Checks* read & write, *Pull requests* read & write, *Contents* read, *Issues* read & write. Events: *Pull request*.
4. Generate a private key and put `GITHUB_APP_ID`, `GITHUB_PRIVATE_KEY_PATH` and `GITHUB_WEBHOOK_SECRET` in `.env`.
5. Install the App on a repository and open a pull request. Hermit posts a Check Run, one summary comment (updated on every push), and an inline review.

### API

| Method | Path | Purpose |
|---|---|---|
| POST | `/webhooks/github` | GitHub webhook (HMAC-SHA256 verified) |
| POST | `/api/analyses` | queue a local repository analysis |
| GET | `/api/analyses` | recent analyses (`?repo=`, `?limit=`) |
| GET | `/api/analyses/{id}` | status, full JSON report and markdown |
| GET | `/api/repos` | repositories with latest score |
| GET | `/api/events` | Server-Sent Events: live stage progress |
| GET | `/api/health` | database / Ollama / Semgrep / GitHub status |

## Configuration

Put a `hermit.toml` in the analyzed repository. See [`hermit.example.toml`](hermit.example.toml): ignore paths, graph depth, sensitive keywords, Semgrep packs, risk weights, AI model and timeouts, and Check Run fail threshold. Environment overrides: `HERMIT_OLLAMA_URL`, `HERMIT_MODEL`, `HERMIT_AI=0`.

## Evaluation

```sh
make bench      # fixtures/<name>/{base,head,expected.toml}
```

| Fixture | What it tests |
|---|---|
| `python_sqli` | SQL injection *fixed*, new cache dependency, 3 callers, 2 tests |
| `js_xss` | new `innerHTML` XSS in TypeScript, vitest selection |
| `js_command_injection` | `execSync` with template string + leaked GitHub token |
| `rust_unsafe` | new `unsafe` block, calls inside `assert_eq!` macros |
| `python_signature_change` | breaking signature change with live callers |
| `clean_pr` | low-risk addition with its own test |

The current result is in `fixtures/bench-results.json`. These fixtures were written by the author to exercise each feature. They show the pipeline works end to end; they are not an independent accuracy benchmark.

## Architecture

```
crates/
  hermit-core      shared types (ChangedFile, Symbol, Finding, RiskReport, AnalysisReport)
  hermit-diff      git2: base..head diff, line ranges, file contents at each revision
  hermit-lang      tree-sitter parsing: symbols, calls, imports, complexity, diff->symbol mapping
  hermit-graph     parallel repo index + call graph, blast radius
  hermit-security  Semgrep runner + bundled rules + builtin secret/fallback rules
  hermit-tests     test discovery, selection, runner commands
  hermit-risk      explainable risk score
  hermit-ai        Ollama client, retrieval, grounded prompt, validator
  hermit-engine    pipeline orchestration, hermit.toml, markdown report
  hermit-store     PostgreSQL: analyses, job queue (SKIP LOCKED), embedding cache
  hermit-github    webhook verification, App JWT/tokens, Check Runs, comments, checkout
  hermit-server    axum API, SSE, dashboard hosting, workers
  hermit-cli       `hermit analyze | serve | bench | doctor`
web/               Next.js + Tailwind dashboard (static export)
action/            GitHub Action
fixtures/          labelled benchmark cases
```

## Limits

- Static call resolution in Python and JavaScript is approximate: dynamic dispatch, monkey-patching and reflection are invisible. Ambiguous names (more than 3 definitions) are skipped rather than guessed.
- The 7B model takes about 10–20 s per PR on an M4 Pro. If it is slow or unavailable, the deterministic report is still produced.
- Registry Semgrep packs (`p/...`) need network access. The bundled rules work offline.
