import json
import sys
from pathlib import Path

import pytest

import opencrs

ROOT = Path(__file__).resolve().parents[3]
HARBOURLINE = ROOT / "examples" / "harbourline"


def spec() -> str:
    return (HARBOURLINE / "retail.screens.json").read_text()


def test_version_is_the_spec_version():
    assert opencrs.version() == "0.1"


def test_lint_accepts_the_harbourline_example():
    res = opencrs.lint(spec())
    assert res["kind"] == "screens"
    assert "schema_errors" not in res
    assert "document_checks" not in res


def test_lint_reports_a_broken_reference():
    broken = json.dumps(
        {
            "opencrs": "0.1",
            "publisher": "a.example",
            "updated_at": "2026-10-20T09:00:00Z",
            "networks": [
                {
                    "id": "n",
                    "name": "N",
                    "formats": [{"id": "f", "pixels": [1, 1], "media": {"static": ["image/png"]}}],
                    "screens": [{"id": "s", "format": "g"}],
                }
            ],
        }
    )
    res = opencrs.lint(broken)
    assert len(res["document_checks"]) == 1
    assert res["document_checks"][0]["code"] == "CRS-REFERENCE"


def test_lint_reports_schema_errors():
    res = opencrs.lint(json.dumps({"opencrs": "0.1", "publisher": "a.example", "networks": []}))
    assert res["schema_errors"]
    assert isinstance(res["schema_errors"][0]["path"], str)
    assert isinstance(res["schema_errors"][0]["message"], str)


def test_lint_raises_for_an_unreadable_document():
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.lint("{not json")
    with pytest.raises(ValueError):
        opencrs.lint('{"hello":1}')


def test_error_class():
    assert issubclass(opencrs.OpenCRSError, ValueError)


def test_resolve_a_format_reference_gives_one_result_per_screen():
    reqs = opencrs.resolve(spec(), "harbourline.example/retail/portrait-totem-75")
    assert [r["screen"] for r in reqs] == ["HC-L1-001", "HC-L1-002"]
    assert reqs[0]["format"] == "portrait-totem-75"
    assert reqs[0]["pixels"] == [1080, 1920]


def test_resolve_a_screen_reference():
    [r] = opencrs.resolve(spec(), "harbourline.example/retail/screen/HC-L1-002")
    assert r["venue"] == "harbour-central"
    assert [b["name"] for b in r["blocked_regions"]] == ["network-logo", "column"]
    assert r["derived"]["min_key_cap_height"]["px"] > 0
    assert isinstance(r["derived"]["max_words"]["value"], int)


def test_resolve_errors_raise_with_the_cores_message():
    with pytest.raises(opencrs.OpenCRSError) as e:
        opencrs.resolve(spec(), "a/b")
    assert str(e.value) == '"a/b" is not a format or screen reference'
    with pytest.raises(opencrs.OpenCRSError) as e:
        opencrs.resolve(spec(), "harbourline.example/nope/x")
    assert str(e.value) == 'network "nope" not found in harbourline.example'


def test_resolve_strictest_combines_the_screens_of_a_format():
    s = opencrs.resolve_strictest(spec(), "harbourline.example/retail/portrait-totem-75")
    assert s["screens"] == 2
    assert s["ref"] == "harbourline.example/retail/portrait-totem-75"
    # The farther totem sets the text size.
    assert s["min_cap_height_px"]["headline"] == 77
    assert s["sources"]["min_cap_height_px.headline"] == "HC-L1-002"
    one = opencrs.resolve_strictest(spec(), "harbourline.example/retail/screen/HC-L1-002")
    assert one["screens"] == 1
    assert one["min_cap_height_px"] == s["min_cap_height_px"]


def test_resolve_strictest_errors_raise_with_the_cores_message():
    with pytest.raises(opencrs.OpenCRSError) as e:
        opencrs.resolve_strictest(spec(), "a/b")
    assert str(e.value) == '"a/b" is not a format or screen reference'


def test_probe_reads_a_files_own_header():
    path = HARBOURLINE / "lunch-portrait.png"
    p = opencrs.probe(path)
    assert p["media_type"] == "image/png"
    assert p["pixels"] == [1080, 1920]
    assert len(p["sha256"]) == 64
    data = path.read_bytes()
    assert p["bytes"] == len(data)
    assert opencrs.probe_bytes(data) == p
    assert opencrs.probe(str(path)) == p


def test_probe_of_an_unknown_file_has_no_pixels():
    p = opencrs.probe_bytes(b"\x01\x02\x03")
    assert p["media_type"] == ""
    assert "pixels" not in p
    assert p["bytes"] == 3


def test_probe_of_a_missing_file_raises():
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.probe(HARBOURLINE / "missing.png")


NOW = "2026-10-22T12:00:00+11:00"


def without_validator(reports):
    return [{k: v for k, v in r.items() if k != "validator"} for r in reports]


def test_check_matches_reports_json_for_harbourline():
    manifest = (HARBOURLINE / "creative.json").read_text()
    probes = {
        name: opencrs.probe(HARBOURLINE / name)
        for name in ["lunch-portrait.png", "lunch-portrait-logo-right.png"]
    }
    res = opencrs.check([spec()], manifest, probes, NOW)
    assert res["notes"] == []
    want = json.loads((HARBOURLINE / "reports.json").read_text())
    assert without_validator(res["reports"]) == without_validator(want)
    assert res["reports"][0]["validator"] == {"name": "opencrs", "version": "0.1"}
    assert opencrs.check_dir(HARBOURLINE, [spec()], manifest, NOW) == {**res, "missing": []}


def test_check_dir_lists_the_files_it_could_not_find(tmp_path):
    # A file not found is judged on the manifest's own values; the caller
    # must be able to see that it happened.
    manifest = (HARBOURLINE / "creative.json").read_text()
    (tmp_path / "lunch-portrait.png").write_bytes((HARBOURLINE / "lunch-portrait.png").read_bytes())
    res = opencrs.check_dir(tmp_path, [spec()], manifest, NOW)
    assert res["missing"] == ["lunch-portrait-logo-right.png"]
    res = opencrs.check_dir(tmp_path / "typo", [spec()], manifest, NOW)
    assert res["missing"] == ["lunch-portrait.png", "lunch-portrait-logo-right.png"]


def test_check_keeps_the_offset_of_now_and_defaults_to_host_time():
    manifest = (HARBOURLINE / "creative.json").read_text()
    res = opencrs.check([spec()], manifest, {}, "2026-10-22T12:00:00+05:30")
    assert res["reports"][0]["checked_at"] == "2026-10-22T12:00:00+05:30"
    res = opencrs.check([spec()], manifest, {})
    assert res["reports"][0]["checked_at"]


CASES = sorted((ROOT / "conformance" / "0.1" / "cases").iterdir())


def within(want, got, at=""):
    """What a strictest case expects: objects by the members it gives, lists exactly."""
    if want is None:
        return
    if isinstance(want, dict):
        assert isinstance(got, dict), f"{at}: {got!r}"
        for k, v in want.items():
            assert k in got, f"{at}.{k} is missing"
            within(v, got[k], f"{at}.{k}")
    elif isinstance(want, list):
        assert isinstance(got, list) and len(got) == len(want), f"{at}: {got!r}, want {want!r}"
        for i, (w, g) in enumerate(zip(want, got)):
            within(w, g, f"{at}[{i}]")
    else:
        assert got == want, f"{at}: {got!r}, want {want!r}"


@pytest.mark.parametrize("case_dir", CASES, ids=[c.name for c in CASES])
def test_check_dir_passes_every_conformance_case(case_dir):
    c = json.loads((case_dir / "case.json").read_text())
    specs = [(case_dir / s).read_text() for s in c["specs"]]
    if c.get("strictest"):
        for want in c["strictest"]:
            if want.get("error"):
                with pytest.raises(opencrs.OpenCRSError, match=want["error"]):
                    opencrs.resolve_strictest(specs[0], want["ref"])
                continue
            got = opencrs.resolve_strictest(specs[0], want["ref"])
            within(want.get("expect"), got, want["ref"])
            if "notes" in want:
                assert len(got.get("notes", [])) == want["notes"], want["ref"]
        return
    manifest = (case_dir / c["creative"]).read_text()
    reports = opencrs.check_dir(case_dir, specs, manifest, c["now"])["reports"]
    assert len(reports) == len(c["expect"])
    for want in c["expect"]:
        [got] = [r for r in reports if r["file"] == want["file"] and r["target"] == want["target"]]
        assert got["result"] == want["result"]

        def key(k):
            return f"{k['code']}/{k['severity']}"

        # Every expected check is present; any other finding must be info.
        have = list(map(key, got["checks"]))
        for k in map(key, want["checks"]):
            assert k in have, f"missing {k}"
            have.remove(k)
        assert [k for k in have if not k.endswith("/info")] == []


def test_check_errors_raise_with_the_cores_message():
    manifest = (HARBOURLINE / "creative.json").read_text()
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.check(["{not json"], manifest, {}, NOW)
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.check([spec()], "{not json", {}, NOW)
    with pytest.raises(opencrs.OpenCRSError, match="RFC 3339"):
        opencrs.check([spec()], manifest, {}, "yesterday")
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.check([spec()], manifest, {"a.png": 1}, NOW)


def manifest_naming(name):
    m = json.loads((HARBOURLINE / "creative.json").read_text())
    m["files"] = [dict(m["files"][0], file=name)]
    return json.dumps(m)


@pytest.mark.parametrize("name", ["../x", "sub/../../x", "/etc/hosts", "/dev/zero"])
def test_check_dir_confines_files_to_the_directory(name):
    with pytest.raises(opencrs.OpenCRSError, match="inside the directory"):
        opencrs.check_dir(HARBOURLINE, [spec()], manifest_naming(name), NOW)


# Unix only: /etc/hosts, /dev/zero and FIFOs don't exist on Windows, and
# symlinks there need admin rights. The core's fs tests cover Windows paths.
@pytest.mark.skipif(sys.platform == "win32", reason="Unix paths, FIFOs and symlinks")
def test_check_dir_rejects_symlinks_out_of_the_directory_and_fifos(tmp_path):
    import os

    (tmp_path / "out").symlink_to("/etc/hosts")
    with pytest.raises(opencrs.OpenCRSError, match="outside the directory"):
        opencrs.check_dir(tmp_path, [spec()], manifest_naming("out"), NOW)
    os.mkfifo(tmp_path / "pipe")
    with pytest.raises(opencrs.OpenCRSError, match="not a regular file"):
        opencrs.check_dir(tmp_path, [spec()], manifest_naming("pipe"), NOW)
    (tmp_path / "gone").mkdir()
    with pytest.raises(opencrs.OpenCRSError, match="not a regular file"):
        opencrs.check_dir(tmp_path, [spec()], manifest_naming("gone"), NOW)


@pytest.mark.skipif(sys.platform == "win32", reason="Unix devices and FIFOs")
def test_probe_refuses_devices_and_fifos(tmp_path):
    import os

    with pytest.raises(opencrs.OpenCRSError, match="not a regular file"):
        opencrs.probe("/dev/zero")
    os.mkfifo(tmp_path / "pipe")
    with pytest.raises(opencrs.OpenCRSError, match="not a regular file"):
        opencrs.probe(tmp_path / "pipe")


def test_probe_bytes_and_probes_are_type_checked():
    with pytest.raises(TypeError):
        opencrs.probe_bytes(5)
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.check([spec()], "{}", {"a": float("nan")})
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.check([spec()], "{}", {"a": object()})
