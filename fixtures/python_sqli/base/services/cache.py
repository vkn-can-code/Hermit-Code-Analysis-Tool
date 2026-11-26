_store = {}


def invalidate_cache(key):
    _store.pop(key, None)


def get_cached(key):
    return _store.get(key)
