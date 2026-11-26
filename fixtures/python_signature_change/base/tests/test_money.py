from shop.money import format_price


def test_format_price():
    assert format_price(1050) == "$10.50"
