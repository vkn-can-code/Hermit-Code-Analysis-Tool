# Hermit — Intelligent Code Analysis

**Hermit reviews pull requests for you, and runs entirely on your own computer.**

When someone changes code, the change often affects more than the lines they touched. Other functions call the changed code, new security holes can slip in, and some tests should be re-run while others don't matter. Hermit works all of that out automatically and gives the reviewer a short, clear report.

No code is sent to the cloud. The AI model runs locally.

> Semester 6 major project · Full documentation: [`docs/Hermit_Features_and_Working.pdf`](docs/Hermit_Features_and_Working.pdf)

---

## What Hermit tells you

For every pull request, Hermit answers five questions:

| Question | How Hermit answers it |
|---|---|
| **What changed?** | Finds exactly which functions and classes were edited, and whether their inputs (signature) or complexity changed. |
| **Is it secure?** | Scans the changed lines for vulnerabilities such as SQL injection, command injection, XSS and leaked passwords or keys. It labels each one **new** (introduced) or **fixed** (removed by this PR). |
| **What could break?** | Follows the code to find every function that calls the changed code (the *blast radius*), up to 3 levels away. It highlights risky areas like login or payments. |
| **Which tests should run?** | Picks only the existing tests that actually reach the changed code and gives you the exact command to run each one. It also lists changed code that no test covers. |
| **How risky is it?** | Gives a score from **0 to 100** (low / medium / high) and shows exactly where the points came from. |

On top of that, a **local AI reviewer** writes a plain-English summary, line-by-line comments and suggested tests.

## Example

Real output for a PR that fixes a SQL-injection bug (try it with `make demo`):

```text
🟢 Hermit risk: LOW (17 / 100)

Security        SQL injection (CWE-89) in services/billing.py   ✅ fixed by this PR
What could break subscription_handler, monthly_total, charge_handler   (payment code)
New dependency  get_user_subscription now calls invalidate_cache
Tests to run    pytest 'tests/test_billing.py::test_get_user_subscription'
                pytest 'tests/test_billing.py::test_monthly_total'      (2 of 3 tests)
AI review       2 comments, all checked against the code
```

## How it works

```
Pull request ─► 1 Read the diff ─► 2 Security scan ─► 3 Find affected callers
                                                              │
     Report  ◄─ 6 AI review ◄─ 5 Risk score ◄─ 4 Pick tests ◄─┘
```

1. **Read the diff.** Git shows which lines changed. Hermit parses the code with tree-sitter to see which *function* each line belongs to. Supported languages: Python, JavaScript, TypeScript and Rust.
2. **Security scan.** [Semgrep](https://semgrep.dev) checks the old and new versions of each changed file with 18 built-in rules, plus a scanner for hard-coded secrets.
3. **Find affected callers.** Hermit builds a map of which function calls which across the whole repository (a *call graph*). It walks backwards from the changed code to find everything that depends on it.
4. **Pick tests.** It keeps only the tests whose call paths lead to the changed code.
5. **Risk score.** Six factors are added up: new security issues, blast radius, breaking changes, added complexity, size of the change and untested code. Each factor has a maximum.
6. **AI review.** A local model (Qwen 2.5 Coder, run with [Ollama](https://ollama.com)) writes the summary and comments.

**Why trust the AI part?** Steps 1–5 are plain, repeatable analysis; the AI only explains them. It is given only verified facts, it must answer in a fixed format, and any comment about lines outside the change, or about code that doesn't exist, is thrown away automatically. If the AI is slow or not running, you still get the full report.

## Ways to use it

- **Command line:** `hermit analyze` prints the report in your terminal.
- **GitHub App:** on every pull request Hermit adds a pass/fail check, one summary comment (updated on each push) and inline review comments.
- **GitHub Action:** runs inside your CI with no server needed, and can fail the build when risk is too high.
- **Web dashboard:** analyze any local repository and watch each step run live.

---

## Getting started

**1. Install the tools** (macOS, once):

```sh
brew install rustup semgrep postgresql@16 ollama
rustup default stable
ollama pull qwen2.5-coder:7b && ollama pull nomic-embed-text
```

**2. Build and check your setup:**

```sh
make build
./target/release/hermit doctor      # confirms git, Semgrep, Ollama and PostgreSQL are ready
```

**3. Try it:**

```sh
make demo                           # the SQL-injection example above
./target/release/hermit analyze --repo ~/code/my-app --base main --head my-branch
```

Useful options for `hermit analyze`:

| Option | What it does |
|---|---|
| `--no-ai` | skip the AI review (faster) |
| `--format json` | machine-readable output |
| `--out report.md` | save to a file |
| `--fail-on high` | exit with code 2 if risk reaches that level (for CI) |
| `--config hermit.toml` | use a custom settings file |

### Dashboard and GitHub App

```sh
createdb hermit                         # database for saved reports
(cd web && npm ci && npm run build)     # build the dashboard
cp .env.example .env                    # add GitHub App details (optional)
scripts/dev.sh                          # open http://127.0.0.1:8080
```

You don't need GitHub to use the dashboard: type a local repository path into the form and press **Run Hermit**.

<details>
<summary><b>Connecting the GitHub App (step by step)</b></summary>

1. On GitHub go to **Settings → Developer settings → GitHub Apps → New GitHub App**.
2. **Webhook URL:** GitHub needs to reach your computer. Create a free channel at <https://smee.io>, use its URL here, and start Hermit with `SMEE_URL=https://smee.io/<id> scripts/dev.sh`. Choose a webhook secret.
3. **Permissions:** Checks, Pull requests and Issues set to *read & write*; Contents set to *read*. **Events:** *Pull request*.
4. Generate a private key, then fill in `GITHUB_APP_ID`, `GITHUB_PRIVATE_KEY_PATH` and `GITHUB_WEBHOOK_SECRET` in `.env`.
5. Install the App on a repository and open a pull request.

</details>

<details>
<summary><b>REST API</b></summary>

| Method | Path | Purpose |
|---|---|---|
| POST | `/webhooks/github` | receives GitHub events (signature-checked) |
| POST | `/api/analyses` | analyze a local repo: `{"repo_path": "...", "base": "main", "head": "feature"}` |
| GET | `/api/analyses` | list recent analyses |
| GET | `/api/analyses/{id}` | one analysis: status, full report, markdown |
| GET | `/api/repos` | repositories and their latest score |
| GET | `/api/events` | live progress (Server-Sent Events) |
| GET | `/api/health` | status of database, Ollama, Semgrep, GitHub |

</details>

### Settings

Add a `hermit.toml` file to the repository you analyze to change the defaults. You can set:

- folders to ignore;
- how far to search for callers;
- which words mark sensitive code (e.g. `auth`, `payment`);
- the risk weights and thresholds;
- the AI model and timeout;
- when the GitHub check should fail.

Every option is explained in [`hermit.example.toml`](hermit.example.toml). You can also use environment variables: `HERMIT_OLLAMA_URL`, `HERMIT_MODEL`, and `HERMIT_AI=0` to turn the AI off.

---

## Does it work? (testing)

`make bench` runs Hermit on six small example projects. Each one has a known correct answer, and Hermit's output is compared against it.

| Example | What it checks |
|---|---|
| `python_sqli` | spots a fixed SQL injection, a new dependency, 3 callers, 2 relevant tests |
| `js_xss` | spots a new cross-site-scripting bug in TypeScript |
| `js_command_injection` | spots a command-injection bug and a leaked token |
| `rust_unsafe` | spots new `unsafe` Rust code; finds tests hidden inside macros |
| `python_signature_change` | flags a change that will break existing callers |
| `clean_pr` | rates a safe change as low risk |

Hermit currently gets all six right (results in `fixtures/bench-results.json`). These examples were written to exercise each feature, so they show that the features work, not how accurate Hermit is on any real-world codebase. On a real project (psf/requests, about 12,000 lines), mapping the whole codebase took 36 ms, and Hermit picked 107 of 347 tests to run.

## Project layout

```
crates/            the Rust code, one folder per part of the pipeline
  hermit-diff        reads the git diff
  hermit-lang        parses code (functions, calls, imports, complexity)
  hermit-graph       builds the call graph and finds affected callers
  hermit-security    runs Semgrep and the secret scanner
  hermit-tests       picks tests and builds the commands to run them
  hermit-risk        calculates the risk score
  hermit-ai          talks to the local AI model and checks its answers
  hermit-engine      runs all the steps in order and writes the report
  hermit-store       saves results in PostgreSQL; job queue
  hermit-github      GitHub App: webhooks, checks, comments
  hermit-server      web server, API and live updates
  hermit-cli         the `hermit` command
  hermit-core        data types shared by all of the above
web/               the dashboard (Next.js)
action/            the GitHub Action
fixtures/          the six test examples
docs/              PDF documentation
```

## Known limits

- **Some calls can't be traced.** In Python and JavaScript, code that decides at runtime what to call (dynamic dispatch, monkey-patching) is invisible to the call graph. When a function name is too common to be sure which one is meant, Hermit skips it rather than guessing.
- **The AI takes time.** The local model takes about 10–20 seconds per pull request on an Apple M4 Pro. Everything else takes about 1–2 seconds.
- **The AI summary isn't fact-checked yet.** Comments are verified, but the summary text could still contain a mistake. That is why the verified facts are always shown next to it.
- **Some extra Semgrep rule packs need the internet.** The built-in rules work offline.
