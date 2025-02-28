import json
from pathlib import Path
import subprocess
from logging import getLogger

from betterproto import which_one_of

from tierkreis.controller.executor.protocol import ControllerExecutor
from tierkreis.controller.models import NodeLocation, OutputLocation
from tierkreis.controller.storage.protocol import ControllerStorage
from tierkreis.core import Labels
from tierkreis.core.function import FunctionName
from tierkreis.core.tierkreis_graph import (
    BoxNode,
    ConstNode,
    FunctionNode,
    InputNode,
    MatchNode,
    OutputNode,
    PortID,
    TagNode,
    TierkreisNode,
)
from tierkreis.core.values import TierkreisValue
from tierkreis.exceptions import TierkreisError

logger = getLogger(__name__)


def start(
    storage: ControllerStorage,
    executor: ControllerExecutor,
    node_location: NodeLocation,
    tk_node: TierkreisNode,
    inputs: dict[PortID, OutputLocation],
    output_list: list[PortID],
) -> None:
    logger.debug(f"start {node_location} {tk_node} {inputs} {output_list}")
    if isinstance(tk_node, FunctionNode):
        name = tk_node.function_name.name
        start_function_node(storage, executor, node_location, name, inputs, output_list)

    elif isinstance(tk_node, InputNode):
        storage.mark_node_finished(node_location)

    elif isinstance(tk_node, OutputNode):
        storage.mark_node_finished(node_location)

        parent_loc = NodeLocation(location=node_location.location[:-1])
        pipe_inputs_to_output_location(storage, parent_loc, inputs)
        storage.mark_node_finished(parent_loc)

    elif isinstance(tk_node, ConstNode):
        bs = bytes_from_value(tk_node.value)
        storage.write_output(node_location, Labels.VALUE, bs)
        storage.mark_node_finished(node_location)

    elif isinstance(tk_node, TagNode):
        loc, port = inputs[Labels.VALUE]
        tag = {"tag": tk_node.tag_name, "node_location": str(loc), "port": port}
        storage.write_output(node_location, Labels.VALUE, json.dumps(tag).encode())
        storage.mark_node_finished(node_location)

    elif isinstance(tk_node, BoxNode):
        raise NotImplementedError("box node not implemented")

    elif isinstance(tk_node, MatchNode):
        NotImplementedError("Controller does not support MatchNodes.")

    else:
        raise TierkreisError(f"Unknown node type {tk_node}.")


def start_function_node(
    storage: ControllerStorage,
    executor: ControllerExecutor,
    node_location: NodeLocation,
    name: str,
    inputs: dict[PortID, OutputLocation],
    output_list: list[PortID],
):
    launcher_name = ".".join(name.split(".")[:-1])
    name = name.split(".")[-1]
    print("-------")
    print(launcher_name)
    print(name)
    def_path = storage.write_node_definition(node_location, name, inputs, output_list)

    if name == "eval":
        pipe_inputs_to_output_location(storage, node_location.append_node(0), inputs)

    elif name == "loop":
        eval_inputs = {k: v for k, v in inputs.items()}
        eval_inputs["thunk"] = inputs["body"]
        start(
            storage,
            executor,
            node_location.append_loop(0),
            FunctionNode(FunctionName("eval")),
            eval_inputs,
            output_list,
        )

    elif name == "map":
        raise NotImplementedError("MAP not implemented.")

    elif name == "copy":
        input_loc, input_port = list(inputs.values())[0]
        for output in output_list:
            storage.link_outputs(node_location, output, input_loc, input_port)
        storage.mark_node_finished(node_location)

    elif name == "switch":
        pred = json.loads(storage.read_output(*inputs["pred"]))
        if pred:
            storage.link_outputs(node_location, Labels.VALUE, *inputs["if_true"])
        else:
            storage.link_outputs(node_location, Labels.VALUE, *inputs["if_false"])
        storage.mark_node_finished(node_location)

    elif name == "discard":
        storage.mark_node_finished(node_location)
    else:
        logger.debug(f"Executing {(str(node_location), name, inputs, output_list)}")
        executor.run(launcher_name, def_path)


def pipe_inputs_to_output_location(
    storage: ControllerStorage,
    output_loc: NodeLocation,
    inputs: dict[PortID, OutputLocation],
) -> None:
    for new_port, (old_loc, old_port) in inputs.items():
        storage.link_outputs(output_loc, new_port, old_loc, old_port)


def bytes_from_value(proto: TierkreisValue) -> bytes:
    if which_one_of(proto.to_proto(), "value")[0] == "graph":
        return proto.to_proto().graph.SerializeToString()

    py_value = proto.try_autopython()
    return json.dumps(py_value).encode()
