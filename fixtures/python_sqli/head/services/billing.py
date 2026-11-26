from services.cache import get_cached, invalidate_cache


def get_user_subscription(db, org_id: str):
    # Hermit Target Line: Parameterized query
    query = "SELECT * FROM accounts WHERE id = :id"
    result = db.execute(query, {"id": org_id}).fetchone()
    invalidate_cache(f"org:{org_id}")
    return result


def monthly_total(db, org_id: str):
    cached = get_cached(f"total:{org_id}")
    if cached is not None:
        return cached
    sub = get_user_subscription(db, org_id)
    return sub["price"] if sub else 0
