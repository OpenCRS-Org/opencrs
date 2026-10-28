import gzip
import http.server
import json
import shutil
import subprocess
import sys
import threading
import time
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
            assert want.get("expect") is not None or "notes" in want, "the case expects nothing"
            within(want.get("expect"), got, want["ref"])
            for path in want.get("absent", []):
                at = got
                for k in path.split("."):
                    at = at.get(k) if isinstance(at, dict) else None
                assert at is None, f"{path} is there"
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


# The calls that take plain data are compared with the WebAssembly build, the
# same core as the browser and Go run: the results must be equal, not merely
# alike. That needs Node, which the tests skip without.
WASM_CALL = Path(__file__).with_name("wasm_call.mjs")
needs_node = pytest.mark.skipif(
    shutil.which("node") is None or not (ROOT / "opencrs.wasm").exists(),
    reason="needs node and the repo's opencrs.wasm",
)


def via_wasm(op: str, request: object) -> object:
    out = subprocess.run(
        ["node", str(WASM_CALL), op],
        input=json.dumps(request),
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def test_every_operation_in_the_manifest_is_exported_and_nothing_else_is():
    manifest = json.loads((ROOT / "parity" / "operations.json").read_text())
    listed = set(manifest["helpers"]["python"])
    for op, entry in manifest["operations"].items():
        names = entry["python"]
        if names is None:
            assert f"python.{op}" in manifest["exceptions"], f"{op}: no name and no exception"
            continue
        assert names, f"{op}: python needs a list of names or null"
        assert f"python.{op}" not in manifest["exceptions"], f"{op}: has names and an exception"
        for name in names:
            assert callable(getattr(opencrs, name, None)), f"{op}: python has no {name}"
            listed.add(name)
    assert set(opencrs.__all__) == listed, "an export is missing from the manifest"


def test_legibility_is_the_specs_worked_example():
    res = opencrs.legibility([2, 6], screen={"width_mm": 940, "width_px": 1080}, dwell_s=4)
    assert res["ratio"] == 150
    assert res["key"] == {"mm": 40, "px": 46, "px_exact": 45.96, "basis": "at 6 m, ratio 150"}
    assert res["legal"] == {"mm": 13.3, "px": 16, "px_exact": 15.32, "basis": "at 2 m, ratio 150"}
    assert res["words"] == {"value": 8, "basis": "4 s dwell at 2 words/s"}
    assert "px" not in opencrs.legibility((3, 10), ratio=250)["key"]


@needs_node
def test_legibility_equals_the_webassembly_build():
    for kwargs, req in [
        (
            {"screen": {"width_mm": 940, "width_px": 1080}, "dwell_s": 4},
            {"distance_m": [2, 6], "screen": {"width_mm": 940, "width_px": 1080}, "dwell_s": 4},
        ),
        ({"ratio": 250}, {"distance_m": [2, 6], "ratio": 250}),
        ({"rate": 3, "dwell_s": 2.5}, {"distance_m": [2, 6], "rate": 3, "dwell_s": 2.5}),
    ]:
        assert opencrs.legibility([2, 6], **kwargs) == via_wasm("legibility", req)


def test_legibility_errors_raise_with_the_cores_message():
    with pytest.raises(opencrs.OpenCRSError, match=r"the near distance \(6 m\) is past the far one \(2 m\)"):
        opencrs.legibility([6, 2])
    with pytest.raises(opencrs.OpenCRSError, match="ratio must be a number above zero"):
        opencrs.legibility([2, 6], ratio=0)
    with pytest.raises(opencrs.OpenCRSError, match="distance_m must be"):
        opencrs.legibility([2])
    with pytest.raises(opencrs.OpenCRSError, match="screen must be"):
        opencrs.legibility([2, 6], screen={"width_mm": 940})


def test_dwell_gives_the_glance_and_each_packs_limits():
    res = opencrs.dwell(80, [20, 150], rule_packs=["au-nsw-transport-corridor@2017-11"])
    assert res["glance"]["words"] == 4
    assert res["glance"]["read_from_m"] == 64.44
    assert res["key"]["mm"] == 429.6
    assert res["packs"][0]["min_dwell_s"] == 25
    assert res["packs"][0]["motion_allowed"] is False
    with pytest.raises(opencrs.OpenCRSError, match=r"unknown rule pack x@1"):
        opencrs.dwell(80, [20, 150], rule_packs=["x@1"])


@needs_node
def test_dwell_equals_the_webassembly_build():
    packs = ["au-nsw-transport-corridor@2017-11"]
    assert opencrs.dwell(80, [20, 150], rule_packs=packs) == via_wasm(
        "dwell", {"speed_kmh": 80, "distance_m": [20, 150], "rule_packs": packs}
    )
    screen = {"width_mm": 940, "width_px": 1080}
    assert opencrs.dwell(60, [20, 150], screen=screen, ratio=200, rate=3) == via_wasm(
        "dwell",
        {"speed_kmh": 60, "distance_m": [20, 150], "screen": screen, "ratio": 200, "rate": 3},
    )


def test_rule_packs_lists_the_built_in_packs():
    res = opencrs.rule_packs()
    assert len(res["rule_packs"]) == 6
    assert all(p["ref"] and p["title"] and p["status"] for p in res["rule_packs"])
    assert [p["ref"] for p in res["rule_packs"]] == sorted(p["ref"] for p in res["rule_packs"])


@needs_node
def test_rule_packs_equals_the_webassembly_build():
    assert opencrs.rule_packs() == via_wasm("rule_packs", {})


INDEX_URL = "https://harbourline.example/.well-known/opencrs.json"
DOC_URL = "https://harbourline.example/opencrs/retail.screens.json"
SERVED = {"cache_control": "max-age=60", "etag": '"v1"', "content_type": "application/json"}


def index() -> str:
    return (HARBOURLINE / ".well-known" / "opencrs.json").read_text()


def test_publisher_asks_for_the_index_then_checks_what_it_is_given():
    first = opencrs.publisher("harbourline.example", {}, NOW)
    assert first["wanted"] == [INDEX_URL]
    assert first["wanted_detail"] == [{"url": INDEX_URL, "kind": "index", "same_host": True}]

    pages = {
        INDEX_URL: {"status": 200, "body": index(), **SERVED},
        DOC_URL: {"status": 200, "body": spec().encode(), **SERVED, "allow_origin": "*"},
    }
    res = opencrs.publisher("harbourline.example", pages, NOW)
    assert res["wanted"] == []
    assert res["errors"] == 0
    [doc] = res["discovery"]["documents"]
    assert (doc["status"], doc["screens"]) == ("trusted", 3)

    part = {INDEX_URL: pages[INDEX_URL]}
    missing = opencrs.publisher("harbourline.example", part, NOW, "skipped for now")
    assert missing["wanted"] == [DOC_URL]
    assert "skipped for now" in missing["discovery"]["documents"][0]["reason"]
    broke = {**part, DOC_URL: {"status": 0, "error": "connection reset"}}
    reason = opencrs.publisher("harbourline.example", broke, NOW)["discovery"]["documents"][0]["reason"]
    assert "connection reset" in reason


@needs_node
def test_publisher_equals_the_webassembly_build():
    pages = {
        INDEX_URL: {"status": 200, "body": index(), **SERVED},
        DOC_URL: {"status": 200, "body": spec(), **SERVED, "allow_origin": "*"},
    }
    bare = {INDEX_URL: {"status": 200, "body": index()}, DOC_URL: {"status": 200, "body": spec()}}
    part = {INDEX_URL: pages[INDEX_URL]}
    gone = {**part, DOC_URL: {"status": 404}}
    for given, missing in [({}, None), (pages, None), (bare, None), (part, "skipped for now"), (gone, None)]:
        req = {"domain": "harbourline.example", "pages": given, "now": NOW, "missing": missing or ""}
        assert opencrs.publisher("harbourline.example", given, NOW, missing) == via_wasm("publisher", req)
    # Bodies given as bytes are the same as the text.
    as_bytes = {u: {**p, "body": p["body"].encode()} for u, p in pages.items()}
    assert opencrs.publisher("harbourline.example", as_bytes, NOW) == opencrs.publisher(
        "harbourline.example", pages, NOW
    )


def test_publisher_advice_is_about_how_the_files_were_served():
    bare = {INDEX_URL: {"status": 200, "body": index()}, DOC_URL: {"status": 200, "body": spec()}}
    res = opencrs.publisher("harbourline.example", bare, NOW)
    assert res["warnings"] >= 2
    assert any(a["url"] == DOC_URL and "Cache-Control" in a["message"] for a in res["advice"])
    # Not UTF-8: arrives as it is, and is not an index.
    bad = opencrs.publisher("harbourline.example", {INDEX_URL: {"status": 200, "body": b"<\xff"}}, NOW)
    assert bad["errors"] == 1 and bad["error"]


def test_publisher_raises_for_bad_arguments():
    with pytest.raises(opencrs.OpenCRSError, match="not an RFC 3339 time"):
        opencrs.publisher("harbourline.example", {}, "yesterday")
    with pytest.raises(TypeError):
        opencrs.publisher("harbourline.example", {INDEX_URL: {"status": 200, "body": 5}})


def test_fetch_publisher_says_what_the_check_says_of_a_domain_that_cannot_publish():
    res = opencrs.fetch_publisher("not a domain", now=NOW)
    assert "isn't a publisher domain" in res["error"]
    assert res["errors"] == 1
    assert res == opencrs.publisher("not a domain", {}, NOW)
    # Local names can't publish either, so a loopback server can't be pointed at.
    assert "isn't a publisher domain" in opencrs.fetch_publisher("localhost")["error"]
    assert "isn't a publisher domain" in opencrs.fetch_publisher("127.0.0.1:1")["error"]
    with pytest.raises(opencrs.OpenCRSError, match="not an RFC 3339 time"):
        opencrs.fetch_publisher("harbourline.example", now="later")
    with pytest.raises(opencrs.OpenCRSError, match="timeout"):
        opencrs.fetch_publisher("harbourline.example", timeout=-1)


@needs_node
def test_fetch_publisher_equals_the_webassembly_build_for_a_failed_index():
    req = {"domain": "not a domain", "pages": {}, "now": NOW, "missing": ""}
    assert opencrs.fetch_publisher("not a domain", now=NOW) == via_wasm("publisher", req)


class Local:
    """A publisher on a local port, answering by path."""

    def __init__(self, handler):
        self.seen = []
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                outer.seen.append(self.path)
                status, headers, body = handler(self.path, outer.addr)
                self.send_response(status)
                for k, v in headers.items():
                    self.send_header(k, v)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.addr = f"127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.server.shutdown()
        self.server.server_close()


def test_discover_reads_a_local_publisher_and_lets_other_threads_run():
    doc = spec().encode()

    def handler(path, addr):
        if path == "/.well-known/opencrs.json":
            body = json.dumps(
                {
                    "opencrs": "0.1",
                    "publisher": {"id": "harbourline.example", "name": "Harbourline"},
                    "documents": [
                        {"url": f"http://{addr}/retail.screens.json", "network": "retail"},
                        {"url": f"http://{addr}/slow.screens.json", "network": "retail"},
                        {"url": f"http://{addr}/metadata.screens.json", "network": "retail"},
                    ],
                }
            ).encode()
            return 200, {"Content-Type": "application/json"}, body
        if path == "/retail.screens.json":
            return 200, {"Content-Encoding": "gzip", "Cache-Control": "max-age=3600"}, gzip.compress(doc)
        if path == "/slow.screens.json":
            time.sleep(0.4)
            return 200, {}, doc
        if path == "/metadata.screens.json":
            return 302, {"Location": "https://169.254.169.254/latest/meta-data/"}, b""
        return 404, {}, b""

    with Local(handler) as srv:
        ticks = 0
        stop = threading.Event()

        def count():
            nonlocal ticks
            while not stop.is_set():
                ticks += 1
                time.sleep(0.01)

        t = threading.Thread(target=count)
        t.start()
        try:
            res = opencrs.discover(srv.addr, unsafe_allow_local_http=True, now=NOW)
        finally:
            stop.set()
            t.join()
        assert ticks >= 5, f"another thread ran only {ticks} times in 400 ms: the GIL was held"
        assert res["index"]["publisher"]["name"] == "Harbourline"
        by = {d["url"].rsplit("/", 1)[1]: d for d in res["documents"]}
        assert by["retail.screens.json"]["status"] == "trusted"
        assert by["retail.screens.json"]["screens"] == 3, "the gzipped body was read"
        assert by["retail.screens.json"]["cache_control"] == "max-age=3600"
        assert by["slow.screens.json"]["status"] == "trusted"
        # A redirect to a cloud metadata address is refused, and nothing is sent there.
        assert by["metadata.screens.json"]["status"] == "failed"
        assert "refusing to connect to 169.254.169.254" in by["metadata.screens.json"]["reason"]
        # Without local testing, the local address isn't a publisher at all.
        with pytest.raises(opencrs.OpenCRSError, match="isn't a publisher domain"):
            opencrs.discover(srv.addr, now=NOW)
        assert "/.well-known/opencrs.json" in srv.seen
    # An index that can't be fetched raises.
    with pytest.raises(opencrs.OpenCRSError):
        opencrs.discover("127.0.0.1:1", unsafe_allow_local_http=True)


def test_huge_or_odd_numbers_in_options_are_clamped_or_errors_not_panics():
    # Clamped: these ask for more than allowed, which is the most allowed.
    res = opencrs.fetch_publisher(
        "not a domain",
        max_document_bytes=2**64 - 1,
        max_total_bytes=10**30,
        max_rounds=2**64 - 1,
        max_pages=10**30,
        timeout=1e300,
        total_timeout=1e300,
        now=NOW,
    )
    assert "isn't a publisher domain" in res["error"]
    # Errors, as ValueError: not numbers of seconds, or negative.
    for bad in (float("nan"), float("inf"), -1.0):
        with pytest.raises(opencrs.OpenCRSError, match="number of seconds"):
            opencrs.fetch_publisher("not a domain", timeout=bad)
        with pytest.raises(ValueError, match="number of seconds"):
            opencrs.fetch_publisher("not a domain", total_timeout=bad)
        with pytest.raises(opencrs.OpenCRSError, match="number of seconds"):
            opencrs.discover("not a domain", timeout=bad)
    for name in ("max_document_bytes", "max_total_bytes", "max_rounds", "max_pages"):
        with pytest.raises(opencrs.OpenCRSError, match="zero or more"):
            opencrs.fetch_publisher("not a domain", **{name: -1})


def test_the_total_time_limit_stops_a_publisher_that_never_finishes():
    class Drip(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Length", "100000000")
            self.end_headers()
            try:
                while True:
                    self.wfile.write(b" ")
                    self.wfile.flush()
                    time.sleep(0.02)
            except OSError:
                pass

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Drip)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    addr = f"127.0.0.1:{server.server_address[1]}"
    try:
        t = time.monotonic()
        with pytest.raises(opencrs.OpenCRSError, match="time limit for the whole check"):
            opencrs.discover(addr, unsafe_allow_local_http=True, total_timeout=0.5, now=NOW)
        took = time.monotonic() - t
        assert 0.4 <= took < 3, f"took {took:.2f} s"
    finally:
        server.shutdown()
        server.server_close()
