#!/usr/bin/env python3
# @feature installation
# @spec docs/features/installation.md
# @boundary release-manifest-json: encode verified build artifacts into a closed versioned release manifest.
"""Bind a complete set of release artifacts to one exact main revision."""
import argparse
import hashlib
import json
import re
from dataclasses import asdict, dataclass
from pathlib import Path

TARGETS = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin")


@dataclass(frozen=True)
class Artifact:
    target: str
    name: str
    sha256: str
    size: int


@dataclass(frozen=True)
class ImageArtifact(Artifact):
    image_id: str


@dataclass(frozen=True)
class ReleaseManifest:
    format_version: int
    version: str
    revision: str
    binaries: list[Artifact]
    images: list[ImageArtifact]


def artifact(directory: Path, target: str, name: str) -> Artifact:
    path = directory / name
    if path.is_symlink() or not path.is_file():
        raise FileNotFoundError(f"release artifact is not a regular file: {name}")
    size = path.stat().st_size
    if not 0 < size <= 512 * 1024 * 1024:
        raise ValueError(f"release artifact size is unsupported: {name}")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return Artifact(target, name, digest.hexdigest(), size)


def image_artifact(directory: Path, target: str) -> ImageArtifact:
    archive = artifact(directory, target, f"nexus-service-{target}.tar")
    image_id = (directory / f"image-id-{target}.txt").read_text().strip()
    if re.fullmatch(r"sha256:[0-9a-f]{64}", image_id) is None:
        raise ValueError(f"invalid image identity for {target}")
    return ImageArtifact(**asdict(archive), image_id=image_id)


def release_manifest(directory: Path, revision: str, version: str) -> ReleaseManifest:
    if re.fullmatch(r"[0-9a-f]{40}", revision) is None:
        raise ValueError("a release requires the full main revision")
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", version) is None:
        raise ValueError("a release requires a semantic package version")
    binaries = [artifact(directory, target, f"nexus-{target}") for target in TARGETS]
    images = [image_artifact(directory, target) for target in TARGETS[:2]]
    return ReleaseManifest(1, version, revision, binaries, images)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    manifest = release_manifest(args.directory, args.revision, args.version)
    (args.directory / "manifest.json").write_text(json.dumps(asdict(manifest), indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
