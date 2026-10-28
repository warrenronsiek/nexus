# @feature installation
# @spec docs/features/installation.md
import hashlib
import tempfile
import unittest
from pathlib import Path

from scripts.release_manifest import release_manifest


class ReleaseManifestTests(unittest.TestCase):
    def test_complete_release_binds_each_platform_and_image_to_one_revision(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin"]
            for target in targets:
                (directory / f"nexus-{target}").write_bytes(target.encode())
            for target in targets[:2]:
                (directory / f"nexus-service-{target}.tar").write_bytes(b"container image")
                (directory / f"image-id-{target}.txt").write_text("sha256:" + "a" * 64)
            manifest = release_manifest(directory, "b" * 40, "0.2.0")
            self.assertEqual(manifest.revision, "b" * 40)
            self.assertEqual(len(manifest.binaries), 4)
            self.assertEqual(len(manifest.images), 2)
            self.assertEqual(manifest.binaries[0].sha256, hashlib.sha256(targets[0].encode()).hexdigest())
            self.assertEqual(manifest.images[0].image_id, "sha256:" + "a" * 64)

    def test_partial_or_unbound_releases_are_not_publishable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                release_manifest(Path(temporary), "not-a-revision", "0.2.0")
            with self.assertRaises(FileNotFoundError):
                release_manifest(Path(temporary), "b" * 40, "0.2.0")


if __name__ == "__main__":
    unittest.main()
