import io
import subprocess

from houritsu_builder.fastimport import Stream, jst_midnight, write_blob


def test_jst_midnight():
    assert jst_midnight("2017-03-01") == 1488294000
    assert jst_midnight("1960-01-01") == 86400


def test_stream_imports_into_git(tmp_path):
    buf = io.BytesIO()
    s = Stream(buf)
    s.commit("refs/heads/main", "2017-03-01", "初版\n", {"laws/act/A.md": "# 法\n"}, author="平成十五年法律第五十七号")
    s.commit("refs/heads/upcoming", "2027-01-17", "改正\n", {"laws/act/A.md": "# 法　改\n"}, from_ref="refs/heads/main",
             author="令和八年法律第五十六号")
    s.done()
    subprocess.run(["git", "init", "-q", "-b", "main", str(tmp_path)], check=True)
    subprocess.run(["git", "fast-import", "--quiet", "--done"], cwd=tmp_path, input=buf.getvalue(), check=True)
    log = subprocess.run(["git", "log", "--format=%an|%ae|%cn|%ad|%s", "--date=iso", "upcoming"], cwd=tmp_path, capture_output=True, text=True, check=True).stdout
    assert log.splitlines() == ["令和八年法律第五十六号||houritsu-builder|2027-01-17 00:00:00 +0900|改正",
                              "平成十五年法律第五十七号||houritsu-builder|2017-03-01 00:00:00 +0900|初版"]
    show = subprocess.run(["git", "show", "main:laws/act/A.md"], cwd=tmp_path, capture_output=True, text=True, check=True).stdout
    assert show == "# 法\n"


def test_stream_deletes_files(tmp_path):
    buf = io.BytesIO()
    s = Stream(buf)
    s.commit("refs/heads/main", "2017-03-01", "初版\n", {"laws/act/A.md": "# 法\n", "laws/act/B.md": "# 乙\n"})
    s.commit("refs/heads/main", "2018-03-01", "廃止\n", {}, deleted={"laws/act/A.md", "laws/act/C.md"})
    s.done()
    subprocess.run(["git", "init", "-q", "-b", "main", str(tmp_path)], check=True)
    subprocess.run(["git", "fast-import", "--quiet", "--done"], cwd=tmp_path, input=buf.getvalue(), check=True)
    files = subprocess.run(["git", "ls-tree", "-r", "--name-only", "main"], cwd=tmp_path, capture_output=True, text=True, check=True).stdout
    assert files.splitlines() == ["laws/act/B.md"]


def test_stream_references_loose_blobs(tmp_path):
    subprocess.run(["git", "init", "-q", "-b", "main", str(tmp_path)], check=True)
    sha = write_blob(tmp_path / ".git" / "objects", "# 法\n".encode())
    assert sha == subprocess.run(["git", "hash-object", "--stdin"], input="# 法\n".encode(), capture_output=True, check=True).stdout.decode().strip()
    buf = io.BytesIO()
    s = Stream(buf)
    s.commit("refs/heads/main", "2017-03-01", "初版\n", {"README.md": "r\n"}, blobs={"laws/act/A.md": sha})
    s.done()
    subprocess.run(["git", "fast-import", "--quiet", "--done"], cwd=tmp_path, input=buf.getvalue(), check=True)
    show = subprocess.run(["git", "show", "main:laws/act/A.md"], cwd=tmp_path, capture_output=True, text=True, check=True).stdout
    assert show == "# 法\n"
    subprocess.run(["git", "fsck", "--strict"], cwd=tmp_path, check=True, capture_output=True)
