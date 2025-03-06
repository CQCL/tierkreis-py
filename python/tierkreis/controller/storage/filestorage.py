import json
import os
import shutil
from pathlib import Path
from uuid import UUID, uuid4

from tierkreis.controller.models import NodeDefinition, NodeLocation, OutputLocation
from tierkreis.core.tierkreis_graph import PortID


class ControllerFileStorage:
    def __init__(
        self, workflow_id: UUID, tierkreis_directory: Path = Path("/tmp/tierkreis")
    ) -> None:
        self.workflow_dir: Path = tierkreis_directory / str(workflow_id)
        self.workflow_dir.mkdir(parents=True, exist_ok=True)

    def _definition_path(self, node_location: NodeLocation) -> Path:
        path = self.workflow_dir / str(node_location) / "definition"
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def _output_path(self, node_location: NodeLocation, port_name: PortID) -> Path:
        path = self.workflow_dir / str(node_location) / "outputs" / port_name
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def _done_path(self, node_location: NodeLocation) -> Path:
        path = self.workflow_dir / str(node_location) / "_done"
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def clean_graph_files(self) -> None:
        if self.workflow_dir.exists():
            shutil.move(self.workflow_dir, f"/tmp/{uuid4()}")

    def add_input(self, port_name: PortID, value: bytes) -> NodeLocation:
        input_loc = NodeLocation(location=[]).append_node(-1)
        path = self._output_path(input_loc, port_name)
        path.parent.mkdir(parents=True, exist_ok=True)
        with open(path, "wb+") as fh:
            fh.write(bytes(value))

        return input_loc

    def write_node_definition(
        self,
        node_location: NodeLocation,
        function_name: str,
        inputs: dict[PortID, OutputLocation],
        output_list: list[PortID],
    ) -> Path:
        node_definition_path = self._definition_path(node_location)
        node_definition = NodeDefinition(
            function_name=function_name,
            inputs={
                k: self._output_path(loc, port) for k, (loc, port) in inputs.items()
            },
            outputs={k: self._output_path(node_location, k) for k in output_list},
            done_path=self._done_path(node_location),
        )
        with open(node_definition_path, "w+") as fh:
            fh.write(node_definition.model_dump_json())

        return node_definition_path

    def read_node_definition(self, node_location: NodeLocation) -> NodeDefinition:
        node_definition_path = self._definition_path(node_location)
        with open(node_definition_path, "r") as fh:
            return NodeDefinition(**json.loads(fh.read()))

    def link_outputs(
        self,
        new_location: NodeLocation,
        new_port: PortID,
        old_location: NodeLocation,
        old_port: PortID,
    ) -> None:
        new_dir = self._output_path(new_location, new_port)
        new_dir.parent.mkdir(parents=True, exist_ok=True)
        os.link(self._output_path(old_location, old_port), new_dir)

    def is_output_ready(self, node_location: NodeLocation, output_name: PortID) -> bool:
        return self._output_path(node_location, output_name).exists()

    def write_output(
        self, node_location: NodeLocation, output_name: PortID, value: bytes
    ) -> None:
        with open(self._output_path(node_location, output_name), "wb+") as fh:
            fh.write(bytes(value))

    def read_output(self, node_location: NodeLocation, output_name: PortID) -> bytes:
        with open(self._output_path(node_location, output_name), "rb") as fh:
            return fh.read()

    def is_node_started(self, node_location: NodeLocation) -> bool:
        return Path(self._definition_path(node_location)).exists()

    def is_node_finished(self, node_location: NodeLocation) -> bool:
        return self._done_path(node_location).exists()

    def mark_node_finished(self, node_location: NodeLocation) -> None:
        self._done_path(node_location).touch()
