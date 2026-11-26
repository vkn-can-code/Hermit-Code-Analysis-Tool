RATES = {"USD": 1.0, "EUR": 1.08, "INR": 0.012}


def to_cents(amount, currency):
    if currency not in RATES:
        raise ValueError(f"unknown currency {currency}")
    usd = amount * RATES[currency]
    return int(round(usd * 100))


def format_price(cents):
    return f"${cents / 100:.2f}"
