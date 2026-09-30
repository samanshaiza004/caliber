#!/usr/bin/env python3
"""Resolve and smoke-test the checked-in consumer fixture in a clean directory."""

from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_ROOT = REPO_ROOT / "examples" / "fresh-consumer"


def run(command: list[str], cwd: Path) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def library_path(target_root: Path) -> Path:
    target = target_root / "debug"
    system = platform.system()
    if system == "Windows":
        return target / "caliber_ffi.dll"
    if system == "Darwin":
        return target / "libcaliber_ffi.dylib"
    if system == "Linux":
        return target / "libcaliber_ffi.so"
    raise RuntimeError(f"unsupported CI platform: {system}")


def build_c_consumer(dependency_root: Path, output: Path) -> None:
    source = dependency_root / "examples" / "c-counter" / "lifecycle.c"
    header = dependency_root / "include"

    if os.name == "nt" and shutil.which("cl"):
        command = [
            "cl",
            "/nologo",
            "/std:c11",
            "/W4",
            "/WX",
            "/I",
            str(header),
            str(source),
            f"/Fe:{output}",
            f"/Fo:{output.with_suffix('.obj')}",
        ]
    elif os.name == "nt":
        compiler = shutil.which("gcc")
        if compiler is None:
            raise RuntimeError("Windows C consumer requires cl or gcc on PATH")
        command = [
            compiler,
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
            str(header),
            str(source),
            "-o",
            str(output),
        ]
    else:
        compiler = shutil.which("cc")
        if compiler is None:
            raise RuntimeError("C consumer requires cc on PATH")
        command = [
            compiler,
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pthread",
            "-I",
            str(header),
            str(source),
        ]
        if platform.system() == "Linux":
            command.append("-ldl")
        command.extend(["-o", str(output)])

    run(command, output.parent)


def main() -> None:
    fixture_lock = json.loads((FIXTURE_ROOT / "dependencies.lock.json").read_text())
    pinned_revision = fixture_lock["caliber"]["revision"]
    cargo_exe = shutil.which("cargo")
    if cargo_exe is None:
        raise RuntimeError("cargo is required on PATH")

    cli_name = "caliber.exe" if os.name == "nt" else "caliber"
    caliber_cli = REPO_ROOT / "target" / "debug" / cli_name
    if not caliber_cli.is_file():
        raise RuntimeError(f"build the Caliber CLI first: {caliber_cli}")

    temporary_parent = os.environ.get("RUNNER_TEMP")
    with tempfile.TemporaryDirectory(
        prefix="caliber-fresh-consumer-", dir=temporary_parent
    ) as temporary_directory:
        consumer_root = Path(temporary_directory)
        shutil.copytree(FIXTURE_ROOT, consumer_root, dirs_exist_ok=True)

        run(
            [str(caliber_cli), "sync", "--project-root", str(consumer_root)],
            REPO_ROOT,
        )

        dependency_root = consumer_root / ".deps" / "caliber"
        target_root = consumer_root / "build" / "caliber"
        actual_revision = subprocess.check_output(
            ["git", "-C", str(dependency_root), "rev-parse", "HEAD"],
            text=True,
        ).strip()
        if actual_revision != pinned_revision:
            raise RuntimeError(
                f"resolved {actual_revision}, expected pinned {pinned_revision}"
            )

        run(
            [
                cargo_exe,
                "build",
                "-p",
                "caliber-ffi",
                "--manifest-path",
                str(dependency_root / "Cargo.toml"),
                "--target-dir",
                str(target_root),
            ],
            consumer_root,
        )

        run(
            [str(caliber_cli), "doctor", "--project-root", str(consumer_root)],
            consumer_root,
        )
        run(
            [str(caliber_cli), "check", "--project-root", str(consumer_root)],
            consumer_root,
        )

        library = library_path(target_root)
        if not library.is_file():
            raise RuntimeError(f"expected shared library was not built: {library}")

        c_binary = consumer_root / (
            "caliber-c-lifecycle.exe" if os.name == "nt" else "caliber-c-lifecycle"
        )
        build_c_consumer(dependency_root, c_binary)
        run([str(c_binary), str(library)], consumer_root)
        run(
            [
                sys.executable,
                str(dependency_root / "examples" / "python-ctypes" / "lifecycle.py"),
                str(library),
            ],
            consumer_root,
        )

    print(f"Fresh consumer passed for pinned Caliber {pinned_revision}.")


if __name__ == "__main__":
    main()
