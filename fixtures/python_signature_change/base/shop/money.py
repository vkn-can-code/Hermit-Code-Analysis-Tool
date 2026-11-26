def to_cents(amount):
    return int(round(amount * 100))


def format_price(cents):
    return f"${cents / 100:.2f}"
