from shop.money import to_cents, format_price
from shop.orders import order_total


def invoice_line(item):
    return format_price(to_cents(item["price"]))


def invoice_total(items):
    return format_price(order_total(items))
