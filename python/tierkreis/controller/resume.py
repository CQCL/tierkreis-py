import json
from logging import getLogger

from tierkreis.controller.executor.protocol import ControllerExecutor
from tierkreis.controller.models import NodeLocation
from tierkreis.controller.start import start
from tierkreis.controller.storage.protocol import ControllerStorage
from tierkreis.core import Labels
from tierkreis.core.function import FunctionName
from tierkreis.core.tierkreis_graph import FunctionNode, PortID, TierkreisGraph

from tierkreis.core.protos.tierkreis.v1alpha1.graph import Graph, Value

logger = getLogger(__name__)


def resume(
    storage: ControllerStorage,
    executor: ControllerExecutor,
    node_location: NodeLocation,
) -> None:
    """Should only be called when a node has started and has not finished."""

    logger.debug(f"\n\nRESUME {node_location}")
    name = storage.read_node_definition(node_location).function_name

    if name == "eval":
        resume_eval(storage, executor, node_location)

    elif name == "loop":
        resume_loop(storage, executor, node_location)

    elif name == "map":
        raise NotImplementedError("MAP not implemented.")

    else:
        logger.debug(f"{name} already started")


def resume_eval(
    storage: ControllerStorage,
    executor: ControllerExecutor,
    node_location: NodeLocation,
):
    message = storage.read_output(node_location.append_node(0), Labels.THUNK)
    graph = TierkreisGraph.from_proto(Graph.FromString(message))

    logger.debug(graph.n_nodes)
    for i in range(graph.n_nodes):
        new_location = node_location.append_node(i)
        logger.debug(f"new_location: {new_location}")

        if storage.is_node_finished(new_location):
            logger.debug(f"{new_location} is finished")
            continue

        if storage.is_node_started(new_location):
            logger.debug(f"{new_location} is started")
            resume(storage, executor, new_location)
            continue

        inputs = {
            x.target.port: (
                node_location.append_node(x.source.node_ref.idx),
                x.source.port,
            )
            for x in graph.in_edges(i)
        }
        if all(storage.is_node_finished(loc) for (loc, _) in inputs.values()):
            logger.debug(f"{new_location} is_ready_to_start")
            output_list = [x.source.port for x in graph.out_edges(i)]
            start(storage, executor, new_location, graph[i], inputs, output_list)
            return

        logger.debug(f"node not ready to start {new_location}")


def resume_loop(
    storage: ControllerStorage,
    executor: ControllerExecutor,
    node_location: NodeLocation,
):
    i = 0
    while storage.is_node_started(node_location.append_loop(i + 1)):
        i += 1
    new_location = node_location.append_loop(i)
    logger.debug(f"found latest iteration of loop: {new_location}")

    if not storage.is_node_finished(new_location):
        resume(storage, executor, new_location)
        return

    # Latest iteration is finished. Do we BREAK or CONTINUE?
    pointer_struct = json.loads(storage.read_output(new_location, Labels.VALUE))
    tag: str = pointer_struct["tag"]
    old_loc = NodeLocation.from_str(pointer_struct["node_location"])
    old_port: PortID = pointer_struct["port"]
    logger.debug(f"tagged node location {tag}, {old_loc}, {old_port}")
    if tag == Labels.BREAK:
        storage.link_outputs(node_location, Labels.VALUE, old_loc, old_port)
        storage.mark_node_finished(node_location)

    else:
        start(
            storage,
            executor,
            node_location.append_loop(i + 1),
            FunctionNode(FunctionName("eval")),
            {
                Labels.VALUE: (old_loc, old_port),
                Labels.THUNK: (new_location.append_node(0), Labels.THUNK),
            },
            [Labels.VALUE],
        )
