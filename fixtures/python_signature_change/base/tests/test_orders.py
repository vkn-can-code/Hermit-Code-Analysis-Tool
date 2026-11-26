from shop.orders import order_total


def test_order_total():
    assert order_total([{"price": 1.5, "qty": 2}]) == 300
