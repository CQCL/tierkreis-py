from pathlib import Path
from typing import Protocol


from tierkreis.controller.models import NodeLocation, NodeDefinition, OutputLocation
from tierkreis.core.tierkreis_graph import PortID
from tierkreis.core.protos.tierkreis.v1alpha1.graph import Value


class ControllerStorage(Protocol):
    def write_node_definition(
        self,
        node_location: NodeLocation,
        function_name: str,
        inputs: dict[PortID, OutputLocation],
        output_list: list[PortID],
    ) -> Path: ...

    def read_node_definition(self, node_location: NodeLocation) -> NodeDefinition: ...

    def mark_node_finished(self, node_location: NodeLocation) -> None: ...

    def is_node_finished(self, node_location: NodeLocation) -> bool: ...

    def link_outputs(
        self,
        new_location: NodeLocation,
        new_port: PortID,
        old_location: NodeLocation,
        old_port: PortID,
    ) -> None: ...

    def write_output(
        self, node_location: NodeLocation, output_name: PortID, value: Value
    ) -> None: ...

    def read_output(
        self, node_location: NodeLocation, output_name: PortID
    ) -> Value: ...

    def is_node_started(self, node_location: NodeLocation) -> bool: ...
