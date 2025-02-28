from pathlib import Path
from typing import Protocol


class ControllerExecutor(Protocol):
    def run(self, launcher_name: str, node_definition_path: Path) -> None: ...
