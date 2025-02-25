from enum import Enum
from pathlib import Path

from pydantic import BaseModel

from tierkreis.core.tierkreis_graph import PortID
from tierkreis.exceptions import TierkreisError


class NodeDefinition(BaseModel):
    function_name: str
    inputs: dict[str, Path]
    outputs: dict[str, Path]
    done_path: Path


class NodeType(Enum):
    NODE = 0
    LOOP = 1
    MAP = 2

    def __str__(self) -> str:
        match self:
            case NodeType.NODE:
                return "N"
            case NodeType.LOOP:
                return "L"
            case NodeType.MAP:
                return "M"


class NodeStep(BaseModel):
    node_type: NodeType
    idx: int

    def __str__(self) -> str:
        return f"{self.node_type}{self.idx}"


class NodeLocation(BaseModel):
    location: list[NodeStep]

    def append_node(self, idx: int) -> "NodeLocation":
        return NodeLocation(
            location=[x for x in self.location]
            + [NodeStep(node_type=NodeType.NODE, idx=idx)]
        )

    def append_loop(self, idx: int) -> "NodeLocation":
        return NodeLocation(
            location=[x for x in self.location]
            + [NodeStep(node_type=NodeType.LOOP, idx=idx)]
        )

    def append_map(self, idx: int) -> "NodeLocation":
        return NodeLocation(
            location=[x for x in self.location]
            + [NodeStep(node_type=NodeType.MAP, idx=idx)]
        )

    def peek_node(self) -> int:
        frame = self.location[len(self.location) - 1]
        if frame.node_type != NodeType.NODE:
            raise TierkreisError(f"Location {self} does not end in an N.")
        return frame.idx

    def __str__(self) -> str:
        frame_strs = [str(x) for x in self.location]
        return ".".join(frame_strs)


OutputLocation = tuple[NodeLocation, PortID]
