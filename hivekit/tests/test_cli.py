import json

from click.testing import CliRunner

from hivekit.cli import main

from conftest import EXAMPLES, needs_ndsr


def test_build_and_functions(tmp_path):
    r = CliRunner().invoke(main, ["build", str(EXAMPLES / "counter.py"), "-o", str(tmp_path)])
    assert r.exit_code == 0, r.output
    out = json.loads(r.output)
    assert out["functions"] == ["get", "increment", "note", "relay"]
    assert (tmp_path / "counter.hbc").exists()
    r = CliRunner().invoke(main, ["functions", str(EXAMPLES / "counter.py")])
    assert r.output.splitlines() == ["0\tget", "1\tincrement", "2\tnote", "3\trelay"]


def test_run_local():
    r = CliRunner().invoke(main, ["run", "--local", str(EXAMPLES / "counter.py"), "increment", '{"by": 3}'])
    assert r.exit_code == 0, r.output
    assert json.loads(r.output) == {"runner": "local", "success": True, "output": {"count": 3}}


@needs_ndsr
def test_run_delegates_to_ndsr():
    r = CliRunner().invoke(main, ["run", str(EXAMPLES / "counter.py"), "increment", '{"by": 3}'])
    assert r.exit_code == 0, r.output
    report = json.loads(r.output)
    assert report["runner"] == "ndsr"
    assert report["success"] is True
    assert json.loads(report["output"]) == {"count": 3}
    assert report["events"] == [{"name": "incremented", "data": {"by": 3, "count": 3}}]


def test_build_reports_errors(tmp_path):
    bad = tmp_path / "bad.py"
    bad.write_text("from hivekit import hive\nname = 'x'\nhive.define(name, lambda i: 1)\n")
    r = CliRunner().invoke(main, ["build", str(bad), "-o", str(tmp_path)])
    assert r.exit_code == 1
    assert "string literal" in r.output
