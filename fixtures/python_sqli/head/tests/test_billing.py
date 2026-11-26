from services.billing import monthly_total, get_user_subscription


class FakeDB:
    def execute(self, q, params=None):
        return self

    def fetchone(self):
        return {"price": 10}


def test_monthly_total():
    assert monthly_total(FakeDB(), "o1") == 10


def test_get_user_subscription():
    assert get_user_subscription(FakeDB(), "o1")["price"] == 10
