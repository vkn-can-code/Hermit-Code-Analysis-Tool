from lib.text import slugify, truncate


def test_slugify():
    assert slugify("Hello World") == "hello-world"


def test_truncate():
    assert truncate("abcdef", 4) == "abc…"
