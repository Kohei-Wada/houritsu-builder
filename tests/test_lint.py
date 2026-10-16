import base64
import gzip
import json
import shutil
import subprocess
from importlib import resources

import pytest

from conftest import TABLE_FIXTURE
from houritsu_builder.build import MARKDOWNLINT
from houritsu_builder.render import render_law


@pytest.mark.skipif(shutil.which("npx") is None, reason="npx not installed")
def test_table_fixture_passes_shipped_markdownlint_config(tmp_path):
    xml = base64.b64decode(json.loads(gzip.decompress(TABLE_FIXTURE.read_bytes()))["law_full_text"])
    target = tmp_path / "laws" / "act" / "x.md"
    target.parent.mkdir(parents=True)
    target.write_text(render_law(xml).markdown)
    config = resources.files("houritsu_builder").joinpath("assets", "markdownlint-cli2.yaml").read_text()
    (tmp_path / ".markdownlint-cli2.yaml").write_text(config)
    result = subprocess.run(MARKDOWNLINT, cwd=tmp_path, capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
