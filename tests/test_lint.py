import base64
import gzip
import json
import subprocess
from importlib import resources

from conftest import TABLE_FIXTURE
from houritsu_builder.build import RUMDL
from houritsu_builder.render import render_law


def test_table_fixture_passes_shipped_rumdl_config(tmp_path):
    xml = base64.b64decode(json.loads(gzip.decompress(TABLE_FIXTURE.read_bytes()))["law_full_text"])
    target = tmp_path / "laws" / "act" / "x.md"
    target.parent.mkdir(parents=True)
    target.write_text(render_law(xml).markdown)
    config = resources.files("houritsu_builder").joinpath("assets", "rumdl.toml").read_text()
    (tmp_path / ".rumdl.toml").write_text(config)
    result = subprocess.run([RUMDL, "check", "--no-cache", "--respect-gitignore=false", "."], cwd=tmp_path, capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr


def test_shipped_rumdl_config_rejects_a_skipped_heading_level(tmp_path):
    target = tmp_path / "laws" / "act" / "x.md"
    target.parent.mkdir(parents=True)
    target.write_text("# 法\n\n### 第一条\n\n本文\n")
    (tmp_path / ".rumdl.toml").write_text(resources.files("houritsu_builder").joinpath("assets", "rumdl.toml").read_text())
    result = subprocess.run([RUMDL, "check", "--no-cache", "--respect-gitignore=false", "."], cwd=tmp_path, capture_output=True, text=True)
    assert result.returncode != 0 and "MD001" in result.stdout, result.stdout + result.stderr
