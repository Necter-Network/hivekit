import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from hivekit.runner import find_ndsr

ROOT = Path(__file__).resolve().parent.parent
EXAMPLES = ROOT / "examples"
NDSR = find_ndsr(str(ROOT))

needs_ndsr = pytest.mark.skipif(NDSR is None, reason="ndsr binary not found (set NDSR_BIN)")


@pytest.fixture(scope="session")
def vectors():
    return json.loads((Path(__file__).parent / "fixtures" / "test-vectors.json").read_text(encoding="utf-8"))


def ndsr_inspect(hbc_path):
    p = subprocess.run([NDSR, "inspect", str(hbc_path)], capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout)


def ndsr_run(hbc_path, fn, input_text="", data_dir=None, gas=9_000_000_000):
    from hivekit.runner import run_with_ndsr

    return run_with_ndsr(NDSR, str(hbc_path), fn, input_text, gas=gas, data_dir=str(data_dir) if data_dir else None)


def install(data_dir, address, hbc_bytes):
    d = Path(data_dir) / "modules"
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{address}.hbc").write_bytes(hbc_bytes)
