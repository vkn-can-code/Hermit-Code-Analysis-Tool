def slugify(s):
    return "-".join(s.lower().split())


def truncate(s, n):
    return s if len(s) <= n else s[: n - 1] + "…"
