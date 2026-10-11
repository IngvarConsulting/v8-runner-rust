"""Exercise the public shell installer with isolated homes and release transport."""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


INSTALLER = Path(__file__).resolve().parents[2] / "docs/site/install.sh"
PACKAGE = "v8-runner-linux-x86_64-musl"


class InstallerTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.mocks = self.root / "mocks"
        self.mocks.mkdir()
        binary = b"#!/bin/sh\nprintf 'v8-runner fixture\\n'\n"
        archive = self.root / "archive.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            member = tarfile.TarInfo(PACKAGE + "/v8-runner")
            member.size = len(binary)
            member.mode = 0o755
            tar.addfile(member, io.BytesIO(binary))
        self.asset = {"name": PACKAGE + ".tar.gz", "role": "portable-archive",
                      "targetTriple": "x86_64-unknown-linux-musl", "format": "tar.gz",
                      "size": archive.stat().st_size,
                      "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
                      "buildAttestationRequired": True,
                      "binary": {"path": PACKAGE + "/v8-runner", "size": len(binary),
                                 "sha256": hashlib.sha256(binary).hexdigest()}}
        self.manifest = {"schemaVersion": 2, "release": {
            "repository": "https://github.com/IngvarConsulting/v8-runner-rust",
            "sourceTag": "v0.15.0", "sourceCommit": "a" * 40,
            "builderWorkflow": ".github/workflows/release.yml",
            "runnerEnvironment": "github-hosted"}, "assets": [self.asset]}
        self.write_manifest()
        self.mock("uname", "#!/bin/sh\ncase \"$1\" in -s) echo Linux;; -m) echo \"${TEST_ARCH:-x86_64}\";; esac\n")
        self.mock("curl", "#!" + sys.executable + "\n" +
                  "import os,sys,shutil\nfrom pathlib import Path\n" +
                  "args=sys.argv[1:];url=next(x for x in args if x.startswith('https:'))\n" +
                  "root=Path(os.environ['TEST_RELEASE'])\n" +
                  "with (root/'requests').open('a') as f:f.write(url+'\\n')\n" +
                  "if os.environ.get('TEST_FAIL_DOWNLOAD'):sys.exit(22)\n" +
                  "src=root/('manifest.json' if url.endswith('.json') else 'archive.tar.gz')\n" +
                  "shutil.copyfile(src,args[args.index('-o')+1])\n")
        self.env = dict(os.environ, HOME=str(self.home), SHELL="/bin/zsh",
                        PATH=str(self.mocks) + os.pathsep + os.environ["PATH"],
                        TEST_RELEASE=str(self.root))
        self.env.pop("V8_RUNNER_INSTALL_DIR", None)
        self.installed = self.home / ".local/bin/v8-runner"

    def mock(self, name, source):
        path = self.mocks / name
        path.write_text(source)
        path.chmod(0o755)

    def write_manifest(self):
        (self.root / "manifest.json").write_text(json.dumps(self.manifest, indent=2) + "\n")

    def run_install(self, **extra):
        return subprocess.run(["sh", str(INSTALLER)], env=dict(self.env, **extra),
                              capture_output=True, text=True, timeout=15)

    def assert_preserved(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.installed.read_bytes(), b"previous binary")
        self.assertFalse((self.home / ".zshrc").exists())

    def previous_binary(self):
        self.installed.parent.mkdir(parents=True)
        self.installed.write_bytes(b"previous binary")

    def test_success_and_repeat_do_not_duplicate_path(self):
        for _ in range(2):
            result = self.run_install()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(b"fixture", self.installed.read_bytes())
        self.assertEqual((self.home / ".zshrc").read_text().count('export PATH='), 1)
        urls = (self.root / "requests").read_text().splitlines()
        self.assertIn('/download/v0.15.0/' + PACKAGE + '.tar.gz', urls[1])
        self.assertFalse(list(self.installed.parent.glob('.v8-runner.*')))

    def test_archive_checksum_failure_preserves_installation(self):
        self.previous_binary()
        self.asset["sha256"] = "0" * 64
        self.write_manifest()
        self.assert_preserved(self.run_install())

    def test_binary_checksum_failure_preserves_installation(self):
        self.previous_binary()
        self.asset["binary"]["sha256"] = "0" * 64
        self.write_manifest()
        self.assert_preserved(self.run_install())

    def test_missing_archive_record(self):
        self.previous_binary()
        self.manifest["assets"] = []
        self.write_manifest()
        self.assert_preserved(self.run_install())

    def test_duplicate_archive_record(self):
        self.previous_binary()
        self.manifest["assets"].append(self.asset.copy())
        self.write_manifest()
        self.assert_preserved(self.run_install())

    def test_unsupported_architecture(self):
        self.previous_binary()
        self.assert_preserved(self.run_install(TEST_ARCH="aarch64"))
        self.assertFalse((self.root / "requests").exists())

    def test_download_failure(self):
        self.previous_binary()
        self.assert_preserved(self.run_install(TEST_FAIL_DOWNLOAD="1"))


if __name__ == "__main__":
    unittest.main()
