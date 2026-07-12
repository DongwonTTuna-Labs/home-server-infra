# /// script
# requires-python = ">=3.12"
# ///
# How to run: python /opt/paca/enforce_xhigh.py

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Final, override


DEFAULT_BUILDER_PATH: Final = Path("/app/src/agent/builder.py")
BUILDER_PATH_ENV: Final = "PACA_BUILDER_PATH"
XHIGH_ARGUMENT: Final = '        reasoning_effort="xhigh",\n'
CALL_SUFFIX: Final = "        stream=True,\n    )"


@dataclass(frozen=True, slots=True)
class PatchContractError(RuntimeError):
    target: Path
    detail: str

    @override
    def __str__(self) -> str:
        return f"cannot enforce xhigh in {self.target}: {self.detail}"


def patch_builder(target: Path) -> None:
    source = target.read_text(encoding="utf-8")
    if XHIGH_ARGUMENT in source:
        return
    if "reasoning_effort=" in source:
        raise PatchContractError(target, "an unsupported reasoning_effort is already configured")
    if source.count(CALL_SUFFIX) != 1:
        raise PatchContractError(target, "upstream build_llm contract drifted")

    _ = target.write_text(
        source.replace(CALL_SUFFIX, f"        stream=True,\n{XHIGH_ARGUMENT}    )"),
        encoding="utf-8",
    )


def main() -> None:
    patch_builder(Path(os.environ.get(BUILDER_PATH_ENV, DEFAULT_BUILDER_PATH)))


if __name__ == "__main__":
    main()
