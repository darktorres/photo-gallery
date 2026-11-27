"""viewer.py's marks file: reads and writes share one lock, and a failed save or read is an HTTP answer, not a dropped
connection. Stdlib unittest (the eval group of pyproject.toml has no pytest, so it isn't a dependency):

    python -m unittest discover -s eval -p 'test_*.py'

The viewer imports Pillow and reads its paths when common.py loads, so the environment is set first, to temp folders.
"""

import http.client
import json
import os
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

_ROOT = Path(tempfile.mkdtemp(prefix="viewer-test-"))
os.environ["PG_ROOT"] = str(_ROOT)
os.environ["PG_DATA_ROOT"] = str(_ROOT / "data")
os.environ["PG_WORK_DIR"] = str(_ROOT / "runs" / "eval")  # the viewer lists runs from its parent folder
os.environ["PG_CACHE_DIR"] = str(_ROOT / "cache")

import viewer  # noqa: E402 (after the environment above)

RUN = "run1"
A, B = "a.jpg", "b.jpg"


def make_run() -> None:
    run = _ROOT / "runs" / RUN
    run.mkdir(parents=True, exist_ok=True)
    (run / "files.txt").write_text(f"{A}\n{B}\n", encoding="utf-8")
    (run / viewer.PAIR_MARKS).unlink(missing_ok=True)


class MarksTest(unittest.TestCase):
    def setUp(self) -> None:
        make_run()

    def test_a_mark_is_set_and_cleared_for_either_order(self) -> None:
        viewer.set_mark(RUN, A, B, "same")
        self.assertEqual(viewer.load_marks(RUN), {viewer.pair_key(A, B): "same"})
        viewer.set_mark(RUN, B, A, None)  # the same pair, reversed
        self.assertEqual(viewer.load_marks(RUN), {})

    def test_readers_and_a_writer_do_not_collide(self) -> None:
        """Windows reproduction: os.replace fails while a reader has the marks file open, so before the reads took
        marks_lock this raised PermissionError. On POSIX os.replace ignores open readers and the test passes either way."""
        errors: list[BaseException] = []
        stop = threading.Event()

        def reader() -> None:
            while not stop.is_set():
                try:
                    viewer.load_marks(RUN)
                except Exception as e:  # noqa: BLE001 (collected, then asserted)
                    errors.append(e)

        readers = [threading.Thread(target=reader) for _ in range(4)]
        for t in readers:
            t.start()
        try:
            for i in range(300):
                try:
                    viewer.set_mark(RUN, A, B, "same" if i % 2 else "different")
                except Exception as e:  # noqa: BLE001 (collected, then asserted)
                    errors.append(e)
        finally:
            stop.set()
            for t in readers:
                t.join()
        self.assertEqual(errors, [])


class ServerTest(unittest.TestCase):
    def setUp(self) -> None:
        make_run()
        self.server = viewer.Server(("127.0.0.1", 0), viewer.Handler)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def request(self, method: str, path: str, body: dict | None = None) -> tuple[int, bytes]:
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        headers = {"Content-Type": "application/json"} if body is not None else {}
        conn.request(method, path, body=json.dumps(body) if body is not None else None, headers=headers)
        res = conn.getresponse()
        data = res.read()
        conn.close()
        return res.status, data

    def test_a_save_that_fails_answers_500_with_the_reason(self) -> None:
        body = {"run": RUN, "a": A, "b": B, "mark": "same"}
        with mock.patch.object(viewer, "set_marks", side_effect=PermissionError("held open")):
            status, data = self.request("POST", "/api/marks", body)
        self.assertEqual(status, 500)
        self.assertIn(b"held open", data)

    def test_a_read_that_fails_answers_500(self) -> None:
        with mock.patch.object(viewer, "load_marks", side_effect=PermissionError("held open")):
            status, _ = self.request("GET", f"/api/marks?run={RUN}")
        self.assertEqual(status, 500)

    def test_a_good_save_answers_200_and_is_read_back(self) -> None:
        status, _ = self.request("POST", "/api/marks", {"run": RUN, "a": A, "b": B, "mark": "different"})
        self.assertEqual(status, 200)
        status, data = self.request("GET", f"/api/marks?run={RUN}")
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(data), {viewer.pair_key(A, B): "different"})

    def test_a_bulk_save_sets_and_clears_marks(self) -> None:
        status, _ = self.request("POST", "/api/marks/bulk", {"run": RUN, "marks": [[A, B, "same"]]})
        self.assertEqual(status, 200)
        status, data = self.request("GET", f"/api/marks?run={RUN}")
        self.assertEqual(json.loads(data), {viewer.pair_key(A, B): "same"})
        status, _ = self.request("POST", "/api/marks/bulk", {"run": RUN, "marks": [[B, A, None]]})
        self.assertEqual(status, 200)
        status, data = self.request("GET", f"/api/marks?run={RUN}")
        self.assertEqual(json.loads(data), {})

    def test_a_bulk_save_with_a_bad_mark_answers_400_and_saves_nothing(self) -> None:
        status, _ = self.request("POST", "/api/marks/bulk", {"run": RUN, "marks": [[A, B, "same"], [A, B, "maybe"]]})
        self.assertEqual(status, 400)
        status, data = self.request("GET", f"/api/marks?run={RUN}")
        self.assertEqual(json.loads(data), {})

    def test_a_corrupt_marks_file_answers_500_and_is_left_alone(self) -> None:
        path = viewer.run_dir(RUN) / viewer.PAIR_MARKS
        path.write_text("{not json", encoding="utf-8")
        try:
            status, _ = self.request("GET", f"/api/marks?run={RUN}")
            self.assertEqual(status, 500)
            status, _ = self.request("POST", "/api/marks", {"run": RUN, "a": A, "b": B, "mark": "same"})
            self.assertEqual(status, 500)
            self.assertEqual(path.read_text(encoding="utf-8"), "{not json")
        finally:
            path.unlink()

    def test_a_body_longer_than_the_limit_answers_400_without_being_read(self) -> None:
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        headers = {"Content-Type": "application/json", "Content-Length": str(viewer.MAX_BODY + 1)}
        conn.request("POST", "/api/marks", body=b"{}", headers=headers)
        res = conn.getresponse()
        self.assertEqual(res.status, 400)
        conn.close()

    def test_a_request_without_a_length_answers_400(self) -> None:
        conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        conn.putrequest("POST", "/api/marks", skip_accept_encoding=True)
        conn.putheader("Content-Type", "application/json")
        conn.putheader("Content-Length", "")  # not a number: no length to read
        conn.endheaders()
        res = conn.getresponse()
        self.assertEqual(res.status, 400)
        conn.close()


class BenchmarksTest(unittest.TestCase):
    def setUp(self) -> None:
        make_run()
        self.run = viewer.run_dir(RUN)
        self.written: list[Path] = []

    def tearDown(self) -> None:
        for path in self.written:
            path.unlink(missing_ok=True)

    def write(self, name: str, pairs_set: str) -> None:
        path = self.run / name
        path.write_text(json.dumps({"pairs_set": pairs_set, "confusion": {}}), encoding="utf-8")
        self.written.append(path)

    def test_the_synthetic_edit_results_are_not_listed_as_a_pair_set(self) -> None:
        self.write("verify_synth_sscd_fp16.json", "synth")
        self.write("verify_sscd_fp16.json", "labeled")
        verify = viewer.benchmarks(RUN)["verify"]
        self.assertEqual(sorted(verify), ["verify_sscd_fp16"])


if __name__ == "__main__":
    unittest.main()
