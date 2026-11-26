from services import billing


def charge_handler(request, db):
    org = request.args["org"]
    amount = billing.monthly_total(db, org)
    return {"org": org, "amount": amount}


def subscription_handler(request, db):
    return billing.get_user_subscription(db, request.args["org"])
