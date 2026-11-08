"""viewer.py's marks file: reads and writes share one lock, and a failed save or read is an HTTP answer, not a dropped
connection. Stdlib unittest (eval/requirements.txt is a frozen freeze, so pytest isn't a dependency):

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


if __name__ == "__main__":
    unittest.main()
