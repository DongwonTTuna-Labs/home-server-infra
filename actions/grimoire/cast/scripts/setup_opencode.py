#!/usr/bin/env python3
from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

STAGE = "grimoire-opencode"
PACKAGE_NAME = "opencode-ai"
PACKAGE_VERSION = "1.17.7"
PACKAGE_SPEC = f"{PACKAGE_NAME}@{PACKAGE_VERSION}"


class ProvisionError(Exception):
    def __init__(self, category: str) -> None:
        super().__init__(category)
        self.category: str = category


def write_outputs(values: dict[str, str]) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT", "")
    if not output_path:
        return
    with pathlib.Path(output_path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            _ = handle.write(f"{key}={value}\n")


def append_github_path(path: pathlib.Path) -> None:
    github_path = os.environ.get("GITHUB_PATH", "")
    if not github_path:
        return
    with pathlib.Path(github_path).open("a", encoding="utf-8") as handle:
        _ = handle.write(str(path) + "\n")


def opencode_version(opencode_path: str, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        [opencode_path, "--version"],
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=False,
    )
    if completed.returncode != 0:
        raise ProvisionError("runtime-failed:opencode-command-failed")
    version = completed.stdout.strip().splitlines()[0] if completed.stdout.strip() else ""
    if not version:
        raise ProvisionError("runtime-failed:opencode-command-failed")
    return version[:120]


def install_with_npm() -> tuple[str, pathlib.Path, str]:
    npm_path = shutil.which("npm")
    if npm_path is None:
        raise ProvisionError("missing-runtime:npm-unavailable")

    runner_temp = pathlib.Path(os.environ.get("RUNNER_TEMP") or tempfile.gettempdir()).resolve()
    install_root = runner_temp / "grimoire-opencode-runtime" / PACKAGE_SPEC
    bin_dir = install_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env.update(
        {
            "NPM_CONFIG_AUDIT": "false",
            "NPM_CONFIG_FUND": "false",
            "NPM_CONFIG_UPDATE_NOTIFIER": "false",
            "PATH": str(bin_dir) + os.pathsep + env.get("PATH", ""),
        }
    )
    completed = subprocess.run(
        [
            npm_path,
            "install",
            "--global",
            "--prefix",
            str(install_root),
            PACKAGE_SPEC,
            "--no-audit",
            "--no-fund",
            "--loglevel=error",
        ],
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=300,
        check=False,
    )
    if completed.returncode != 0:
        raise ProvisionError("runtime-failed:opencode-install-failed")

    opencode_path = shutil.which("opencode", path=env["PATH"])
    if opencode_path is None:
        raise ProvisionError("missing-runtime:opencode-unavailable")
    version = opencode_version(opencode_path, env)
    append_github_path(bin_dir)
    return opencode_path, bin_dir, version


def run() -> int:
    existing_path = shutil.which("opencode")
    source = "existing"
    bin_dir = pathlib.Path(existing_path).resolve().parent if existing_path else pathlib.Path("")
    try:
        if existing_path is not None:
            try:
                opencode_path = existing_path
                version = opencode_version(opencode_path)
            except ProvisionError:
                source = "npm"
                opencode_path, bin_dir, version = install_with_npm()
        else:
            source = "npm"
            opencode_path, bin_dir, version = install_with_npm()
        write_outputs(
            {
                "status": "ok",
                "source": source,
                "package": PACKAGE_NAME,
                "package_version": PACKAGE_VERSION,
                "opencode_version": version,
                "bin_dir": str(bin_dir),
            }
        )
        print(f"{STAGE}: status=ok source={source} package={PACKAGE_SPEC} opencode_version={version}")
        return 0
    except ProvisionError as exc:
        write_outputs({"status": "blocked", "blocked_reason_category": exc.category, "package": PACKAGE_NAME, "package_version": PACKAGE_VERSION})
        print(f"::error title=OpenCode runtime provisioning failed::{exc.category}", file=sys.stderr)
        print(f"{STAGE}: status=blocked reason={exc.category}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(run())
