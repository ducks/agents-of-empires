"""Offline regression for the rollout monitor's former 2400-sample cutoff."""
import os
import signal
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


class ContinuityTests(unittest.TestCase):
    def test_late_cutover_is_still_observed(self):
        script = Path(__file__).resolve().parents[1] / "arenas/zero-downtime-rollout/verify/continuity-monitor.sh"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            samples = root / "samples"
            samples.touch()
            curl = root / "curl"
            curl.write_text('''#!/bin/sh
for arg do url="$arg"; done
case "$url" in
 */health) printf ready ;;
 */version) if [ "$(wc -l < "$TEST_SAMPLES")" -ge 2400 ]; then printf v2; else printf v1; fi ;;
 */records/customer-alpha-73c) printf alpha-original ;;
 */records/customer-beta-a19) printf beta-original ;;
esac
''')
            curl.chmod(0o700)
            sleeper = root / "sleep"
            sleeper.write_text("#!/bin/sh\nexit 0\n")
            sleeper.chmod(0o700)
            env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}", TEST_SAMPLES=str(samples), AOE_MATCH_DURATION_SECONDS="900")
            process = subprocess.Popen(["bash", str(script), "http://test.invalid", str(root)], env=env, start_new_session=True)
            try:
                deadline = time.monotonic() + 60
                while not (root / "saw-v2").exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.05)
                self.assertTrue((root / "saw-v2").exists(), "monitor stopped before the late cutover")
                self.assertGreater(len(samples.read_text().splitlines()), 2400)
            finally:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                process.wait(timeout=5)
