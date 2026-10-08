"""
HiveKit example: a token ledger kept in module state.

Build:  hivec build token_module.py
Run:    hivec run token_module.py mint '{"to": "alice", "amount": 100}' --data-dir .state
        hivec run token_module.py transfer '{"from": "alice", "to": "bob", "amount": 40}' --data-dir .state
        hivec run token_module.py balance '{"account": "bob"}' --data-dir .state
"""

from hivekit import hive


def _balance(account):
    return hive.db.get("balance:" + account, 0)


@hive.define("mint")
def mint(input):
    to, amount = input.get("to"), input.get("amount", 0)
    if not to or not isinstance(amount, int) or amount <= 0:
        hive.fail("mint needs a recipient and a positive integer amount")
    hive.db.set("balance:" + to, _balance(to) + amount)
    hive.emit("minted", {"to": to, "amount": amount})
    return {"ok": True, "balance": _balance(to)}


@hive.define("transfer")
def transfer(input):
    sender, receiver, amount = input.get("from"), input.get("to"), input.get("amount", 0)
    if not sender or not receiver:
        hive.fail("missing from/to")
    if not isinstance(amount, int) or amount <= 0:
        hive.fail("amount must be a positive integer")
    if _balance(sender) < amount:
        hive.fail("insufficient balance")
    hive.db.set("balance:" + sender, _balance(sender) - amount)
    hive.db.set("balance:" + receiver, _balance(receiver) + amount)
    hive.emit("transfer", {"from": sender, "to": receiver, "amount": amount})
    return {"ok": True}


@hive.define("balance")
def balance(input):
    account = input.get("account")
    if not account:
        hive.fail("account required")
    return {"account": account, "balance": _balance(account)}
