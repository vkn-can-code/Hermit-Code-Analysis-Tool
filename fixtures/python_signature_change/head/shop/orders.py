from shop.money import to_cents


def order_total(items):
    return sum(to_cents(i["price"]) * i["qty"] for i in items)
