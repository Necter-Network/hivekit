"""
HiveKit example — math operations module.
Run locally:  hivec run math_module.py addNumbers '{"a": 10, "b": 5}'
Compile:      hivec build math_module.py
"""

from hivekit import hive


@hive.define("addNumbers")
def add_numbers(input: dict) -> dict:
    a = input.get("a", 0)
    b = input.get("b", 0)
    return {"total": a + b}


@hive.define("multiply")
def multiply(input: dict) -> dict:
    a = input.get("a", 0)
    b = input.get("b", 0)
    return {"result": a * b}


@hive.define("stats")
def stats(input: dict) -> dict:
    numbers = input.get("numbers", [])
    if not numbers:
        return {"error": "no numbers provided"}
    return {
        "count": len(numbers),
        "sum": sum(numbers),
        "min": min(numbers),
        "max": max(numbers),
        "avg": sum(numbers) / len(numbers),
    }
