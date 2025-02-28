from enum import Enum
from logging import getLogger
from pathlib import Path

from pydantic import BaseModel

from tierkreis.core.tierkreis_graph import PortID
from tierkreis.exceptions import TierkreisError

logger = getLogger(__name__)


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

    @staticmethod
    def from_str(type: str) -> "NodeType":
        match type:
            case "N":
                return NodeType.NODE
            case "L":
                return NodeType.LOOP
            case "M":
                return NodeType.MAP
            case _:
                raise TierkreisError("Unknown node type.")


class NodeStep(BaseModel):
    node_type: NodeType
    idx: int

    def __str__(self) -> str:
        return f"{self.node_type}{self.idx}"

    @staticmethod
    def from_str(frame: str) -> "NodeStep":
        try:
            c, idx = frame[0], int(frame[1:])
            return NodeStep(node_type=NodeType.from_str(c), idx=idx)
        except (IndexError, ValueError) as exc:
            logger.exception(exc)
            raise TierkreisError(f"Invalid NodeStep {frame}")


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

    @staticmethod
    def from_str(location: str) -> "NodeLocation":
        if not location:
            return NodeLocation(location=[])
        frames = location.split(".")
        return NodeLocation(location=[NodeStep.from_str(x) for x in frames])


OutputLocation = tuple[NodeLocation, PortID]
