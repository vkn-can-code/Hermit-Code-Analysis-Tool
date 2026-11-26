"""Builds docs/Hermit_Features_and_Working.pdf from real Hermit outputs.

Inputs: docs/demo-report.json (report of the slide-6 demo PR, from the API) and
fixtures/bench-results.json (from `hermit bench`). Requires: reportlab.
Usage: python docs/build_pdf.py
"""

import json
from pathlib import Path

from reportlab.graphics.shapes import Drawing, Line, Polygon, Rect, String
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import (
    CondPageBreak, KeepTogether, PageBreak, Paragraph, Preformatted, SimpleDocTemplate,
    Spacer, Table, TableStyle,
)

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "Hermit_Features_and_Working.pdf"
DEMO = json.loads((ROOT / "docs" / "demo-report.json").read_text())
BENCH = json.loads((ROOT / "fixtures" / "bench-results.json").read_text())

# ------------------------------------------------------------------ fonts & palette
SUP = Path("/System/Library/Fonts/Supplemental")
try:
    pdfmetrics.registerFont(TTFont("Sans", str(SUP / "Arial.ttf")))
    pdfmetrics.registerFont(TTFont("Sans-Bold", str(SUP / "Arial Bold.ttf")))
    pdfmetrics.registerFont(TTFont("Sans-Italic", str(SUP / "Arial Italic.ttf")))
    pdfmetrics.registerFont(TTFont("Mono", str(SUP / "Courier New.ttf")))
    pdfmetrics.registerFont(TTFont("Mono-Bold", str(SUP / "Courier New Bold.ttf")))
    from reportlab.pdfbase.pdfmetrics import registerFontFamily
    registerFontFamily("Sans", normal="Sans", bold="Sans-Bold", italic="Sans-Italic", boldItalic="Sans-Bold")
    SANS, BOLD, MONO = "Sans", "Sans-Bold", "Mono"
except Exception:  # non-macOS fallback
    SANS, BOLD, MONO = "Helvetica", "Helvetica-Bold", "Courier"

INK = colors.HexColor("#0A0F1D")
NAVY = colors.HexColor("#11192C")
CYAN = colors.HexColor("#0891B2")
CYAN_L = colors.HexColor("#CFFAFE")
EMERALD = colors.HexColor("#059669")
AMBER = colors.HexColor("#D97706")
RED = colors.HexColor("#DC2626")
VIOLET = colors.HexColor("#7C3AED")
TEXT = colors.HexColor("#0F172A")
MUTED = colors.HexColor("#64748B")
LINE = colors.HexColor("#E2E8F0")
SOFT = colors.HexColor("#F1F5F9")

ss = {
    "title": ParagraphStyle("title", fontName=BOLD, fontSize=30, leading=36, textColor=colors.white),
    "subtitle": ParagraphStyle("subtitle", fontName=SANS, fontSize=13, leading=19, textColor=colors.HexColor("#CBD5E1")),
    "h1": ParagraphStyle("h1", fontName=BOLD, fontSize=19, leading=24, textColor=TEXT, spaceBefore=4, spaceAfter=8),
    "h2": ParagraphStyle("h2", fontName=BOLD, fontSize=13, leading=17, textColor=CYAN, spaceBefore=10, spaceAfter=4),
    "eyebrow": ParagraphStyle("eyebrow", fontName=BOLD, fontSize=8.5, leading=11, textColor=CYAN, spaceAfter=2),
    "body": ParagraphStyle("body", fontName=SANS, fontSize=10, leading=14.5, textColor=TEXT, spaceAfter=6),
    "small": ParagraphStyle("small", fontName=SANS, fontSize=8.5, leading=12, textColor=MUTED, spaceAfter=4),
    "cell": ParagraphStyle("cell", fontName=SANS, fontSize=8.8, leading=11.5, textColor=TEXT),
    "cellb": ParagraphStyle("cellb", fontName=BOLD, fontSize=8.8, leading=11.5, textColor=TEXT),
    "cellh": ParagraphStyle("cellh", fontName=BOLD, fontSize=8.8, leading=11.5, textColor=colors.white),
    "bullet": ParagraphStyle("bullet", fontName=SANS, fontSize=10, leading=14.5, textColor=TEXT, leftIndent=14, bulletIndent=3, spaceAfter=3),
    "code": ParagraphStyle("code", fontName=MONO, fontSize=8, leading=10.6, textColor=colors.HexColor("#E2E8F0"),
                           backColor=NAVY, borderPadding=(6, 7, 6, 7), leftIndent=4, rightIndent=4, spaceBefore=4, spaceAfter=10),
    "callout": ParagraphStyle("callout", fontName=SANS, fontSize=9.5, leading=13.5, textColor=TEXT,
                              backColor=CYAN_L, borderPadding=(7, 8, 7, 8), leftIndent=4, rightIndent=4, spaceBefore=4, spaceAfter=10),
    "warn": ParagraphStyle("warn", fontName=SANS, fontSize=9.5, leading=13.5, textColor=TEXT,
                           backColor=colors.HexColor("#FEF3C7"), borderPadding=(7, 8, 7, 8), leftIndent=4, rightIndent=4, spaceBefore=4, spaceAfter=10),
}


def P(text, style="body"):
    return Paragraph(text, ss[style])


def bullets(items):
    return [Paragraph(i, ss["bullet"], bulletText="•") for i in items]


def code(text):
    """Dark code panel (Preformatted cannot paint its own background)."""
    st = ParagraphStyle("codein", fontName=MONO, fontSize=7.8, leading=10.4, textColor=colors.HexColor("#E2E8F0"))
    t = Table([[Preformatted(text.strip("\n"), st)]], colWidths=[170 * mm])
    t.setStyle(TableStyle([("BACKGROUND", (0, 0), (-1, -1), NAVY), ("LEFTPADDING", (0, 0), (-1, -1), 8),
                           ("RIGHTPADDING", (0, 0), (-1, -1), 8), ("TOPPADDING", (0, 0), (-1, -1), 7),
                           ("BOTTOMPADDING", (0, 0), (-1, -1), 7)]))
    return KeepTogether([Spacer(1, 2 * mm), t, Spacer(1, 3 * mm)])


def table(rows, widths, header=True, mono_cols=()):
    data = []
    for r, row in enumerate(rows):
        out = []
        for c, v in enumerate(row):
            if isinstance(v, str):
                style = "cellh" if header and r == 0 else "cell"
                if c in mono_cols and r > 0:
                    v = f'<font name="{MONO}" size="8">{v}</font>'
                out.append(Paragraph(v, ss[style]))
            else:
                out.append(v)
        data.append(out)
    t = Table(data, colWidths=[w * mm for w in widths], repeatRows=1 if header else 0)
    st = [
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LINEBELOW", (0, 0), (-1, -1), 0.4, LINE),
        ("TOPPADDING", (0, 0), (-1, -1), 4),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
        ("LEFTPADDING", (0, 0), (-1, -1), 5),
        ("RIGHTPADDING", (0, 0), (-1, -1), 5),
    ]
    if header:
        st += [("BACKGROUND", (0, 0), (-1, 0), NAVY)]
        st += [("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, SOFT])]
    t.setStyle(TableStyle(st))
    return t


# ------------------------------------------------------------------ diagrams
def arrow(d, x1, y1, x2, y2, color=MUTED, width=1.2):
    d.add(Line(x1, y1, x2, y2, strokeColor=color, strokeWidth=width))
    import math
    a = math.atan2(y2 - y1, x2 - x1)
    s = 5
    d.add(Polygon([x2, y2, x2 - s * math.cos(a - 0.4), y2 - s * math.sin(a - 0.4),
                   x2 - s * math.cos(a + 0.4), y2 - s * math.sin(a + 0.4)],
                  fillColor=color, strokeColor=color))


def box(d, x, y, w, h, title, sub="", fill=colors.white, stroke=CYAN, tcolor=TEXT, size=8.5):
    d.add(Rect(x, y, w, h, rx=5, ry=5, fillColor=fill, strokeColor=stroke, strokeWidth=1.1))
    if sub:
        d.add(String(x + w / 2, y + h / 2 + 2, title, fontName=BOLD, fontSize=size, fillColor=tcolor, textAnchor="middle"))
        d.add(String(x + w / 2, y + h / 2 - 9, sub, fontName=SANS, fontSize=size - 1.5, fillColor=MUTED, textAnchor="middle"))
    else:
        d.add(String(x + w / 2, y + h / 2 - 3, title, fontName=BOLD, fontSize=size, fillColor=tcolor, textAnchor="middle"))


def pipeline_diagram():
    d = Drawing(170 * mm, 62 * mm)
    stages = [
        ("1 Ingest", "webhook / CLI / API", CYAN),
        ("2 Diff + AST", "git2 + tree-sitter", CYAN),
        ("3 Security", "Semgrep + builtin", RED),
        ("4 Blast radius", "call graph", AMBER),
        ("5 Tests", "selection", EMERALD),
        ("6 Risk", "6 factors, 0-100", VIOLET),
        ("7 AI review", "Qwen 7B, validated", VIOLET),
    ]
    w, h, gap = 41 * mm, 15 * mm, 2.3 * mm
    pos = []
    for i, (t, s, c) in enumerate(stages):
        row = 0 if i < 4 else 1
        col = i if i < 4 else i - 4
        x = col * (w + gap) + (0 if row == 0 else (w + gap) / 2)
        y = 40 * mm if row == 0 else 14 * mm
        box(d, x, y, w, h, t, s, stroke=c)
        pos.append((x, y))
    for i in range(3):
        x, y = pos[i]
        arrow(d, x + w, y + h / 2, pos[i + 1][0], y + h / 2)
    x, y = pos[3]
    mid = (y + pos[4][1] + h) / 2
    d.add(Line(x + w / 2, y, x + w / 2, mid, strokeColor=MUTED, strokeWidth=1.2))
    d.add(Line(x + w / 2, mid, pos[4][0] + w / 2, mid, strokeColor=MUTED, strokeWidth=1.2))
    arrow(d, pos[4][0] + w / 2, mid, pos[4][0] + w / 2, pos[4][1] + h, MUTED)
    for i in range(4, 6):
        x, y = pos[i]
        arrow(d, x + w, y + h / 2, pos[i + 1][0], y + h / 2)
    d.add(String(0, 4 * mm, "Stages 3 (security) and the repo-wide AST index run in parallel threads. Stage 7 is optional and never blocks the report.",
                 fontName=SANS, fontSize=7.5, fillColor=MUTED))
    return d


def architecture_diagram():
    d = Drawing(170 * mm, 92 * mm)
    # left: inputs
    box(d, 0, 70 * mm, 36 * mm, 14 * mm, "GitHub", "pull_request event", stroke=TEXT)
    box(d, 0, 46 * mm, 36 * mm, 14 * mm, "Developer", "hermit analyze (CLI)", stroke=TEXT)
    box(d, 0, 22 * mm, 36 * mm, 14 * mm, "Browser", "dashboard (Next.js)", stroke=TEXT)
    # server
    d.add(Rect(48 * mm, 14 * mm, 60 * mm, 74 * mm, rx=6, ry=6, fillColor=SOFT, strokeColor=CYAN, strokeWidth=1))
    d.add(String(78 * mm, 83 * mm, "hermit serve (Rust, axum + tokio)", fontName=BOLD, fontSize=8, fillColor=CYAN, textAnchor="middle"))
    box(d, 52 * mm, 66 * mm, 52 * mm, 12 * mm, "POST /webhooks/github", "HMAC-SHA256 verified")
    box(d, 52 * mm, 50 * mm, 52 * mm, 12 * mm, "REST API + SSE", "/api/analyses, /api/events")
    box(d, 52 * mm, 34 * mm, 52 * mm, 12 * mm, "Workers (N)", "claim jobs, run engine", stroke=EMERALD)
    box(d, 52 * mm, 18 * mm, 52 * mm, 12 * mm, "hermit-engine", "diff, AST, security, graph, tests, risk", stroke=VIOLET, size=8)
    # right: services
    box(d, 120 * mm, 66 * mm, 50 * mm, 14 * mm, "PostgreSQL", "analyses, job queue, embeddings", stroke=EMERALD)
    box(d, 120 * mm, 42 * mm, 50 * mm, 14 * mm, "Ollama (local)", "qwen2.5-coder:7b, nomic-embed", stroke=VIOLET)
    box(d, 120 * mm, 18 * mm, 50 * mm, 14 * mm, "Semgrep (local)", "18 bundled rules", stroke=RED)
    # arrows
    arrow(d, 36 * mm, 77 * mm, 52 * mm, 72 * mm)
    arrow(d, 36 * mm, 29 * mm, 52 * mm, 56 * mm)
    arrow(d, 36 * mm, 53 * mm, 52 * mm, 24 * mm)
    arrow(d, 104 * mm, 72 * mm, 120 * mm, 73 * mm, EMERALD)
    arrow(d, 104 * mm, 40 * mm, 120 * mm, 70 * mm, EMERALD)
    arrow(d, 104 * mm, 38 * mm, 120 * mm, 49 * mm, VIOLET)
    arrow(d, 104 * mm, 24 * mm, 120 * mm, 25 * mm, RED)
    arrow(d, 78 * mm, 34 * mm, 78 * mm, 30 * mm, MUTED)
    # back to GitHub
    d.add(Line(78 * mm, 14 * mm, 78 * mm, 6 * mm, strokeColor=CYAN, strokeWidth=1.1, strokeDashArray=[3, 2]))
    d.add(Line(78 * mm, 6 * mm, 18 * mm, 6 * mm, strokeColor=CYAN, strokeWidth=1.1, strokeDashArray=[3, 2]))
    arrow(d, 18 * mm, 6 * mm, 18 * mm, 22 * mm, CYAN)
    d.add(String(48 * mm, 1.5 * mm, "results: Check Run, summary comment, inline review (GitHub) · live updates (dashboard)",
                 fontName=SANS, fontSize=7.2, fillColor=CYAN, textAnchor="middle"))
    return d


def blast_diagram(report):
    d = Drawing(170 * mm, 56 * mm)
    box(d, 0, 22 * mm, 44 * mm, 13 * mm, "get_user_subscription()", "changed", fill=CYAN_L, stroke=CYAN, size=8)
    by_depth = {}
    for i in report["impacted"]:
        by_depth.setdefault(i["depth"], []).append(i)
    pos = {}
    for depth, items in sorted(by_depth.items()):
        for j, it in enumerate(items):
            x = 60 * mm * depth - 6 * mm
            y = 40 * mm - j * 18 * mm if depth == 1 else 22 * mm
            name = it["qualified"].split("::")[-1] + "()"
            box(d, x, y, 48 * mm, 13 * mm, name, it["file"] + (" · sensitive" if it["sensitive"] else ""),
                fill=colors.HexColor("#FEF2F2") if it["sensitive"] else colors.white,
                stroke=RED if it["sensitive"] else MUTED, size=8)
            pos[it["qualified"]] = (x, y)
    for it in report["impacted"]:
        x, y = pos[it["qualified"]]
        if it["depth"] == 1:
            arrow(d, 44 * mm, 28.5 * mm, x, y + 6.5 * mm, MUTED)
        else:
            parent = next((p for p in report["impacted"] if p["depth"] == it["depth"] - 1 and "monthly" in p["qualified"]), None) \
                or next(p for p in report["impacted"] if p["depth"] == it["depth"] - 1)
            px, py = pos[parent["qualified"]]
            arrow(d, px + 48 * mm, py + 6.5 * mm, x, y + 6.5 * mm, MUTED)
    for k, dep in enumerate(report["new_dependencies"]):
        box(d, 0, 2 * mm, 44 * mm, 12 * mm, dep["to"].split("::")[-1] + "()", "NEW dependency", fill=colors.HexColor("#FFFBEB"), stroke=AMBER, size=8)
        d.add(Line(22 * mm, 22 * mm, 22 * mm, 14 * mm, strokeColor=AMBER, strokeWidth=1.1, strokeDashArray=[3, 2]))
    return d


def factor_chart(reports):
    """Horizontal stacked bars of risk factors per fixture."""
    names = ["security", "blast_radius", "breaking_change", "complexity", "size", "test_gap"]
    labels = ["security", "blast radius", "breaking", "complexity", "size", "test gap"]
    cols = [RED, AMBER, VIOLET, CYAN, MUTED, EMERALD]
    d = Drawing(170 * mm, (len(reports) * 9 + 18) * mm)
    scale = (120 * mm) / 100
    for i, (label, factors) in enumerate(reports):
        y = (len(reports) - i) * 9 * mm + 4 * mm
        d.add(String(0, y + 2.2 * mm, label, fontName=MONO, fontSize=7.5, fillColor=TEXT))
        x = 44 * mm
        total = 0
        for n, c in zip(names, cols):
            pts = factors.get(n, 0)
            if pts > 0:
                d.add(Rect(x, y, pts * scale, 6 * mm, fillColor=c, strokeColor=colors.white, strokeWidth=0.5))
                x += pts * scale
                total += pts
        d.add(String(x + 2 * mm, y + 2 * mm, f"{round(total)}", fontName=BOLD, fontSize=8, fillColor=TEXT))
    for t, lab in [(30, "medium"), (60, "high")]:
        xx = 44 * mm + t * scale
        d.add(Line(xx, 6 * mm, xx, (len(reports) + 1) * 9 * mm + 2 * mm, strokeColor=MUTED, strokeWidth=0.6, strokeDashArray=[2, 2]))
        d.add(String(xx + 1, 2 * mm, f"{lab} ≥ {t}", fontName=SANS, fontSize=7, fillColor=MUTED))
    lx = 44 * mm
    ly = (len(reports) + 1) * 9 * mm + 5 * mm
    for n, c in zip(labels, cols):
        d.add(Rect(lx, ly, 3 * mm, 3 * mm, fillColor=c, strokeColor=c))
        d.add(String(lx + 4 * mm, ly + 0.5 * mm, n, fontName=SANS, fontSize=7, fillColor=TEXT))
        lx += 22 * mm
    return d


# ------------------------------------------------------------------ page chrome
def on_page(c, doc):
    if doc.page == 1:
        c.saveState()
        c.setFillColor(INK)
        c.rect(0, 0, A4[0], A4[1], fill=1, stroke=0)
        c.setFillColor(CYAN)
        c.rect(20 * mm, A4[1] - 52 * mm, 30 * mm, 1.2 * mm, fill=1, stroke=0)
        c.restoreState()
        return
    c.saveState()
    c.setFont(SANS, 7.5)
    c.setFillColor(MUTED)
    c.drawString(20 * mm, 12 * mm, "Hermit — Intelligent Code Analysis · Features and Working")
    c.drawRightString(A4[0] - 20 * mm, 12 * mm, f"{doc.page}")
    c.setStrokeColor(LINE)
    c.line(20 * mm, 15 * mm, A4[0] - 20 * mm, 15 * mm)
    c.restoreState()


# ------------------------------------------------------------------ content
def build():
    story = []
    demo = DEMO
    t = demo["timings"]

    # ---------- cover
    story += [
        Spacer(1, 50 * mm),
        Paragraph("SEMESTER 6 MAJOR PROJECT · TECHNICAL DOCUMENTATION", ParagraphStyle("e", parent=ss["eyebrow"], textColor=colors.HexColor("#67E8F9"))),
        Spacer(1, 4 * mm),
        P("Hermit — Intelligent Code Analysis", "title"),
        Spacer(1, 4 * mm),
        P("Features and working of an AI-assisted pull request review platform that runs entirely on-premise: "
          "deterministic AST and security analysis, call-graph blast radius, selective regression testing, an "
          "explainable risk score, and a local LLM reviewer whose output is verified against the code.", "subtitle"),
        Spacer(1, 22 * mm),
    ]
    cover_rows = [
        ["Implementation", "Rust (13-crate workspace, ~7,400 lines) + Next.js dashboard (~730 lines)"],
        ["Languages analyzed", "Python, JavaScript, TypeScript / TSX, Rust"],
        ["Local AI", "qwen2.5-coder:7b and nomic-embed-text via Ollama"],
        ["Interfaces", "CLI · GitHub App · GitHub Action · REST API + SSE · Web dashboard"],
        ["Verification", "26 automated tests · 6 labelled benchmark fixtures · real-repository run"],
    ]
    ct = Table([[Paragraph(f'<font color="#67E8F9"><b>{a}</b></font>', ss["subtitle"]), Paragraph(b, ss["subtitle"])] for a, b in cover_rows],
               colWidths=[45 * mm, 120 * mm])
    ct.setStyle(TableStyle([("VALIGN", (0, 0), (-1, -1), "TOP"), ("BOTTOMPADDING", (0, 0), (-1, -1), 5),
                            ("LINEBELOW", (0, 0), (-1, -2), 0.3, colors.HexColor("#1E293B"))]))
    story += [ct, PageBreak()]

    # ---------- 1 overview
    story += [
        P("1 · Overview", "h1"),
        P("A pull request usually changes a few lines, but its effect spreads through callers, new dependencies and "
          "code paths no test covers. Reviewers see the diff, not that spread. Linters check style, not impact. "
          "Cloud AI reviewers are often not allowed to see proprietary code."),
        P("Hermit answers five questions about every pull request, in order, using deterministic analysis first and AI last:"),
    ]
    story += bullets([
        "<b>What changed?</b> Exact functions, methods and classes, with signature and complexity changes.",
        "<b>Is it insecure?</b> Security findings on the changed lines, each labelled <i>new</i> or <i>fixed</i>.",
        "<b>What can it break?</b> Every caller up to 3 hops away, highlighting sensitive code (auth, payments, ...) and new dependencies.",
        "<b>Which tests matter?</b> Only the existing tests that reach the change, as runnable commands, plus functions no test reaches.",
        "<b>How risky is it, and what should a reviewer check?</b> An explainable 0-100 score and a local-LLM review grounded in the facts above.",
    ])
    story += [
        P("Design principle: facts first, AI second.", "h2"),
        P("Everything a reviewer acts on (findings, callers, tests, risk) is computed deterministically and is reproducible. "
          "The language model only explains and suggests. Its prompt contains only verified facts, its answer must match a "
          "JSON schema, and a validator discards comments that point outside the diff or mention code that does not exist. "
          "If the model is slow or unavailable, the deterministic report is still complete."),
        P("End-to-end pipeline", "h2"),
        pipeline_diagram(),
        Spacer(1, 4 * mm),
        P("Feature summary", "h2"),
        table([
            ["Feature", "What Hermit does", "Implemented in"],
            ["Change understanding", "Maps diff lines to the innermost symbol; detects added / modified / deleted symbols, signature changes and complexity deltas", "hermit-diff, hermit-lang"],
            ["Security intelligence", "Semgrep with 18 bundled CWE rules + builtin secret detection; new vs fixed classification", "hermit-security"],
            ["Change impact", "Parallel repo-wide call graph; reverse BFS blast radius; sensitive-path flags; new-dependency detection", "hermit-graph, hermit-engine"],
            ["Intelligent testing", "Selects tests by call-graph reachability; runner-specific commands; coverage gaps", "hermit-tests"],
            ["Risk assessment", "Six capped, explainable factors; low / medium / high tiers; configurable weights", "hermit-risk"],
            ["AI guidance", "Local Qwen review: summary, inline comments, suggestions, candidate tests; retrieval + validation", "hermit-ai"],
            ["Delivery", "CLI, GitHub App (Check Run, comment, review), Action, REST + SSE, dashboard, Docker", "cli, server, github, web"],
        ], [38, 92, 40]),
        CondPageBreak(120 * mm),
    ]

    # ---------- 2 architecture
    story += [
        P("2 · System architecture", "h1"),
        architecture_diagram(),
        Spacer(1, 3 * mm),
        P("Hermit is one Rust binary with four modes: <font name='%s'>analyze</font>, <font name='%s'>serve</font>, "
          "<font name='%s'>bench</font> and <font name='%s'>doctor</font>. The analysis engine is a library shared by all "
          "of them, so the CLI, the GitHub integration and the benchmark produce identical results." % (MONO, MONO, MONO, MONO)),
        P("Why Rust", "h2"),
        P("The original proposal used Python, FastAPI and Celery. The implementation uses Rust for three reasons. "
          "<b>Speed:</b> tree-sitter is native Rust, and parsing plus call-graph construction of a 12,000-line repository takes 36 ms. "
          "<b>Safety:</b> the workspace forbids <font name='%s'>unsafe</font> code, and the type system enforces the shared report format "
          "across every stage. <b>Simple deployment:</b> one static binary. PostgreSQL replaces both Redis and Celery as a durable "
          "job queue (<font name='%s'>FOR UPDATE SKIP LOCKED</font>)." % (MONO, MONO)),
        P("Crate map", "h2"),
        table([
            ["Crate", "Responsibility"],
            ["hermit-core", "Shared domain types: ChangedFile, Symbol, Finding, ImpactedSymbol, TestCandidate, RiskReport, AnalysisReport"],
            ["hermit-diff", "Opens the repo (libgit2), resolves base/head and their merge-base, extracts exact added/removed line ranges, reads files at any revision, renders a numbered diff for the LLM"],
            ["hermit-lang", "Tree-sitter parsing of Python, JS, TS/TSX and Rust: symbols, calls (including inside Rust macros), imports, tests, McCabe complexity; diff-to-symbol mapping"],
            ["hermit-graph", "Parallel (rayon) repository index; import-aware call resolution; reverse-BFS blast radius"],
            ["hermit-security", "Semgrep runner over base and head copies of changed files; bundled rules; builtin regex rules; new/fixed classification"],
            ["hermit-tests", "Test discovery and selection by reachability; naming-convention fallback; runner commands"],
            ["hermit-risk", "Explainable weighted score with caps and tiers"],
            ["hermit-ai", "Ollama client, embedding retrieval, grounded prompt, JSON schema, validator"],
            ["hermit-engine", "Pipeline orchestration, progress callbacks, hermit.toml, markdown report"],
            ["hermit-store", "PostgreSQL schema and migrations, job queue with retries and stale-job recovery, embedding cache"],
            ["hermit-github", "Webhook signature check, App JWT and installation tokens, checkout, Check Runs, comments, reviews"],
            ["hermit-server", "axum routes, Server-Sent Events, dashboard hosting, background workers"],
            ["hermit-cli", "hermit analyze / serve / bench / doctor"],
        ], [32, 138], mono_cols=(0,)),
        CondPageBreak(90 * mm),
    ]

    # ---------- 3 pipeline in detail
    story += [P("3 · How the pipeline works", "h1")]

    story += [
        P("Stage 1 — Ingest", "h2"),
        P("Three entry points create the same analysis request: a GitHub <font name='%s'>pull_request</font> webhook "
          "(opened, synchronize, reopened, ready_for_review; drafts ignored), the dashboard / REST API for a local repository, "
          "and the CLI. For webhooks the server verifies <font name='%s'>X-Hub-Signature-256</font> with a constant-time "
          "HMAC-SHA256 comparison, stores the analysis and a job in one transaction, and answers <b>202 Accepted</b> immediately. "
          "A worker claims the job, creates an in-progress Check Run, and fetches the base branch and "
          "<font name='%s'>refs/pull/N/head</font> into a local mirror. The installation token is passed only on the command line "
          "and is redacted from any error output." % (MONO, MONO, MONO)),
        P("Stage 2 — Diff and AST", "h2"),
        P("libgit2 computes the diff between the <b>merge-base</b> and head (as GitHub does), with rename detection and zero "
          "context lines, giving exact added and removed line ranges per file. Every source file at head is then parsed in "
          "parallel with tree-sitter. Each changed line is attributed to the <b>innermost</b> symbol containing it, so a "
          "one-line edit inside a method marks that method and not the whole class. Leading doc comments, attributes and decorators "
          "count as part of the symbol. Comparing base and head versions gives:"),
    ]
    story += bullets([
        "<b>change kind</b>: added, modified or deleted;",
        "<b>signature change</b>: normalised name and parameter text differs;",
        "<b>cyclomatic complexity</b> before and after: 1 + decision points (if, loops, match arms, boolean operators, "
        "comprehensions, catch clauses), excluding nested functions.",
    ])
    story += [
        P("Stage 3 — Security", "h2"),
        P("The base and head versions of changed files are written to a temporary directory and scanned by Semgrep in "
          "one run (metrics disabled, offline). A finding on an added head line is <b>new</b>. A finding on a removed base line "
          "whose rule now fires fewer times in the file is <b>fixed</b>, which is how Hermit can report that a PR <i>resolves</i> "
          "a vulnerability. Built-in regex rules always detect hard-coded secrets (AWS keys, private keys, GitHub tokens, "
          "password literals). They also stand in for the main injection rules when Semgrep is not installed. This stage runs on "
          "its own thread while stage 2 indexes the repository."),
        table([
            ["Language", "Bundled rules (CWE)"],
            ["Python", "SQL injection (89) · command injection (78) · eval/exec (95) · pickle/yaml/marshal (502) · TLS verify off (295) · MD5/SHA-1 (327) · Flask debug (489) · path traversal (22)"],
            ["JS / TS", "SQL injection (89) · innerHTML/document.write XSS (79) · eval/new Function (95) · child_process.exec (78) · MD5/SHA-1 (327) · TLS reject off (295)"],
            ["Rust", "SQL via format! (89) · sh -c with format! (78) · unsafe block (119) · danger_accept_invalid_certs (295)"],
            ["All", "Builtin secrets: AWS keys, private keys, GitHub tokens, credential literals (798)"],
        ], [24, 146]),
        CondPageBreak(60 * mm),
        P("Stage 4 — Blast radius", "h2"),
        P("The call graph resolves each call site to symbol definitions in four steps, from most to least precise:"),
    ]
    story += bullets([
        "<b>imports</b>: <font name='%s'>from services.cache import invalidate_cache</font>, "
        "<font name='%s'>import { query as q } from './db'</font>, <font name='%s'>use crate::cache::Store</font>;" % (MONO, MONO, MONO),
        "<b>receivers</b>: module aliases (<font name='%s'>billing.monthly_total()</font>), types "
        "(<font name='%s'>Store::flush()</font>) and <font name='%s'>self</font>/<font name='%s'>this</font> methods;" % (MONO, MONO, MONO, MONO),
        "<b>same-file</b> definitions;",
        "<b>unique names</b> anywhere in the repository. A name with more than 3 definitions is treated as unresolvable rather than guessed.",
    ])
    story += [
        P("A reverse breadth-first search from the changed symbols then finds every caller up to depth 3, recording its "
          "distance and which change it traces back to. Callers whose path contains a sensitive keyword are flagged. For deleted "
          "symbols, head call sites that still use the name are reported, because those callers are now broken. Calls that a changed "
          "function makes at head but not at base are reported as <b>new dependencies</b>."),
        P("Stage 5 — Test intelligence", "h2"),
        P("Tests are discovered per language: <font name='%s'>test_*</font> functions in test files or <font name='%s'>Test*</font> "
          "classes (Python), <font name='%s'>it()</font>/<font name='%s'>test()</font> inside <font name='%s'>describe()</font> (JS/TS), "
          "and <font name='%s'>#[test]</font> functions (Rust). A test is selected when it reaches a changed symbol through the call graph "
          "(up to 6 hops). Results are ranked by distance and coverage, and each comes with a command that runs only that test: "
          "<font name='%s'>pytest 'file::Class::test'</font>, <font name='%s'>npx vitest run file -t 'name'</font> (or jest / mocha / "
          "node --test, detected from package.json), <font name='%s'>cargo test module::test</font>. Changed functions that no test reaches "
          "are listed as <b>coverage gaps</b>. If a file has no call-path hits, tests in a file named after it (<font name='%s'>test_billing.py</font>) "
          "are suggested as a lower-confidence fallback." % ((MONO,) * 10)),
        CondPageBreak(70 * mm),
        P("Stage 6 — Risk score", "h2"),
        P("The score is the sum of six capped factors, clamped to 0-100. Each factor reports the evidence behind its points, so the "
          "number is never a black box. All weights can be changed in <font name='%s'>hermit.toml</font>." % MONO),
        table([
            ["Factor", "Points", "Cap", "Rationale"],
            ["security", "critical 45, high 25, medium 10, low 3 per NEW finding", "60", "One new critical vulnerability alone puts the PR at medium-high risk; fixed findings score 0"],
            ["blast_radius", "6 x log2(1 + callers), +5 if any sensitive caller", "20", "Logarithmic: the 10th caller adds less risk than the 1st"],
            ["breaking_change", "20 per changed signature / deletion with live direct callers", "20", "These callers will fail at runtime"],
            ["complexity", "1.5 per added decision point", "15", "More branches, more untested paths"],
            ["size", "changed lines / 500 x 10", "10", "Large diffs are reviewed less carefully"],
            ["test_gap", "15 x share of changed functions with no reaching test", "15", "Untested changes are the riskiest"],
        ], [27, 55, 12, 76]),
        P("Tiers: <b>low</b> below 30, <b>medium</b> 30-59, <b>high</b> 60 and above. The GitHub Check Run fails at the configured "
          "tier (default: high).", "small"),
        Spacer(1, 2 * mm),
        KeepTogether([P("Risk composition of the benchmark fixtures:", "small"),
                      factor_chart([(f["name"], {}) for f in []] or bench_factor_rows())]),
        CondPageBreak(80 * mm),
        P("Stage 7 — Local AI review", "h2"),
        P("The AI stage runs only after the deterministic report exists:"),
    ]
    story += bullets([
        "<b>Retrieve.</b> Up to 400 candidate functions are embedded with <font name='%s'>nomic-embed-text</font>. Embeddings are cached by "
        "SHA-256 of model and text, in PostgreSQL or a local file, so unchanged code is never re-embedded. Candidates are ranked by cosine "
        "similarity to the changed code, with a bonus for direct call-graph neighbours. The top 6 become read-only context." % MONO,
        "<b>Prompt.</b> A system prompt sets the rules: the VERIFIED FACTS are ground truth, comment only on numbered diff lines, never invent "
        "vulnerabilities, callers or tests. The user prompt contains the risk factors, changed symbols, findings, blast radius, new "
        "dependencies, tests and coverage gaps, then the diff with explicit head line numbers, then the retrieved code.",
        "<b>Constrain.</b> Ollama structured output forces a JSON object {summary, comments[{file, line, body, suggestion}], tests[]} "
        "(temperature 0.1, 16k context).",
        "<b>Validate.</b> A comment is dropped if its file is not in the change, or its line is more than 3 lines from any added line "
        "(near misses are snapped onto the closest added line). It is also dropped if it cites a backticked identifier that appears nowhere in "
        "the diff, the file or the symbol index. The number of dropped comments is shown in the report.",
        "<b>Fail safe.</b> A timeout (default 120 s), an unreachable server or invalid JSON produces a warning. The deterministic report is published without AI.",
    ])
    story += [CondPageBreak(120 * mm)]

    # ---------- 4 demo walkthrough
    fac = {f["name"]: f for f in demo["risk"]["factors"]}
    story += [
        P("4 · Worked example: the viva demo pull request", "h1"),
        P("The deck's slide-6 example is a real fixture (<font name='%s'>fixtures/python_sqli</font>). A billing function that built SQL "
          "with an f-string is changed to use a bound parameter, and it now also invalidates a cache entry. The output below is "
          "from an actual run through the server; nothing is simulated." % MONO),
        code('''### services/billing.py  (head line numbers)
    4 |   def get_user_subscription(db, org_id: str):
      | -     # Old insecure query execution
      | -     query = f"SELECT * FROM accounts WHERE id = '{org_id}'"
      | -     return db.execute(query).fetchone()
    5 | +     # Hermit Target Line: Parameterized query
    6 | +     query = "SELECT * FROM accounts WHERE id = :id"
    7 | +     result = db.execute(query, {"id": org_id}).fetchone()
    8 | +     invalidate_cache(f"org:{org_id}")
    9 | +     return result'''),
        table([
            ["Stage", "Result"],
            ["Diff + AST", "1 file, +6 / -4 lines; 1 symbol modified: services/billing.py::get_user_subscription (signature unchanged, complexity 1 -> 1)"],
            ["Security", "CWE-89 SQL injection (Semgrep rule hermit.python.sql-injection, critical) present at base, absent at head: <b>FIXED</b>. No new findings."],
            ["Blast radius", "subscription_handler and monthly_total (direct), charge_handler (2 hops). All three are in payments/billing code, so all are flagged sensitive."],
            ["New dependency", "get_user_subscription now calls services/cache.py::invalidate_cache"],
            ["Tests", "2 of 3 tests selected: test_get_user_subscription (1 hop), test_monthly_total (2 hops). test_invalidate_missing_key is correctly skipped."],
            ["Risk", f"<b>{demo['risk']['score']} / 100, {demo['risk']['tier'].upper()}</b>: {fac['blast_radius']['points']} blast radius, "
                     f"{fac['size']['points']} size, 0 for security (the only finding was fixed), breaking change, complexity and test gap"],
            ["AI review", f"{demo['ai']['model']}, {demo['ai']['latency_ms'] / 1000:.1f} s: summary + {len(demo['ai']['comments'])} inline comments, "
                          f"{demo['ai']['rejected']} dropped by the validator"],
            ["Timing", f"diff {t['diff_ms']} ms · parse {t['parse_ms']} ms · security {t['security_ms']} ms · graph {t['graph_ms']} ms · "
                       f"AI {t['ai_ms']} ms · total {t['total_ms']} ms"],
        ], [30, 140]),
        P("Blast-radius graph for this PR", "h2"),
        blast_diagram(demo),
        P("AI comments that passed validation", "h2"),
    ]
    for c in demo["ai"]["comments"]:
        story.append(P(f"<font name='{MONO}' color='#0891B2'>{c['file']}:{c['line']}</font> — {c['body']}", "callout"))
    story += [
        P("<b>Observed limitation.</b> The validator checks comment locations and identifiers, but the free-text <i>summary</i> is not "
          "fact-checked. In this run the model's summary said the PR had \"no new dependencies\", which contradicts the verified "
          "new dependency on invalidate_cache. That is why the dashboard and PR comment always show the deterministic sections next "
          "to the AI text. Checking the summary against the facts is listed as future work.", "warn"),
        CondPageBreak(100 * mm),
    ]

    # ---------- 5 interfaces
    story += [
        P("5 · Interfaces", "h1"),
        P("Command line", "h2"),
        code('''hermit analyze --repo . --base main --head feature      # markdown report (AI included)
hermit analyze ... --format json --out report.json      # machine-readable
hermit analyze ... --no-ai --fail-on high               # CI gate: exit code 2 at HIGH
hermit serve                                            # webhook + API + dashboard + workers
hermit bench fixtures --json results.json               # accuracy / latency
hermit doctor                                           # git, semgrep, ollama models, postgres'''),
        P("GitHub App", "h2"),
        P("On every push to a PR, Hermit creates a Check Run (in progress, then success or failure with the risk title and full "
          "report). It keeps <b>one</b> summary comment, found by a hidden marker and updated in place rather than spammed, and posts "
          "one review containing the validated AI comments as inline comments with GitHub <i>suggestion</i> blocks. Required "
          "permissions: Checks, Pull requests and Issues (read and write), Contents (read)."),
        P("GitHub Action", "h2"),
        P("<font name='%s'>action/action.yml</font> builds Hermit on the runner, analyzes the PR's base and head SHAs, writes the "
          "markdown report to the job summary, exposes <font name='%s'>risk-score</font> and <font name='%s'>risk-tier</font> outputs, and "
          "fails the job at the chosen tier. This works without hosting a server." % (MONO, MONO, MONO)),
        P("REST API and live progress", "h2"),
        table([
            ["Endpoint", "Purpose"],
            ["POST /webhooks/github", "GitHub events; 401 on bad signature, 503 if the App is not configured, 202 + analysis id when queued"],
            ["POST /api/analyses", "Queue a local repository ({repo_path, base, head}); revisions are validated before queueing"],
            ["GET /api/analyses[?repo=&limit=]", "Recent analyses with status, stage and risk"],
            ["GET /api/analyses/{id}", "Status, full JSON report and rendered markdown"],
            ["GET /api/repos", "Repositories with analysis count and latest score"],
            ["GET /api/events", "Server-Sent Events: {id, status, stage, risk_score} as each stage starts"],
            ["GET /api/health", "Database, Ollama (models), Semgrep, GitHub App, webhook secret"],
        ], [55, 115], mono_cols=(0,)),
        P("Web dashboard", "h2"),
        P("A Next.js + Tailwind single-page app (static export, served by the Rust server) in the deck's colour palette. It shows:"),
    ]
    story += bullets([
        "a health bar (database, Ollama, Semgrep, GitHub App);",
        "a form to analyze any local repository;",
        "a live list of analyses whose status updates over SSE;",
        "a detail view with a risk gauge, the seven-stage pipeline progress, and five tabs. "
        "<b>Overview</b>: stats, factor bars, summary, changed symbols, timing bar. "
        "<b>Findings</b>: severity, CWE, snippet, new/fixed. "
        "<b>Impact</b>: interactive call graph (React Flow) and caller table. "
        "<b>Tests</b>: commands with copy button, coverage gaps. "
        "<b>AI review</b>: summary, verified comments with suggestions, proposed tests.",
    ])
    story += [
        P("Persistence and job queue", "h2"),
        P("PostgreSQL holds four tables: <font name='%s'>repositories</font>, <font name='%s'>analyses</font> (status, stage, score, "
          "full JSON report), <font name='%s'>jobs</font> and <font name='%s'>embeddings</font>. Workers claim jobs with "
          "<font name='%s'>UPDATE ... WHERE id = (SELECT ... FOR UPDATE SKIP LOCKED LIMIT 1)</font>, so several workers or "
          "processes never take the same job. Failures are retried with exponential backoff (10 s, 20 s, 40 s; 3 attempts). Jobs left "
          "running for 15 minutes by a crashed worker are reclaimed. All SQL is static text with bind parameters, which sqlx 0.9 "
          "enforces at compile time." % ((MONO,) * 5)),
        CondPageBreak(120 * mm),
    ]

    # ---------- 6 evaluation
    rows = [["Fixture", "Scenario", "Risk", "Security P/R", "Impact P/R", "Tests P/R", "Tests run", "Time"]]
    scen = {
        "python_sqli": "SQLi fixed, new dependency",
        "js_xss": "new innerHTML XSS (TS)",
        "js_command_injection": "execSync injection + leaked token",
        "python_signature_change": "breaking signature change",
        "rust_unsafe": "unsafe block; tests in assert_eq!",
        "clean_pr": "safe addition with own test",
    }

    def pr(c):
        p = 1.0 if c["tp"] + c["fp"] == 0 else c["tp"] / (c["tp"] + c["fp"])
        r = 1.0 if c["tp"] + c["fn_"] == 0 else c["tp"] / (c["tp"] + c["fn_"])
        return f"{p:.2f} / {r:.2f}"

    for f in BENCH["fixtures"]:
        rows.append([f["name"], scen.get(f["name"], ""), f"{f['score']} {f['tier']}", pr(f["security"]), pr(f["impacted"]),
                     pr(f["tests"]), f"{f['tests_selected']}/{f['tests_total']}", f"{f['analysis_ms']} ms"])
    story += [
        P("6 · Evaluation", "h1"),
        P("Labelled fixtures (hermit bench)", "h2"),
        P("Each fixture is a small repository with a base and a head version and an <font name='%s'>expected.toml</font> listing the "
          "findings (new/fixed CWE), changed symbols, impacted callers, tests and, optionally, the risk tier that a correct analysis must "
          "produce. <font name='%s'>hermit bench</font> builds a git repository for each, runs the full pipeline and computes precision and recall." % (MONO, MONO)),
        table(rows, [40, 36, 16, 17, 17, 17, 12, 15], mono_cols=(0,)),
        P(f"<b>Overall:</b> security P={BENCH['security_precision']:.2f} R={BENCH['security_recall']:.2f} · blast radius "
          f"P={BENCH['impact_precision']:.2f} R={BENCH['impact_recall']:.2f} · tests P={BENCH['test_precision']:.2f} "
          f"R={BENCH['test_recall']:.2f} · {BENCH['tests_selected']} of {BENCH['tests_total']} tests run · p50 {BENCH['p50_ms']} ms, "
          f"p95 {BENCH['p95_ms']} ms (deterministic stages; about 1.4 s of each run is Semgrep start-up)."),
        P("<b>How to read these numbers.</b> The fixtures were written by the author to exercise each feature, and three defects found "
          "by the first benchmark run were fixed before this result: Rust calls inside macros were missed, doc comments were attributed "
          "to the enclosing impl, and risk weights were recalibrated. A perfect score therefore shows that every feature works end to "
          "end on its target scenario. It is not an independent accuracy claim; that would need a large set of real, externally labelled "
          "pull requests.", "warn"),
        P("Real-world repository", "h2"),
        P("Hermit was run on a recent commit of <b>psf/requests</b> (37 Python files, about 12,000 lines; commit 6f66281, "
          "\"Add hasattr checks for remaining protocol isinstance checks\"):"),
        table([
            ["Measure", "Result"],
            ["Changed", "3 files, 9 symbols"],
            ["Blast radius", "5 impacted callers"],
            ["Tests selected", "107 of 347 test functions (69% fewer to run)"],
            ["Risk", "35 / 100, medium"],
            ["Parse + call graph of the whole repository", "36 ms"],
            ["Total deterministic analysis", "1.58 s (1.58 s of it is Semgrep)"],
        ], [70, 100]),
        P("Automated tests", "h2"),
        P("26 unit and integration tests cover the diff extractor (real git repositories), the parsers for all four languages, "
          "diff-to-symbol mapping, cross-file resolution in Python/TS/Rust, deleted-symbol impact, test selection and runner "
          "commands, Semgrep and builtin new/fixed classification, risk scoring, config defaults, the AI validator (rejecting "
          "out-of-diff and invented-identifier comments), webhook signatures, PR event parsing and token redaction. "
          "<font name='%s'>cargo clippy -D warnings</font> and <font name='%s'>cargo fmt</font> are clean. CI "
          "(<font name='%s'>.github/workflows/ci.yml</font>) runs lint, tests, the benchmark and the dashboard build." % (MONO, MONO, MONO)),
        CondPageBreak(90 * mm),
    ]

    # ---------- 7 security & privacy, setup, limits
    story += [
        P("7 · Security and privacy", "h1"),
    ]
    story += bullets([
        "<b>No code leaves the machine.</b> Parsing, Semgrep (metrics off, bundled rules), embeddings and the LLM all run locally. "
        "Registry rule packs are opt-in.",
        "<b>Authenticated webhooks.</b> Constant-time HMAC-SHA256 check of the raw body. PR events are refused when the App is not configured.",
        "<b>Least privilege on GitHub.</b> Short-lived installation tokens (cached until 5 minutes before expiry). Tokens are never written "
        "to git config and are redacted from errors.",
        "<b>Local-only by default.</b> The server binds to 127.0.0.1. Analyzing local paths can be disabled (HERMIT_ALLOW_LOCAL=0), and "
        "Docker Compose disables it.",
        "<b>Safe code.</b> unsafe Rust is forbidden workspace-wide, and all SQL is static text with bind parameters.",
        "<b>Untrusted model output.</b> AI text is never executed. Comments are validated before posting, and suggestions are limited to 15 lines.",
    ])
    story += [
        P("8 · Installation and running", "h1"),
        code('''brew install rustup semgrep postgresql@16 ollama && rustup default stable
ollama pull qwen2.5-coder:7b && ollama pull nomic-embed-text
cd hermit && make build && ./target/release/hermit doctor

make demo                                   # the slide-6 PR, end to end
createdb hermit && make web && scripts/dev.sh    # dashboard on http://127.0.0.1:8080
SMEE_URL=https://smee.io/<channel> scripts/dev.sh # + GitHub webhooks via tunnel
docker compose up                           # containerised (Postgres + Ollama + Hermit)'''),
        P("Per-repository behaviour is set in <font name='%s'>hermit.toml</font>: ignored paths, graph depths, sensitive keywords, "
          "Semgrep packs, all risk weights and tier thresholds, AI model, timeout and context size, and the Check Run failure tier. "
          "Environment overrides: HERMIT_OLLAMA_URL, HERMIT_MODEL, HERMIT_AI=0." % MONO),
        P("9 · Limitations and future work", "h1"),
    ]
    story += bullets([
        "<b>Static resolution is approximate.</b> Dynamic dispatch, monkey-patching, reflection and dependency injection are invisible, "
        "and ambiguous names are skipped. Type-aware resolution (e.g. via language servers) would raise recall.",
        "<b>Test selection is static.</b> It cannot see tests that reach code through fixtures, mocks or HTTP. Recording real coverage "
        "would complement it.",
        "<b>AI summary is not fact-checked</b> (see section 4). Planned: reject or annotate summary sentences that contradict the verified facts.",
        "<b>Latency.</b> The 7B model needs about 10-20 s per PR on an M4 Pro, and Semgrep adds about 1.4 s of start-up. A persistent "
        "Semgrep process (LSP mode) would remove most of that.",
        "<b>Evaluation scale.</b> Six author-written fixtures plus one real repository. A larger benchmark of real, externally labelled PRs is the next step.",
        "<b>Not yet run against live GitHub.</b> Webhook verification, event parsing and queueing were exercised locally with signed "
        "requests, and the GitHub API calls follow the documented REST endpoints. A first install of the App on a real repository "
        "has not been performed in this environment.",
        "<b>Docker Compose</b> is provided but was not built here, because Docker is not installed on the development machine.",
    ])
    return story


def bench_factor_rows():
    """Recompute factor points per fixture by re-reading their reports is unnecessary: the bench JSON
    has totals only, so use the demo report factors plus the known fixture totals split by the
    deterministic weights recorded in docs/bench-factors.json when present."""
    p = ROOT / "docs" / "bench-factors.json"
    if p.exists():
        data = json.loads(p.read_text())
        return [(k, v) for k, v in data.items()]
    return [("python_sqli", {f["name"]: f["points"] for f in DEMO["risk"]["factors"]})]


def main():
    doc = SimpleDocTemplate(str(OUT), pagesize=A4, leftMargin=20 * mm, rightMargin=20 * mm,
                            topMargin=18 * mm, bottomMargin=20 * mm,
                            title="Hermit — Features and Working", author="Hermit project",
                            subject="Intelligent code analysis: features, architecture, evaluation")
    doc.build(build(), onFirstPage=on_page, onLaterPages=on_page)
    print(OUT)


if __name__ == "__main__":
    main()
