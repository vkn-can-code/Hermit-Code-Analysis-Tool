from services.cache import get_cached


def get_user_subscription(db, org_id: str):
    # Old insecure query execution
    query = f"SELECT * FROM accounts WHERE id = '{org_id}'"
    return db.execute(query).fetchone()


def monthly_total(db, org_id: str):
    cached = get_cached(f"total:{org_id}")
    if cached is not None:
        return cached
    sub = get_user_subscription(db, org_id)
    return sub["price"] if sub else 0
