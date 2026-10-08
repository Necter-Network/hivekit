"""
HiveKit example module — hello world.
Run locally:  hivec run hello.py greet '{"name": "Alice"}'
Compile:      hivec build hello.py
"""

from hivekit import hive


@hive.define("greet")
def greet(input: dict) -> dict:
    name = input.get("name", "world")
    return {"message": f"Hello, {name}!"}


@hive.define("ping")
def ping(input: dict) -> dict:
    return {"pong": True, "echo": input}
