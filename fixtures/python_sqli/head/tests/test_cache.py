from services.cache import invalidate_cache, get_cached


def test_invalidate_missing_key():
    invalidate_cache("nope")
    assert get_cached("nope") is None
